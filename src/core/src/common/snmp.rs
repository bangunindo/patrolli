// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::mib;
use crate::{payload, payload::ZString};
use anyhow::{Result, anyhow};
use async_snmp::{
    AuthoritativeEngine, DesSaltState, Oid, PersistedAuthoritativeEngine, PersistedDesSaltState,
    Value,
};
use mib_rs::Oid as MibOid;
use smol_str::ToSmolStr;
use std::borrow::Cow;
use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

/// Decodes the typed values Net-SNMP wraps in an Opaque (draft-perkins-float, draft-perkins-opaque):
/// `9f <type> <len> <value>`, where the short-form `len` must cover the rest exactly. Anything else
/// stays raw bytes, which is all RFC 2578 §7.1.9 asks of a receiver.
fn try_decode_opaque(bytes: &[u8]) -> Option<payload::Value<'static>> {
    let [0x9f, tag, len, value @ ..] = bytes else {
        return None;
    };
    if usize::from(*len) != value.len() {
        return None;
    }
    match (*tag, value) {
        // Float, IEEE 754 single precision
        (0x78, &[a, b, c, d]) => Some(payload::Value::Double(
            f32::from_be_bytes([a, b, c, d]) as f64
        )),
        // Double, IEEE 754 double precision
        (0x79, &[a, b, c, d, e, f, g, h]) => Some(payload::Value::Double(f64::from_be_bytes([
            a, b, c, d, e, f, g, h,
        ]))),
        // Counter64 and UInt64, a BER INTEGER: a value with its top bit set takes a leading 0x00
        (0x76 | 0x7b, value) => {
            let value = match value {
                [0, rest @ ..] if rest.len() == 8 => rest,
                _ => value,
            };
            if value.is_empty() || value.len() > 8 {
                return None;
            }
            Some(u64_value(
                value.iter().fold(0, |acc, &b| acc << 8 | u64::from(b)),
            ))
        }
        // Int64, a two's complement BER INTEGER
        (0x7a, value @ [first, ..]) if value.len() <= 8 => {
            let sign = if first & 0x80 != 0 { -1 } else { 0 };
            Some(payload::Value::Int64(
                value.iter().fold(sign, |acc, &b| acc << 8 | i64::from(b)),
            ))
        }
        _ => None,
    }
}

/// An i64 column can't hold the top half of u64, so those values go out as their decimal string.
fn u64_value(value: u64) -> payload::Value<'static> {
    match i64::try_from(value) {
        Ok(value) => payload::Value::Int64(value),
        Err(_) => payload::Value::String(ZString::from_owned(value.to_string())),
    }
}

pub fn process_value<'a>(id: &Oid, value: &'a Value, use_mib: bool) -> payload::Value<'a> {
    let mut result: Vec<(ZString, payload::Value)>;
    if use_mib {
        result = Vec::with_capacity(4);
    } else {
        result = Vec::with_capacity(2);
    }
    result.push((
        ZString::from_ref("oid"),
        payload::Value::String(ZString::from_smol(id.to_smolstr())),
    ));
    if use_mib {
        let oid_name = mib::get_name_by_oid(&MibOid::from(id.arcs())).unwrap_or_default();
        result.push((
            ZString::from_ref("oid_name"),
            payload::Value::String(ZString::from_owned(oid_name)),
        ));
        let value_formatted = mib::format_value(&MibOid::from(id.arcs()), value)
            .unwrap_or_else(|| payload::Value::String(ZString::from_ref("")));
        result.push((ZString::from_ref("value_formatted"), value_formatted));
    }
    let snmp_value = match value {
        Value::Integer(value) => payload::Value::Int64(*value as i64),
        Value::OctetString(value) => match std::str::from_utf8(value) {
            Ok(value) => payload::Value::String(ZString::from_ref(value)),
            _ => payload::Value::Bytes(Cow::Borrowed(value)),
        },
        Value::Null => payload::Value::Null,
        Value::ObjectIdentifier(id) => payload::Value::String(ZString::from_smol(id.to_smolstr())),
        Value::IpAddress(value) => {
            payload::Value::String(ZString::from_smol(IpAddr::from(*value).to_smolstr()))
        }
        Value::Counter32(value) => payload::Value::Int64(*value as i64),
        Value::Gauge32(value) => payload::Value::Int64(*value as i64),
        Value::TimeTicks(value) => payload::Value::Double(*value as f64 / 100f64),
        Value::Opaque(value) => {
            try_decode_opaque(value).unwrap_or_else(|| payload::Value::Bytes(Cow::Borrowed(value)))
        }
        Value::Counter64(value) => u64_value(*value),
        _ => payload::Value::Null,
    };
    result.push((ZString::from_ref("value"), snmp_value));
    payload::Value::KvList(result)
}

pub fn parse_oid(oid: &str) -> Result<Oid> {
    if let Ok(oid_parsed) = oid.parse::<Oid>() {
        Ok(oid_parsed)
    } else if let Some(oid_parsed) = mib::get_oid_by_name(oid) {
        Ok(Oid::from_slice(oid_parsed.as_ref()))
    } else {
        Err(anyhow!("Invalid OID: {}", oid))
    }
}

/// SNMPv3 makes the receiver of a trap with USM users, and the generator of
/// DES/3DES-encrypted messages, an authoritative engine: an engine ID and an
/// `snmpEngineBoots` that must rise on every restart (RFC 3414 §2.2), which
/// async-snmp requires before it builds either. Instead of a counter persisted
/// to disk, boots is the seconds since [`BOOTS_EPOCH_SECS`] when this process
/// first needs it, which rises on every restart with nothing to store, as long
/// as the clock is not stepped back and restarts are a second apart. Every
/// trap receiver and SNMP client of the process shares this one engine, so its
/// engine time runs from the process's first use and DES salts never repeat
/// within one boots value.
const BOOTS_EPOCH_SECS: u64 = 1_767_225_600; // 2026-01-01T00:00:00Z
/// RFC 3414's `snmpEngineBoots` maximum.
const MAX_ENGINE_BOOTS: u32 = 2_147_483_647;

/// This process's `snmpEngineBoots`: at least 2, so the previous value handed
/// to async-snmp's `restart` is a valid one, and below RFC 3414's maximum,
/// past which authenticated messages are refused.
static ENGINE_BOOTS: LazyLock<u32> = LazyLock::new(|| {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH + Duration::from_secs(BOOTS_EPOCH_SECS))
        .map_or(0, |d| d.as_secs());
    secs.saturating_add(2).min(u64::from(MAX_ENGINE_BOOTS - 1)) as u32
});

/// RFC 3411 `snmpEngineID`: enterprise 0 with the format bit set, format 5
/// (administratively assigned octets), then the cluster node's id, or a random
/// one per process without a cluster.
fn engine_id() -> Vec<u8> {
    let node_id = crate::config::NODE_ID
        .get()
        .copied()
        .unwrap_or_else(uuid::Uuid::new_v4);
    let mut id = vec![0x80, 0x00, 0x00, 0x00, 0x05];
    id.extend_from_slice(node_id.as_bytes());
    id
}

static AUTHORITATIVE_ENGINE: LazyLock<AuthoritativeEngine> = LazyLock::new(|| {
    let previous = PersistedAuthoritativeEngine::new(engine_id(), *ENGINE_BOOTS - 1)
        .expect("engine id and boots are valid by construction");
    // Nothing to persist: the next process derives a higher boots itself.
    AuthoritativeEngine::restart(previous, |_| Ok::<(), Infallible>(()))
        .expect("restarting with a no-op persist cannot fail")
});

static DES_SALT_STATE: LazyLock<DesSaltState> = LazyLock::new(|| {
    let previous =
        PersistedDesSaltState::new(*ENGINE_BOOTS - 1).expect("boots is valid by construction");
    DesSaltState::restart(previous, |_| Ok::<(), Infallible>(()))
        .expect("restarting with a no-op persist cannot fail")
});

/// The process's authoritative SNMP engine; see [`BOOTS_EPOCH_SECS`].
pub fn authoritative_engine() -> AuthoritativeEngine {
    AUTHORITATIVE_ENGINE.clone()
}

/// The process's DES/3DES salt state, on the same boots as
/// [`authoritative_engine`], which async-snmp checks; see [`BOOTS_EPOCH_SECS`].
pub fn des_salt_state() -> DesSaltState {
    DES_SALT_STATE.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(bytes: &[u8]) -> Option<i64> {
        match try_decode_opaque(bytes)? {
            payload::Value::Int64(v) => Some(v),
            _ => panic!("not an Int64"),
        }
    }

    fn double(bytes: &[u8]) -> Option<f64> {
        match try_decode_opaque(bytes)? {
            payload::Value::Double(v) => Some(v),
            _ => panic!("not a Double"),
        }
    }

    #[test]
    fn opaque_float_of_six_bytes_does_not_panic() {
        assert!(try_decode_opaque(&[0x9f, 0x78, 0x04, 0, 0, 0]).is_none());
        assert!(try_decode_opaque(&[0x9f, 0x79, 0x08, 0, 0, 0, 0, 0, 0, 0]).is_none());
        assert!(try_decode_opaque(&[0x9f, 0x78]).is_none());
        assert!(try_decode_opaque(&[]).is_none());
    }

    #[test]
    fn opaque_values_decode_as_in_the_drafts() {
        // the worked examples of draft-perkins-float-00 and draft-perkins-opaque-01
        assert_eq!(double(&[0x9f, 0x78, 0x04, 0x42, 0xf6, 0, 0]), Some(123.0));
        assert_eq!(
            double(&[0x9f, 0x79, 0x08, 0x40, 0x5e, 0xc0, 0, 0, 0, 0, 0]),
            Some(123.0)
        );
        assert_eq!(int(&[0x9f, 0x76, 0x03, 0x00, 0xdd, 0xce]), Some(56782));
        assert_eq!(int(&[0x9f, 0x7b, 0x03, 0x00, 0xdd, 0xce]), Some(56782));
    }

    #[test]
    fn opaque_int64_is_sign_extended() {
        assert_eq!(int(&[0x9f, 0x7a, 0x01, 0xff]), Some(-1));
        assert_eq!(int(&[0x9f, 0x7a, 0x02, 0xff, 0x7f]), Some(-129));
        assert_eq!(int(&[0x9f, 0x7a, 0x01, 0x7f]), Some(127));
        assert_eq!(
            int(&[0x9f, 0x7a, 0x08, 0x80, 0, 0, 0, 0, 0, 0, 0]),
            Some(i64::MIN)
        );
        assert!(try_decode_opaque(&[0x9f, 0x7a, 0x09, 0, 0, 0, 0, 0, 0, 0, 0, 1]).is_none());
    }

    #[test]
    fn opaque_unsigned_takes_a_leading_zero_past_i64_max() {
        let max = [
            0x9f, 0x76, 0x09, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ];
        match try_decode_opaque(&max) {
            Some(payload::Value::String(s)) => assert_eq!(s.as_str(), "18446744073709551615"),
            _ => panic!("u64::MAX should decode to its decimal string"),
        }
        // nine octets are only allowed for that leading zero
        let mut nine = max;
        nine[3] = 0x01;
        assert!(try_decode_opaque(&nine).is_none());
        // Net-SNMP reads eight octets with the top bit set as unsigned too
        assert!(matches!(
            try_decode_opaque(&[0x9f, 0x7b, 0x08, 0x80, 0, 0, 0, 0, 0, 0, 0]),
            Some(payload::Value::String(s)) if s.as_str() == "9223372036854775808"
        ));
        assert_eq!(int(&[0x9f, 0x76, 0x01, 0xff]), Some(255));
    }

    #[test]
    fn malformed_opaque_stays_raw() {
        // trailing octets past the declared length
        assert!(try_decode_opaque(&[0x9f, 0x78, 0x04, 0x42, 0xf6, 0, 0, 0]).is_none());
        // a float with a double's length
        assert!(try_decode_opaque(&[0x9f, 0x78, 0x08, 0, 0, 0, 0, 0, 0, 0, 0]).is_none());
        // empty integers
        assert!(try_decode_opaque(&[0x9f, 0x76, 0x00]).is_none());
        assert!(try_decode_opaque(&[0x9f, 0x7a, 0x00]).is_none());
        // a type the drafts don't define, and plain BER without the extension tag
        assert!(try_decode_opaque(&[0x9f, 0x30, 0x01, 0x00]).is_none());
        assert!(try_decode_opaque(&[0x02, 0x01, 0x05]).is_none());

        let oid: Oid = "1.3.6.1.4.1.1".parse().unwrap();
        let raw = Value::Opaque(bytes::Bytes::from_static(&[0x9f, 0x78, 0x04, 0, 0, 0]));
        let payload::Value::KvList(kv) = process_value(&oid, &raw, false) else {
            panic!("process_value returns a KvList");
        };
        assert!(
            matches!(&kv[1], (k, payload::Value::Bytes(b)) if k.as_str() == "value" && b.len() == 6)
        );
    }
}
