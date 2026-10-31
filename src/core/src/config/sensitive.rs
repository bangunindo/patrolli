// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use anyhow::{anyhow, bail, ensure};
use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, MAX_TAG_LEN, NONCE_LEN, Nonce, UnboundKey};
use aws_lc_rs::pbkdf2::{self, PBKDF2_HMAC_SHA256};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::de::{self, DeserializeOwned, IgnoredAny, MapAccess, Visitor};
use serde::ser::{self, SerializeMap};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt;
use std::mem::discriminant;
use std::num::NonZeroU32;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, OneOfBuilder, Schema, Type};

const KEYS_ENV: &str = "PATROLLI_SECRET_KEYS";
const ACTIVE_KEY_ENV: &str = "PATROLLI_SECRET_ACTIVE_KEY";
const PREFIX: &str = "enc:v1:";
const KDF_ITERATIONS: NonZeroU32 = NonZeroU32::new(600_000).unwrap();

/// A secret config value. `Plain` is the clear text, as YAML, API requests and
/// the running node hold it. `Cipher` is the `enc:v1:` text the database holds.
/// `Set` is `{"set": "<handle>"}`: what responses show, and what an update
/// sends back to keep the stored value with that handle.
#[derive(Clone, PartialEq, Eq)]
pub enum Sensitive {
    Plain(String),
    Cipher(String),
    Set(String),
}

impl fmt::Debug for Sensitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl Sensitive {
    pub fn expose(&self) -> Result<&str, SecretError> {
        match self {
            Self::Plain(value) => Ok(value),
            Self::Cipher(_) => Err(SecretError::Encrypted),
            Self::Set(_) => Err(SecretError::Masked),
        }
    }

    pub fn encrypt(&mut self, keys: &SecretKeys) -> Result<(), SecretError> {
        match self {
            Self::Plain(value) => *self = Self::Cipher(keys.seal(value)?),
            Self::Cipher(_) => {}
            Self::Set(_) => return Err(SecretError::Masked),
        }
        Ok(())
    }

    pub fn decrypt(&mut self, keys: &SecretKeys) -> Result<(), SecretError> {
        match self {
            Self::Cipher(value) => *self = Self::Plain(keys.open(value)?),
            Self::Plain(_) | Self::Set(_) => return Err(SecretError::UnknownFormat),
        }
        Ok(())
    }

    pub fn mask(&mut self) {
        let handle = match self {
            Self::Cipher(value) => handle(value).unwrap_or_default().to_owned(),
            Self::Plain(_) | Self::Set(_) => String::new(),
        };
        *self = Self::Set(handle);
    }

    /// Moves a stored value onto the active key; `Ok(true)` when it changed. On
    /// an error the value is left as it was.
    pub fn rekey(&mut self, keys: &SecretKeys) -> Result<bool, SecretError> {
        match self {
            Self::Cipher(value) if value.starts_with(&header(&keys.active)) => Ok(false),
            Self::Cipher(value) => {
                *value = keys.seal(&keys.open(value)?)?;
                Ok(true)
            }
            Self::Plain(_) | Self::Set(_) => Err(SecretError::UnknownFormat),
        }
    }
}

impl Serialize for Sensitive {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Plain(_) => Err(ser::Error::custom(
                "plaintext secret reached serialization; encrypt or mask it first",
            )),
            Self::Cipher(value) => serializer.serialize_str(value),
            Self::Set(handle) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("set", handle)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Sensitive {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(SensitiveVisitor)
    }
}

const NOT_A_STRING: &str = "secret value must be a string; quote it";
const NOT_A_SECRET: &str = "secret value must be a string or {\"set\": \"<handle>\"}";

// No error from here may contain the input. serde's default messages for
// numbers and booleans quote it, so those are overridden, and the errors from
// reading a map are dropped.
struct SensitiveVisitor;

impl<'de> Visitor<'de> for SensitiveVisitor {
    type Value = Sensitive;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string or {\"set\": \"<handle>\"}")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Sensitive, E> {
        self.visit_string(v.to_owned())
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Sensitive, E> {
        if v.starts_with(PREFIX) {
            Ok(Sensitive::Cipher(v))
        } else {
            Ok(Sensitive::Plain(v))
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Sensitive, A::Error> {
        let key = map.next_key::<String>().ok().flatten();
        if key.as_deref() != Some("set") {
            return Err(de::Error::custom(NOT_A_SECRET));
        }
        let handle = map.next_value::<String>().unwrap_or_default();
        let more_keys = !matches!(map.next_key::<IgnoredAny>(), Ok(None));
        if handle.is_empty() || more_keys {
            return Err(de::Error::custom(NOT_A_SECRET));
        }
        Ok(Sensitive::Set(handle))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Sensitive, E> {
        Err(E::custom(NOT_A_STRING))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Sensitive, E> {
        Err(E::custom(NOT_A_STRING))
    }

    fn visit_i128<E: de::Error>(self, _: i128) -> Result<Sensitive, E> {
        Err(E::custom(NOT_A_STRING))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Sensitive, E> {
        Err(E::custom(NOT_A_STRING))
    }

    fn visit_u128<E: de::Error>(self, _: u128) -> Result<Sensitive, E> {
        Err(E::custom(NOT_A_STRING))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Sensitive, E> {
        Err(E::custom(NOT_A_STRING))
    }
}

impl utoipa::PartialSchema for Sensitive {
    fn schema() -> RefOr<Schema> {
        OneOfBuilder::new()
            .item(ObjectBuilder::new().schema_type(Type::String))
            .item(
                ObjectBuilder::new()
                    .property("set", ObjectBuilder::new().schema_type(Type::String))
                    .required("set"),
            )
            .description(Some(
                "Write-only secret. Send the value as a string. Responses return {\"set\": \"<handle>\"}; send that object back unchanged in an update to keep the stored value.",
            ))
            .into()
    }
}

impl utoipa::ToSchema for Sensitive {}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret value is still encrypted")]
    Encrypted,
    #[error("secret value is a {{\"set\": ...}} handle, not a value")]
    Masked,
    #[error("secret value is not in a known encrypted format")]
    UnknownFormat,
    #[error("unknown key id; add the key to {}", KEYS_ENV)]
    UnknownKey { key_id: String },
    #[error("wrong key for this key id, or the value was modified")]
    WrongKey { key_id: String },
    #[error("failed to encrypt secret value")]
    EncryptFailed,
    #[error("encrypted values cannot be sent; send the plain value")]
    CipherSent,
    #[error("no stored value has this {{\"set\": ...}} handle; send the value")]
    NothingToKeep,
    #[error("the stored config cannot be read; send every secret value again")]
    StoredUnreadable,
    #[error("the type changed, so no stored secret can be kept; send every secret value again")]
    TypeChanged,
}

impl SecretError {
    pub fn key_id(&self) -> Option<&str> {
        match self {
            Self::UnknownKey { key_id } | Self::WrongKey { key_id } => Some(key_id),
            Self::Encrypted
            | Self::Masked
            | Self::UnknownFormat
            | Self::EncryptFailed
            | Self::CipherSent
            | Self::NothingToKeep
            | Self::StoredUnreadable
            | Self::TypeChanged => None,
        }
    }
}

// no Debug, so nothing can print a key
pub struct SecretKeys {
    keys: HashMap<String, LessSafeKey>,
    active: String,
}

impl SecretKeys {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::parse(
            std::env::var(KEYS_ENV).ok().as_deref(),
            std::env::var(ACTIVE_KEY_ENV).ok().as_deref(),
        )
    }

    /// No error quotes an entry: it can hold the secret.
    pub fn parse(keys: Option<&str>, active: Option<&str>) -> anyhow::Result<Self> {
        let keys = keys.ok_or_else(|| {
            anyhow!(
                "{KEYS_ENV} is not set; it must list <id>:<secret> entries, separated by commas and identical on every cluster node"
            )
        })?;
        ensure!(
            !keys.is_empty(),
            "{KEYS_ENV} is empty; it must list <id>:<secret> entries"
        );
        let active = active.ok_or_else(|| {
            anyhow!("{ACTIVE_KEY_ENV} is not set; it must name the key id that encrypts new values")
        })?;
        ensure!(
            valid_key_id(active),
            "{ACTIVE_KEY_ENV} is not a valid key id; use 1 to 20 of A-Z a-z 0-9 _ -"
        );

        let mut parsed = HashMap::new();
        for (index, entry) in keys.split(',').enumerate() {
            let Some((id, secret)) = entry.split_once(':') else {
                bail!("{KEYS_ENV} entry {index} is not <id>:<secret>");
            };
            ensure!(
                valid_key_id(id),
                "{KEYS_ENV} entry {index} has an invalid key id; use 1 to 20 of A-Z a-z 0-9 _ -"
            );
            ensure!(
                secret.len() >= 16,
                "{KEYS_ENV} key {id} is {} bytes; it must be at least 16",
                secret.len()
            );
            // The salt is fixed so every node derives the same key. The id in
            // it makes one secret listed under two ids give two different keys.
            let mut key = [0u8; 32];
            pbkdf2::derive(
                PBKDF2_HMAC_SHA256,
                KDF_ITERATIONS,
                format!("patrolli-secret-key:{id}").as_bytes(),
                secret.as_bytes(),
                &mut key,
            );
            let key = UnboundKey::new(&AES_256_GCM, &key)
                .map_err(|_| anyhow!("{KEYS_ENV} key {id} is not a usable AES-256 key"))?;
            if parsed
                .insert(id.to_owned(), LessSafeKey::new(key))
                .is_some()
            {
                bail!("{KEYS_ENV} lists key id {id} more than once");
            }
        }
        ensure!(
            parsed.contains_key(active),
            "{ACTIVE_KEY_ENV} names key id {active}, which is not in {KEYS_ENV}"
        );
        Ok(Self {
            keys: parsed,
            active: active.to_owned(),
        })
    }

    pub fn active_key_id(&self) -> &str {
        &self.active
    }

    fn seal(&self, plain: &str) -> Result<String, SecretError> {
        let header = header(&self.active);
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| SecretError::EncryptFailed)?;
        let mut sealed = plain.as_bytes().to_vec();
        self.keys[&self.active]
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(header.as_bytes()),
                &mut sealed,
            )
            .map_err(|_| SecretError::EncryptFailed)?;
        let mut blob = nonce.to_vec();
        blob.append(&mut sealed);
        Ok(format!("{header}{}", STANDARD.encode(blob)))
    }

    fn open(&self, cipher: &str) -> Result<String, SecretError> {
        let (key_id, body) = cipher
            .strip_prefix(PREFIX)
            .and_then(|rest| rest.split_once(':'))
            .filter(|(key_id, _)| valid_key_id(key_id))
            .ok_or(SecretError::UnknownFormat)?;
        let mut blob = STANDARD
            .decode(body)
            .map_err(|_| SecretError::UnknownFormat)?;
        if blob.len() < NONCE_LEN + MAX_TAG_LEN {
            return Err(SecretError::UnknownFormat);
        }
        let key = self
            .keys
            .get(key_id)
            .ok_or_else(|| SecretError::UnknownKey {
                key_id: key_id.to_owned(),
            })?;
        let (nonce, sealed) = blob.split_at_mut(NONCE_LEN);
        let nonce =
            Nonce::try_assume_unique_for_key(nonce).map_err(|_| SecretError::UnknownFormat)?;
        let plain = key
            .open_in_place(nonce, Aad::from(header(key_id).as_bytes()), sealed)
            .map_err(|_| SecretError::WrongKey {
                key_id: key_id.to_owned(),
            })?;
        String::from_utf8(plain.to_vec()).map_err(|_| SecretError::UnknownFormat)
    }
}

fn valid_key_id(id: &str) -> bool {
    (1..=20).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn header(key_id: &str) -> String {
    format!("{PREFIX}{key_id}:")
}

/// The base64 of the nonce, right after the header: public, and unique per
/// encryption.
fn handle(cipher: &str) -> Option<&str> {
    let (_, body) = cipher.strip_prefix(PREFIX)?.split_once(':')?;
    body.get(..NONCE_LEN / 3 * 4)
}

/// Every secret leaf of a config with its field kind: a fixed name for the
/// field, the same for every element of a list.
pub type SensitiveRefs<'a> = Vec<(&'static str, &'a mut Sensitive)>;

pub trait SensitiveFields: Clone + Serialize + DeserializeOwned {
    fn sensitive_mut(&mut self) -> SensitiveRefs<'_>;

    fn check_new(&mut self) -> Result<(), SecretError> {
        for (_, value) in self.sensitive_mut() {
            match value {
                Sensitive::Plain(_) => {}
                Sensitive::Cipher(_) => return Err(SecretError::CipherSent),
                Sensitive::Set(_) => return Err(SecretError::NothingToKeep),
            }
        }
        Ok(())
    }

    /// An update request: each `{"set": "<handle>"}` takes the stored encrypted
    /// value with that handle. `stored_type` and `stored_config` are the row's
    /// `type` and `config` columns.
    fn keep_stored(
        &mut self,
        stored_type: &str,
        stored_config: serde_json::Value,
    ) -> Result<(), SecretError> {
        let mut keeps_any = false;
        for (_, value) in self.sensitive_mut() {
            match value {
                Sensitive::Plain(_) => {}
                Sensitive::Cipher(_) => return Err(SecretError::CipherSent),
                Sensitive::Set(_) => keeps_any = true,
            }
        }
        // a row that no longer parses can still be replaced by an update that
        // sends every secret
        if !keeps_any {
            return Ok(());
        }
        // the serde error is dropped: it can quote stored values
        let mut stored: Self = serde_json::from_value(serde_json::json!({
            "type": stored_type,
            "config": stored_config,
        }))
        .map_err(|_| SecretError::StoredUnreadable)?;
        if discriminant(self) != discriminant(&stored) {
            return Err(SecretError::TypeChanged);
        }

        // A handle is kept only in the same kind of field, so a secret cannot
        // move into another field. A handle used twice is rejected.
        let mut stored_ciphers: HashMap<&str, (&str, &str)> = stored
            .sensitive_mut()
            .into_iter()
            .filter_map(|(kind, value)| match value {
                Sensitive::Cipher(cipher) => Some((handle(cipher)?, (kind, cipher.as_str()))),
                Sensitive::Plain(_) | Sensitive::Set(_) => None,
            })
            .collect();
        for (kind, value) in self.sensitive_mut() {
            if let Sensitive::Set(handle) = value {
                let cipher = match stored_ciphers.remove(handle.as_str()) {
                    Some((stored_kind, cipher)) if stored_kind == kind => cipher,
                    _ => return Err(SecretError::NothingToKeep),
                };
                *value = Sensitive::Cipher(cipher.to_owned());
            }
        }
        Ok(())
    }

    fn encrypt_all(&mut self, keys: &SecretKeys) -> anyhow::Result<()> {
        for (_, value) in self.sensitive_mut() {
            value.encrypt(keys)?;
        }
        Ok(())
    }

    fn decrypt_all(&mut self, keys: &SecretKeys) -> anyhow::Result<()> {
        for (_, value) in self.sensitive_mut() {
            value.decrypt(keys)?;
        }
        Ok(())
    }

    fn mask_all(&mut self) {
        for (_, value) in self.sensitive_mut() {
            value.mask();
        }
    }

    fn rekey_all(&mut self, keys: &SecretKeys) -> (usize, Vec<SecretError>) {
        let mut changed = 0;
        let mut errors = Vec::new();
        for (_, value) in self.sensitive_mut() {
            match value.rekey(keys) {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(e) => errors.push(e),
            }
        }
        (changed, errors)
    }
}

/// A response field: every secret in it serializes as `{"set": "<handle>"}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct Masked<T>(pub T);

impl<T: SensitiveFields> Serialize for Masked<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut masked = self.0.clone();
        masked.mask_all();
        masked.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWE=";
    const KEY_B: &str = "YmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmI=";
    const SECRET_15_BYTES: &str = "fifteen-bytes!!";

    fn keys(keys: &str, active: &str) -> SecretKeys {
        SecretKeys::parse(Some(keys), Some(active)).unwrap()
    }

    fn sealed(keys: &SecretKeys, plain: &str) -> String {
        let mut value = Sensitive::Plain(plain.to_owned());
        value.encrypt(keys).unwrap();
        match value {
            Sensitive::Cipher(cipher) => cipher,
            _ => panic!("encrypt did not produce a cipher"),
        }
    }

    #[test]
    fn parse_rejects_bad_env_without_quoting_key_material() {
        let a = format!("a:{KEY_A}");
        let cases = [
            (None, Some("a")),
            (Some(String::new()), Some("a")),
            (Some(a.clone()), None),
            (Some(a.clone()), Some("not valid!")),
            (Some(a.clone()), Some("b")),
            (Some(KEY_A.to_owned()), Some("a")),
            (Some(format!(":{KEY_A}")), Some("a")),
            (Some(format!("bad id:{KEY_A}")), Some("a")),
            (Some(format!("{}:{KEY_A}", "x".repeat(21))), Some("a")),
            (Some(format!("a:{SECRET_15_BYTES}")), Some("a")),
            (Some(format!("a:{KEY_A},a:{KEY_B}")), Some("a")),
            (Some(format!("a:{KEY_A},")), Some("a")),
        ];
        for (env_keys, active) in cases {
            let Err(e) = SecretKeys::parse(env_keys.as_deref(), active) else {
                panic!("{env_keys:?} with active {active:?} was accepted");
            };
            let message = format!("{e:#}");
            for key in [KEY_A, KEY_B, SECRET_15_BYTES] {
                assert!(
                    !message.contains(key),
                    "error quotes key material: {message}"
                );
            }
        }

        let parsed = keys(&format!("a:{KEY_A},b-2_B:{KEY_B}"), "b-2_B");
        assert_eq!(parsed.active_key_id(), "b-2_B");
    }

    #[test]
    fn sensitive_serde_round_trip() {
        let cases = [
            (
                serde_json::json!("hunter2"),
                Sensitive::Plain("hunter2".to_owned()),
            ),
            (
                serde_json::json!("enc:v1:a:AAAA"),
                Sensitive::Cipher("enc:v1:a:AAAA".to_owned()),
            ),
            (
                serde_json::json!({ "set": "AAAAAAAAAAAAAAAA" }),
                Sensitive::Set("AAAAAAAAAAAAAAAA".to_owned()),
            ),
        ];
        for (json, expected) in cases {
            let value: Sensitive = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(value, expected, "{json}");
            match value {
                Sensitive::Plain(_) => assert!(serde_json::to_value(&value).is_err()),
                _ => assert_eq!(serde_json::to_value(&value).unwrap(), json),
            }
        }
    }

    #[test]
    fn sensitive_deserialize_errors_do_not_quote_the_input() {
        let cases = [
            ("975318642", "975318642"),
            ("-975318642", "975318642"),
            ("97531.8642", "97531"),
            ("true", "true"),
            ("[\"hunter2\"]", "hunter2"),
            (r#"{"set": 975318642}"#, "975318642"),
            (r#"{"set": ["hunter2"]}"#, "hunter2"),
            (r#"{"set": true}"#, "true"),
            (r#"{"set": false}"#, "false"),
            (r#"{"set": ""}"#, "\"\""),
            (r#"{"set": "h", "hunter2": 1}"#, "hunter2"),
            (r#"{"set": "h", "extra": "hunter2"}"#, "hunter2"),
            (r#"{"password": "hunter2"}"#, "hunter2"),
            (r#"{"hunter2": true}"#, "hunter2"),
        ];
        for (json, secret) in cases {
            let Err(e) = serde_json::from_str::<Sensitive>(json) else {
                panic!("{json} was accepted");
            };
            assert!(!e.to_string().contains(secret), "{json}: {e}");
        }
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let keys = keys(&format!("a:{KEY_A}"), "a");
        let mut value = Sensitive::Plain("hunter2".to_owned());
        value.encrypt(&keys).unwrap();
        let Sensitive::Cipher(cipher) = &value else {
            panic!("encrypt did not produce a cipher");
        };
        assert!(cipher.starts_with("enc:v1:a:"));
        assert!(!cipher.contains("hunter2"));
        value.decrypt(&keys).unwrap();
        assert_eq!(value.expose().unwrap(), "hunter2");
    }

    #[test]
    fn decrypt_rejects_values_it_cannot_open() {
        let keys_a = keys(&format!("a:{KEY_A}"), "a");
        let same_bytes_as_b = keys(&format!("b:{KEY_A}"), "b");
        let cipher = sealed(&keys_a, "hunter2");
        let body = cipher.strip_prefix("enc:v1:a:").unwrap();
        let mut blob = STANDARD.decode(body).unwrap();
        *blob.last_mut().unwrap() ^= 1;
        let tampered = format!("enc:v1:a:{}", STANDARD.encode(&blob));

        let cases = [
            (
                serde_json::json!(cipher),
                &same_bytes_as_b,
                SecretError::UnknownKey { key_id: "a".into() },
            ),
            (
                serde_json::json!(format!("enc:v1:b:{body}")),
                &same_bytes_as_b,
                SecretError::WrongKey { key_id: "b".into() },
            ),
            (
                serde_json::json!(tampered),
                &keys_a,
                SecretError::WrongKey { key_id: "a".into() },
            ),
            (
                serde_json::json!("hunter2"),
                &keys_a,
                SecretError::UnknownFormat,
            ),
            (
                serde_json::json!({ "set": "AAAAAAAAAAAAAAAA" }),
                &keys_a,
                SecretError::UnknownFormat,
            ),
            (
                serde_json::json!("enc:v1:a:!!!"),
                &keys_a,
                SecretError::UnknownFormat,
            ),
            (
                serde_json::json!("enc:v1:a:AAAA"),
                &keys_a,
                SecretError::UnknownFormat,
            ),
            (
                serde_json::json!("enc:v1:hunter2"),
                &keys_a,
                SecretError::UnknownFormat,
            ),
        ];
        for (stored, keys, expected) in cases {
            let mut value: Sensitive = serde_json::from_value(stored.clone()).unwrap();
            let e = value.decrypt(keys).unwrap_err();
            assert_eq!(discriminant(&e), discriminant(&expected), "{stored}: {e}");
            assert_eq!(e.key_id(), expected.key_id(), "{stored}");
        }
    }

    #[test]
    fn keep_stored_finds_each_secret_by_handle_in_a_list_without_identity() {
        #[derive(Clone, Serialize, Deserialize)]
        struct ListFoo {
            config: Foos,
        }
        #[derive(Clone, Serialize, Deserialize)]
        struct Foos {
            foos: Vec<Sensitive>,
            #[serde(default)]
            bars: Vec<Sensitive>,
        }
        impl SensitiveFields for ListFoo {
            fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
                let foos = self.config.foos.iter_mut().map(|value| ("foos", value));
                let bars = self.config.bars.iter_mut().map(|value| ("bars", value));
                foos.chain(bars).collect()
            }
        }

        let keys = keys(&format!("a:{KEY_A}"), "a");
        let stored = ["same", "same", "other"].map(|plain| sealed(&keys, plain));
        let stored_config = serde_json::json!({ "foos": stored });
        let stored_row: ListFoo =
            serde_json::from_value(serde_json::json!({ "config": stored_config })).unwrap();
        let response = serde_json::to_value(Masked(stored_row)).unwrap();
        let masked = &response["config"]["foos"];

        let kept = |i: usize| Sensitive::Cipher(stored[i].clone());
        let new = || Sensitive::Plain("new".to_owned());
        let cases = [
            (
                serde_json::json!({ "foos": [masked[2], masked[1], "new"] }),
                vec![kept(2), kept(1), new()],
            ),
            (
                serde_json::json!({ "foos": [masked[0], masked[2]] }),
                vec![kept(0), kept(2)],
            ),
            (
                serde_json::json!({ "foos": [masked[1], "new", masked[0], masked[2]] }),
                vec![kept(1), new(), kept(0), kept(2)],
            ),
        ];
        for (config, expected) in cases {
            let mut request: ListFoo =
                serde_json::from_value(serde_json::json!({ "config": config })).unwrap();
            request.keep_stored("list", stored_config.clone()).unwrap();
            assert!(request.config.foos == expected, "{config}");
        }

        let unknown = Sensitive::Set("AAAAAAAAAAAAAAAA".to_owned());
        let empty = Sensitive::Set(String::new());
        let reused: Sensitive = serde_json::from_value(masked[0].clone()).unwrap();
        let request = |foos: Vec<Sensitive>, bars: Vec<Sensitive>| ListFoo {
            config: Foos { foos, bars },
        };
        let cases = [
            (request(vec![unknown], vec![]), SecretError::NothingToKeep),
            (request(vec![empty], vec![]), SecretError::NothingToKeep),
            (
                request(vec![reused.clone(), reused.clone()], vec![]),
                SecretError::NothingToKeep,
            ),
            (request(vec![], vec![reused]), SecretError::NothingToKeep),
        ];
        for (mut request, expected) in cases {
            let e = request
                .keep_stored("list", stored_config.clone())
                .unwrap_err();
            assert_eq!(discriminant(&e), discriminant(&expected), "{e}");
        }
    }
}
