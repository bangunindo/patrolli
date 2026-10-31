// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::{GenericClient, Pool};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::{PostgresBinder, PostgresValues};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::api::error::ApiResultExt;
use crate::api::response::ResponseError;
use crate::api::state::AppState;
use crate::config::cluster::OidcAuth;
use crate::db::schema::{AdmStatus, RefreshTokens, Users};

/// Bytes of entropy, before base64 encoding.
const REFRESH_TOKEN_LEN: usize = 32;

pub const SECRET_ENV: &str = "PATROLLI_JWT_SECRET";

const MIN_SECRET_LEN: usize = 32;

/// Rate limit on JWKS fetches, and so also the worst-case delay before a rotated
/// signing key is picked up. Without it an unknown `kid` sends a request to the IdP.
const JWKS_REFETCH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

const JWKS_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Clock drift allowed between issuer and verifier.
/// About leeway, see: https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.4
const CLOCK_SKEW_LEEWAY_SECS: u64 = 5;

const REQUIRED_LOCAL_CLAIMS: &[&str] = &["exp", "iss"];
const REQUIRED_EXTERNAL_CLAIMS: &[&str] = &["exp", "iss", "aud", "sub"];

/// Public-key algorithms only
const EXTERNAL_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database connection pool error: {0}")]
    Pool(#[from] deadpool_postgres::PoolError),
    #[error("postgres error: {0}")]
    Postgres(#[from] tokio_postgres::Error),
    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("token is not a well formed JWT")]
    Malformed,
    #[error("untrusted token issuer: {0}")]
    UntrustedIssuer(String),
    #[error("token header has no `kid`")]
    MissingKeyId,
    #[error("no signing key published for kid {0}")]
    UnknownKeyId(String),
    #[error("failed to fetch JWKS: {0}")]
    JwksFetch(#[from] reqwest::Error),
    #[error("identity provider keys are being fetched or could not be fetched")]
    KeysUnavailable,
    #[error("failed to build query: {0}")]
    Query(#[from] sea_query::error::Error),
}

/// `sub` is a string, not a `Uuid`: external issuers do not use our ids.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub iat: i64,
    pub exp: i64,
    #[serde(default)]
    pub preferred_username: String,
}

/// A local token already names a Patrolli user. An OIDC token names an identity
/// provider account, which [`require_auth`] maps to a Patrolli user.
#[derive(Debug)]
pub enum VerifiedToken {
    Local(AuthUser),
    Oidc {
        subject: String,
        /// `None` when the claim is missing, or is an email the provider says is not
        /// verified. Either way it must not be used to link an account.
        username: Option<String>,
    },
}

/// Attached to the request by [`require_auth`].
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub username: String,
}

/// Keys are cached by `kid` and refetched when an unrecognised one appears,
/// which is how signing key rotation is picked up without a restart.
pub struct Oidc {
    issuer: String,
    jwks_url: String,
    audience: String,
    username_claim_path: String,
    http: reqwest::Client,
    keys: RwLock<KeyCache>,
}

#[derive(Default)]
struct KeyCache {
    by_kid: HashMap<String, DecodingKey>,
    last_fetch: Option<Instant>,
    /// False while a fetch is running and after a failed one. A missing key then
    /// means "try again" (503), not "bad token" (401).
    last_fetch_ok: bool,
}

impl Oidc {
    fn new(config: &OidcAuth) -> anyhow::Result<Self> {
        let mut http = reqwest::Client::builder().timeout(JWKS_FETCH_TIMEOUT);
        if let Some(tls) = &config.tls {
            http = http.use_preconfigured_tls(crate::common::tls::build_rustls_client_config(tls)?);
        }

        Ok(Self {
            issuer: config.issuer.clone(),
            jwks_url: config.jwks_url.clone(),
            audience: config.audience.clone(),
            username_claim_path: config.username_claim_path.clone(),
            http: http.build()?,
            keys: RwLock::new(KeyCache::default()),
        })
    }

    async fn verify(&self, token: &str) -> Result<VerifiedToken, Error> {
        let header = jsonwebtoken::decode_header(token)?;
        if !EXTERNAL_ALGORITHMS.contains(&header.alg) {
            return Err(jsonwebtoken::errors::Error::from(
                jsonwebtoken::errors::ErrorKind::InvalidAlgorithm,
            )
            .into());
        }
        let kid = header.kid.ok_or(Error::MissingKeyId)?;

        let key = match self.cached_key(&kid).await {
            Some(key) => key,
            None => {
                self.fetch_jwks().await?;
                self.cached_key(&kid)
                    .await
                    .ok_or_else(|| Error::UnknownKeyId(kid.clone()))?
            }
        };

        // Safe to take from the header: it is on the allowed list, and jsonwebtoken
        // rejects a key that does not belong to this algorithm.
        let mut validation = Validation::new(header.alg);
        validation.leeway = CLOCK_SKEW_LEEWAY_SECS;
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        // On its own the line above accepts a token carrying no `aud` at all.
        validation.set_required_spec_claims(REQUIRED_EXTERNAL_CLAIMS);

        let claims = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)?.claims;
        let subject = claims["sub"].as_str().ok_or(Error::Malformed)?.to_string();
        // `email_verified` only describes the `email` claim. Missing counts as verified,
        // because some providers (Entra ID) never send it.
        let unverified_email = self.username_claim_path == "email"
            && (claims["email_verified"] == false || claims["email_verified"] == "false");

        Ok(VerifiedToken::Oidc {
            username: read_claim_path(&claims, &self.username_claim_path)
                .filter(|_| !unverified_email)
                .map(str::to_string),
            subject,
        })
    }

    async fn cached_key(&self, kid: &str) -> Option<DecodingKey> {
        self.keys.read().await.by_kid.get(kid).cloned()
    }

    async fn fetch_jwks(&self) -> Result<(), Error> {
        {
            // Claimed before the request so concurrent misses do not start more
            // fetches; they get 503 until it succeeds. A failed fetch spends the
            // window on purpose.
            let mut cache = self.keys.write().await;
            if let Some(last) = cache.last_fetch
                && last.elapsed() < JWKS_REFETCH_COOLDOWN
            {
                return if cache.last_fetch_ok {
                    Ok(())
                } else {
                    Err(Error::KeysUnavailable)
                };
            }
            cache.last_fetch = Some(Instant::now());
            cache.last_fetch_ok = false;
        }

        info!(url = %self.jwks_url, "fetching JWKS");
        let jwks: JwkSet = self
            .http
            .get(&self.jwks_url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let by_kid: HashMap<String, DecodingKey> = jwks
            .keys
            .iter()
            // Drops encryption keys because they are not meant to verify signatures.
            .filter(|jwk| !matches!(jwk.common.public_key_use, Some(PublicKeyUse::Encryption)))
            // Drops symmetric keys because a published secret would let anyone forge tokens.
            .filter(|jwk| !matches!(jwk.algorithm, AlgorithmParameters::OctetKey(_)))
            .filter_map(|jwk| {
                let kid = jwk.common.key_id.clone()?;
                match DecodingKey::from_jwk(jwk) {
                    Ok(key) => Some((kid, key)),
                    Err(err) => {
                        debug!(kid, error = %err, "skipping unusable JWK");
                        None
                    }
                }
            })
            .collect();

        debug!(keys = by_kid.len(), "cached signing keys");
        let mut cache = self.keys.write().await;
        cache.by_kid = by_kid;
        cache.last_fetch_ok = true;
        Ok(())
    }
}

pub struct Jwt {
    issuer: String,
    access_ttl: Duration,
    refresh_ttl: Duration,
    encoding: EncodingKey,
    decoding: DecodingKey,
    oidc: Option<Oidc>,
}

impl Jwt {
    pub fn from_env(
        issuer: String,
        access_ttl: std::time::Duration,
        refresh_ttl: std::time::Duration,
        oidc: Option<&OidcAuth>,
    ) -> anyhow::Result<Self> {
        let secret = resolve_secret(std::env::var(SECRET_ENV).ok())?;
        let mut jwt = Self::new(&secret, issuer, access_ttl, refresh_ttl);

        if let Some(oidc) = oidc {
            let oidc = Oidc::new(oidc)?;
            // Lazy: an identity provider outage must not stop this node starting, or
            // local login stops being a fallback.
            info!(issuer = %oidc.issuer, "accepting tokens from OIDC issuer");
            jwt.oidc = Some(oidc);
        }

        Ok(jwt)
    }

    /// Separate from [`Self::from_env`] so tests need not touch the environment.
    pub fn new(
        secret: &str,
        issuer: String,
        access_ttl: std::time::Duration,
        refresh_ttl: std::time::Duration,
    ) -> Self {
        Self {
            issuer,
            access_ttl: Duration::from_std(access_ttl).unwrap_or_else(|_| Duration::minutes(15)),
            refresh_ttl: Duration::from_std(refresh_ttl).unwrap_or_else(|_| Duration::days(7)),
            encoding: EncodingKey::from_secret(secret.as_bytes()),
            decoding: DecodingKey::from_secret(secret.as_bytes()),
            oidc: None,
        }
    }

    pub fn access_ttl_secs(&self) -> i64 {
        self.access_ttl.num_seconds()
    }

    pub fn refresh_expiry(&self) -> DateTime<Utc> {
        Utc::now() + self.refresh_ttl
    }

    pub fn sign_access(&self, user_id: Uuid, username: &str) -> Result<String, Error> {
        let now = Utc::now();
        let claims = Claims {
            sub: user_id.to_string(),
            iss: self.issuer.clone(),
            iat: now.timestamp(),
            exp: (now + self.access_ttl).timestamp(),
            preferred_username: username.to_string(),
        };

        Ok(jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &self.encoding,
        )?)
    }

    /// `iss` selects the verifier, and with it the key and algorithm. It is read
    /// unverified, so it may only ever choose among issuers already configured.
    pub async fn verify(&self, token: &str) -> Result<VerifiedToken, Error> {
        let issuer = peek_issuer(token).ok_or(Error::Malformed)?;

        if issuer == self.issuer {
            return self.verify_local(token);
        }

        match &self.oidc {
            Some(oidc) if oidc.issuer == issuer => oidc.verify(token).await,
            _ => Err(Error::UntrustedIssuer(issuer)),
        }
    }

    fn verify_local(&self, token: &str) -> Result<VerifiedToken, Error> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = CLOCK_SKEW_LEEWAY_SECS;
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(REQUIRED_LOCAL_CLAIMS);
        // No audience is minted into our own tokens, so nothing to check.
        validation.validate_aud = false;

        let claims = jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation)?.claims;
        let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
            jsonwebtoken::errors::Error::from(jsonwebtoken::errors::ErrorKind::InvalidSubject)
        })?;

        Ok(VerifiedToken::Local(AuthUser {
            user_id,
            username: claims.preferred_username,
        }))
    }
}

fn read_claim_path<'a>(claims: &'a serde_json::Value, path: &str) -> Option<&'a str> {
    path.split('.')
        .try_fold(claims, |value, key| value.get(key))?
        .as_str()
}

/// Unverified: the result may only be used to choose a verifier.
fn peek_issuer(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = B64.decode(payload).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    Some(claims.get("iss")?.as_str()?.to_string())
}

const SESSION_EXPIRED: &str = "Your session has expired. Please refresh or log in again.";
const INVALID_TOKEN: &str = "Invalid ID token. Ask your administrator to check that the `oidc` field in the cluster is configured correctly.";

/// Local access tokens are checked by signature alone, so whoever knows the secret
/// can sign in as anyone: there is no fallback, and a short one is refused.
fn resolve_secret(from_env: Option<String>) -> anyhow::Result<String> {
    let secret = from_env.unwrap_or_default();

    if secret.is_empty() {
        anyhow::bail!(
            "{SECRET_ENV} is not set; it must be at least {MIN_SECRET_LEN} bytes, identical on every cluster node"
        );
    }
    if secret.len() < MIN_SECRET_LEN {
        anyhow::bail!(
            "{SECRET_ENV} is {} bytes; it must be at least {MIN_SECRET_LEN}",
            secret.len()
        );
    }

    Ok(secret)
}

pub async fn issue_pair(
    client: &impl GenericClient,
    jwt: &Jwt,
    user_id: Uuid,
    username: &str,
) -> Result<(String, String), Error> {
    let access = jwt.sign_access(user_id, username)?;
    let refresh = B64.encode(rand::random::<[u8; REFRESH_TOKEN_LEN]>());

    let (sql, values) = DbQuery::insert()
        .into_table(RefreshTokens::Table)
        .columns([
            RefreshTokens::TokenHash,
            RefreshTokens::UserId,
            RefreshTokens::ExpiresAt,
        ])
        .values([
            Expr::cust_with_values("digest($1, 'sha256')", [refresh.clone()]),
            user_id.into(),
            jwt.refresh_expiry().into(),
        ])?
        .build_postgres(PostgresQueryBuilder);
    client.execute(sql.as_str(), &values.as_params()).await?;

    Ok((access, refresh))
}

/// Tokens arrive only as `Authorization: Bearer`
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    let Some(token) = bearer else {
        debug!("missing bearer token");
        return unauthorized("You must be logged in to access this resource.");
    };
    let token = token.to_string();

    let user = match state.jwt.verify(&token).await {
        Ok(VerifiedToken::Local(user)) => user,
        Ok(VerifiedToken::Oidc { subject, username }) => {
            // Missing or unverified username claim: `username_claim_path` does not fit
            // this provider, so the token can never be accepted.
            let Some(username) = username else {
                debug!("access token has no usable username claim");
                return unauthorized(INVALID_TOKEN);
            };
            let row = match find_or_link_sso_user(&state.db_pool, &subject, Some(username.as_str()))
                .await
                .ctx("look up sso user")
            {
                Ok(row) => row,
                Err(err) => return err.into_response(),
            };
            // One answer for every reason, so it does not tell which usernames exist.
            let Some(row) = row else {
                debug!("sso login matches no patrolli user");
                return unauthorized("No Patrolli account exists for this login.");
            };
            if !row.get::<_, bool>("allow_sso") {
                debug!("sso login rejected, sso is disabled for the user");
                return forbidden("SSO is disabled for this account.");
            }
            if row.get::<_, i16>("adm_status") != AdmStatus::Enabled as i16 {
                debug!("sso login rejected for inactive account");
                return forbidden("This account is disabled.");
            }
            AuthUser {
                user_id: row.get("user_id"),
                username: row.get("username"),
            }
        }
        Err(err) => {
            let chain = std::iter::successors(Some(&err as &dyn std::error::Error), |e| e.source())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(": ");

            return match &err {
                Error::Jwt(jwt_err)
                    if matches!(
                        jwt_err.kind(),
                        ErrorKind::ExpiredSignature | ErrorKind::ImmatureSignature
                    ) =>
                {
                    debug!(error = %chain, "access token expired");
                    unauthorized(SESSION_EXPIRED)
                }
                Error::JwksFetch(_) | Error::KeysUnavailable => {
                    if matches!(err, Error::JwksFetch(_)) {
                        warn!(error = %chain, "identity provider jwks fetch failed");
                    } else {
                        debug!(error = %chain, "identity provider keys not available yet");
                    }
                    ResponseError::from_status(StatusCode::SERVICE_UNAVAILABLE)
                        .with_title("Identity provider unreachable")
                        .with_detail(format!(
                            "Could not fetch the identity provider JWKS: {chain}"
                        ))
                        .into_response()
                }
                // Wrong issuer, audience, signature or shape: no retry will ever pass,
                // usually because the cluster's `oidc` config does not match the provider.
                _ => {
                    debug!(error = %chain, "access token rejected");
                    unauthorized(INVALID_TOKEN)
                }
            };
        }
    };

    // Fills the `user` field left empty by the request span, so every log line emitted
    // downstream of here is attributable.
    tracing::Span::current().record("user", tracing::field::display(&user.username));
    request.extensions_mut().insert(user);
    next.run(request).await
}

const SSO_USER_COLUMNS: [Users; 4] = [
    Users::UserId,
    Users::Username,
    Users::AllowSso,
    Users::AdmStatus,
];

fn sso_user_by_subject(subject: &str) -> (String, PostgresValues) {
    DbQuery::select()
        .columns(SSO_USER_COLUMNS)
        .from(Users::Table)
        .and_where(Expr::col(Users::OidcSubject).eq(subject))
        .build_postgres(PostgresQueryBuilder)
}

/// On the first SSO login the user is found by username and `subject` is saved on it.
/// Later logins find it by `subject` only.
async fn find_or_link_sso_user(
    pool: &Pool,
    subject: &str,
    username: Option<&str>,
) -> Result<Option<tokio_postgres::Row>, Error> {
    let client = pool.get().await?;
    let (sql, values) = sso_user_by_subject(subject);
    if let Some(row) = client.query_opt(sql.as_str(), &values.as_params()).await? {
        return Ok(Some(row));
    }
    let Some(username) = username else {
        return Ok(None);
    };

    let (sql, values) = DbQuery::update()
        .table(Users::Table)
        .value(Users::OidcSubject, subject)
        .value(Users::LastUpdated, Utc::now())
        .and_where(Expr::col(Users::Username).eq(username))
        .and_where(Expr::col(Users::AllowSso).eq(true))
        .and_where(Expr::col(Users::OidcSubject).is_null())
        .returning(DbQuery::returning().columns(SSO_USER_COLUMNS))
        .build_postgres(PostgresQueryBuilder);
    let linked = client.query_opt(sql.as_str(), &values.as_params()).await?;
    if linked.is_some() {
        return Ok(linked);
    }
    // A parallel request from the same first login may have linked it just now.
    let (sql, values) = sso_user_by_subject(subject);
    Ok(client.query_opt(sql.as_str(), &values.as_params()).await?)
}

fn forbidden(detail: &str) -> Response {
    ResponseError::from_status(StatusCode::FORBIDDEN)
        .with_title("Forbidden")
        .with_detail(detail)
        .into_response()
}

fn unauthorized(detail: &str) -> Response {
    ResponseError::from_status(StatusCode::UNAUTHORIZED)
        .with_title("Unauthorized")
        .with_detail(detail)
        .into_response()
}

pub fn spawn_expired_token_cleanup(
    pool: Pool,
    period: tokio::time::Duration,
) -> (
    tokio::task::JoinHandle<()>,
    tokio_util::sync::CancellationToken,
) {
    let token = tokio_util::sync::CancellationToken::new();
    let mut interval = tokio::time::interval(period);

    let handle = tokio::spawn({
        let token = token.clone();
        async move {
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(err) = purge_expired(&pool).await {
                            // Retried on the next tick; stale rows are harmless until then.
                            warn!(error = %err, "failed to purge expired refresh tokens");
                        }
                    }
                    _ = token.cancelled() => break,
                }
            }
        }
    });

    (handle, token)
}

async fn purge_expired(pool: &Pool) -> Result<u64, Error> {
    Ok(pool
        .get()
        .await
        .map_err(Error::Pool)?
        .execute("DELETE FROM refresh_tokens WHERE expires_at < now()", &[])
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-not-used-anywhere-else";

    fn jwt(issuer: &str) -> Jwt {
        Jwt::new(
            SECRET,
            issuer.to_string(),
            std::time::Duration::from_secs(900),
            std::time::Duration::from_secs(604_800),
        )
    }

    #[test]
    fn secret_must_be_set_and_long_enough() {
        assert!(resolve_secret(None).is_err());
        assert!(resolve_secret(Some(String::new())).is_err());
        assert!(resolve_secret(Some("a".repeat(MIN_SECRET_LEN - 1))).is_err());
        assert_eq!(
            resolve_secret(Some("a".repeat(MIN_SECRET_LEN))).unwrap(),
            "a".repeat(MIN_SECRET_LEN)
        );
    }

    #[tokio::test]
    async fn round_trip_recovers_the_user() {
        let jwt = jwt("cluster-a");
        let user_id = Uuid::now_v7();

        let token = jwt.sign_access(user_id, "alice").unwrap();
        let VerifiedToken::Local(user) = jwt.verify(&token).await.unwrap() else {
            panic!("a locally issued token must produce a local user");
        };

        assert_eq!(user.user_id, user_id);
        assert_eq!(user.username, "alice");
    }

    fn expired_seconds_ago(secs: i64) -> String {
        let at = (Utc::now() - Duration::seconds(secs)).timestamp();
        let claims = Claims {
            sub: Uuid::now_v7().to_string(),
            iss: "cluster-a".to_string(),
            iat: at,
            exp: at,
            preferred_username: "alice".to_string(),
        };

        jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_token_is_rejected_seconds_past_expiry() {
        let jwt = jwt("cluster-a");

        assert!(jwt.verify(&expired_seconds_ago(3)).await.is_ok());
        assert!(jwt.verify(&expired_seconds_ago(30)).await.is_err());
    }
}

#[cfg(test)]
mod oidc_tests {
    use super::*;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::traits::PublicKeyParts;
    use std::sync::LazyLock;

    const KID: &str = "test-signing-key";
    const ISSUER: &str = "https://sso.example.test/realms/patrolli";

    struct TestKey {
        private_pem: String,
        jwks: String,
    }

    /// Generated rather than checked in, so no private key lives in the repo.
    /// Once per test binary: RSA-2048 keygen costs a couple of seconds.
    static KEY: LazyLock<TestKey> = LazyLock::new(|| {
        let private = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let public = private.to_public_key();

        TestKey {
            private_pem: private.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
            jwks: format!(
                r#"{{"keys":[{{"kty":"RSA","use":"sig","alg":"RS256","kid":"{KID}","n":"{}","e":"{}"}}]}}"#,
                B64.encode(public.n().to_bytes_be()),
                B64.encode(public.e().to_bytes_be()),
            ),
        }
    });

    /// Cache pre-loaded and cooldown spent, so callers never reach the network.
    fn oidc_with_key_cached() -> Oidc {
        let jwks: JwkSet = serde_json::from_str(&KEY.jwks).unwrap();

        Oidc {
            issuer: ISSUER.to_string(),
            jwks_url: "http://jwks.invalid/certs".to_string(),
            audience: "patrolli".to_string(),
            username_claim_path: "preferred_username".to_string(),
            http: reqwest::Client::new(),
            keys: RwLock::new(KeyCache {
                by_kid: HashMap::from([(
                    KID.to_string(),
                    DecodingKey::from_jwk(&jwks.keys[0]).unwrap(),
                )]),
                last_fetch: Some(Instant::now()),
                last_fetch_ok: true,
            }),
        }
    }

    fn token(iss: &str, aud: Option<&str>, kid: &str) -> String {
        token_named(iss, aud, kid, Some("oidcuser"))
    }

    fn token_named(iss: &str, aud: Option<&str>, kid: &str, username: Option<&str>) -> String {
        let mut claims = serde_json::json!({
            "sub": "9f1c7a10-0000-0000-0000-000000000001",
            "iss": iss,
            "iat": Utc::now().timestamp(),
            "exp": (Utc::now() + Duration::hours(1)).timestamp(),
        });
        if let Some(aud) = aud {
            claims["aud"] = serde_json::json!(aud);
        }
        if let Some(username) = username {
            claims["preferred_username"] = serde_json::json!(username);
        }
        sign(&claims, kid)
    }

    fn sign(claims: &serde_json::Value, kid: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        jsonwebtoken::encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(KEY.private_pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn an_oidc_token_maps_to_a_subject_and_username() {
        let VerifiedToken::Oidc { subject, username } = oidc_with_key_cached()
            .verify(&token(ISSUER, Some("patrolli"), KID))
            .await
            .unwrap()
        else {
            panic!("an OIDC token must not produce a local user");
        };
        assert_eq!(subject, "9f1c7a10-0000-0000-0000-000000000001");
        assert_eq!(username.as_deref(), Some("oidcuser"));

        // Still verified, but without a username it cannot link an account.
        let VerifiedToken::Oidc { username, .. } = oidc_with_key_cached()
            .verify(&token_named(ISSUER, Some("patrolli"), KID, None))
            .await
            .unwrap()
        else {
            panic!("an OIDC token must not produce a local user");
        };
        assert_eq!(username, None);
    }

    /// An email the provider says is not verified could belong to anyone, so it must
    /// never be offered for linking. A missing flag is allowed (Entra ID never sends it).
    #[tokio::test]
    async fn an_unverified_email_cannot_link_an_account() {
        let mut oidc = oidc_with_key_cached();
        oidc.username_claim_path = "email".to_string();

        for (email_verified, can_link) in [
            (serde_json::json!(false), false),
            (serde_json::json!("false"), false),
            (serde_json::json!(true), true),
            (serde_json::Value::Null, true),
        ] {
            let mut claims = serde_json::json!({
                "sub": "9f1c7a10-0000-0000-0000-000000000001",
                "iss": ISSUER,
                "aud": "patrolli",
                "exp": (Utc::now() + Duration::hours(1)).timestamp(),
                "email": "andi@example.test",
            });
            if email_verified != serde_json::Value::Null {
                claims["email_verified"] = email_verified.clone();
            }

            let VerifiedToken::Oidc { username, .. } =
                oidc.verify(&sign(&claims, KID)).await.unwrap()
            else {
                panic!("an OIDC token must not produce a local user");
            };
            assert_eq!(
                username.is_some(),
                can_link,
                "email_verified = {email_verified}"
            );
        }
    }

    /// The missing-`aud` case is the one worth pinning: `set_audience` alone
    /// accepts it, which would let every client in the realm through.
    #[tokio::test]
    async fn a_token_must_carry_the_expected_claims() {
        let oidc = oidc_with_key_cached();

        assert!(oidc.verify(&token(ISSUER, None, KID)).await.is_err());
        assert!(
            oidc.verify(&token(ISSUER, Some("account"), KID))
                .await
                .is_err()
        );
        assert!(
            oidc.verify(&token(
                "https://sso.example.test/realms/other",
                Some("patrolli"),
                KID
            ))
            .await
            .is_err()
        );
    }

    /// Rotation recovery, and the rate limit that keeps it from being abused.
    #[tokio::test]
    async fn an_unknown_key_id_refetches_the_jwks_at_most_once() {
        use axum::{Router, routing::get};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fetches = Arc::new(AtomicUsize::new(0));
        let counter = fetches.clone();
        let app = Router::new().route(
            "/certs",
            get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { ([("content-type", "application/json")], KEY.jwks.clone()) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mut oidc = oidc_with_key_cached();
        oidc.jwks_url = format!("http://{addr}/certs");
        oidc.keys = RwLock::new(KeyCache::default());

        oidc.verify(&token(ISSUER, Some("patrolli"), KID))
            .await
            .unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 1);

        // A kid the IdP does not publish must not buy another fetch each time.
        for _ in 0..3 {
            assert!(
                oidc.verify(&token(ISSUER, Some("patrolli"), "unpublished"))
                    .await
                    .is_err()
            );
        }
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_unconfigured_issuer_never_reaches_a_verifier() {
        let jwt = Jwt::new(
            "test-secret-not-used-anywhere-else",
            "cluster-a".to_string(),
            std::time::Duration::from_secs(900),
            std::time::Duration::from_secs(604_800),
        );

        assert!(matches!(
            jwt.verify(&token(
                "https://evil.example/realms/x",
                Some("patrolli"),
                KID
            ))
            .await,
            Err(Error::UntrustedIssuer(_))
        ));
    }
}
