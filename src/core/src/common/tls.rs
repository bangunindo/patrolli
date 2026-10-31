// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config;
use anyhow::{Context, Result};
use pkcs8::DecodePrivateKey;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    WebPkiSupportedAlgorithms, aws_lc_rs, verify_tls12_signature, verify_tls13_signature,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme};
use std::sync::Arc;
use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};
use tracing::warn;

#[derive(Debug)]
struct NoCertificateVerification {
    schemes: WebPkiSupportedAlgorithms,
}

impl NoCertificateVerification {
    fn new() -> Self {
        Self {
            schemes: aws_lc_rs::default_provider().signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.schemes)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.schemes)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes.supported_schemes()
    }
}

pub fn build_rustls_server_config(tls: &config::Tls) -> Result<ServerConfig> {
    let cert_path = tls
        .cert_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("server TLS requires 'cert_path'"))?;
    let key_path = tls
        .key_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("server TLS requires 'key_path'"))?;

    // Load server certificate chain
    let cert_pem = std::fs::read(cert_path)
        .with_context(|| format!("failed to read server cert: {cert_path}"))?;
    let cert_chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut &cert_pem[..])
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to parse server cert chain")?;

    // Load server private key
    let key = load_private_key(key_path, tls.key_password()?)
        .context("failed to load server private key")?;

    let builder = ServerConfig::builder();

    let config = if let Some(ca_path) = &tls.ca_cert_path {
        // mTLS: verify client certificates against this CA
        let ca_pem =
            std::fs::read(ca_path).with_context(|| format!("failed to read CA cert: {ca_path}"))?;
        let ca_certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut &ca_pem[..])
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to parse CA certs")?;

        let mut root_store = rustls::RootCertStore::empty();
        for cert in ca_certs {
            root_store.add(cert).context("failed to add CA cert")?;
        }

        let verifier = rustls::server::WebPkiClientVerifier::builder(root_store.into())
            .build()
            .context("failed to build client cert verifier")?;

        builder
            .with_client_cert_verifier(verifier)
            .with_single_cert(cert_chain, key)
            .context("failed to build server TLS config with mTLS")?
    } else {
        // Standard TLS: no client cert verification
        builder
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)
            .context("failed to build server TLS config")?
    };

    Ok(config)
}
pub fn build_rustls_client_config(tls: &config::Tls) -> Result<ClientConfig> {
    // ── 1. Server certificate verification ─────────────────────────────
    let builder = if tls.insecure_skip_verify {
        warn!("TLS server certificate verification is disabled (insecure_skip_verify=true)");
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification::new()))
    } else {
        let root_store = match &tls.ca_cert_path {
            Some(ca_path) => {
                // Custom CA provided
                let pem = std::fs::read(ca_path)
                    .with_context(|| format!("failed to read CA cert: {ca_path}"))?;
                let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut &pem[..])
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .context("failed to parse CA certs")?;

                let mut store = rustls::RootCertStore::empty();
                for cert in certs {
                    store.add(cert).context("failed to add CA cert to store")?;
                }
                store
            }
            None => {
                // No CA cert → fall back to well-known public roots
                let mut store = rustls::RootCertStore::empty();
                store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                store
            }
        };
        ClientConfig::builder().with_root_certificates(root_store)
    };

    // ── 2. Client authentication (mTLS) ────────────────────────────────
    let config = match (&tls.cert_path, &tls.key_path) {
        (Some(cert_path), Some(key_path)) => {
            // Load client certificate chain
            let cert_pem = std::fs::read(cert_path)
                .with_context(|| format!("failed to read client cert: {cert_path}"))?;
            let cert_chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut &cert_pem[..])
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to parse client cert chain")?;

            // Load private key (possibly encrypted)
            let key = load_private_key(key_path, tls.key_password()?)
                .context("failed to load client private key")?;

            builder
                .with_client_auth_cert(cert_chain, key)
                .context("failed to configure mTLS client auth")?
        }
        (None, None) => {
            // No client certs → plain TLS (server-only verification)
            builder.with_no_client_auth()
        }
        _ => {
            anyhow::bail!(
                "mTLS misconfiguration: both 'cert_path' and 'key_path' must be provided together"
            );
        }
    };

    Ok(config)
}

/// Mapping of our [`config::Tls`] onto tonic's client TLS. tonic builds its own
/// rustls config and only exposes a custom server-cert verifier hook, so the two
/// verification modes have to be applied through different `Endpoint` methods.
pub enum TonicClientTls {
    /// Standard server verification (custom CA or public roots). Apply with
    /// [`tonic::transport::Endpoint::tls_config`].
    Verified(Box<ClientTlsConfig>),
    /// `insecure_skip_verify`: the server certificate is accepted unconditionally.
    /// Apply with [`tonic::transport::Endpoint::tls_config_with_verifier`].
    Insecure(Box<ClientTlsConfig>, Arc<dyn ServerCertVerifier>),
}

/// Builds tonic client TLS from our config, mirroring [`build_rustls_client_config`]
/// so the OTAP forwarder and the rest of the app share one TLS surface (custom CA,
/// public-root fallback, mTLS with encrypted keys, `insecure_skip_verify`).
///
/// ALPN is left to negotiate normally: tonic offers `h2` and our peers (patrolli's
/// own tonic server, and OTel's OTAP receivers) advertise it, so HTTP/2 is agreed
/// during the handshake — the interoperable default. tonic refuses the connection
/// if `h2` is not negotiated, which is the behaviour we want for OTAP interop.
pub fn build_tonic_client_tls(tls: &config::Tls) -> Result<TonicClientTls> {
    let identity = build_tonic_identity(tls)?;

    if tls.insecure_skip_verify {
        warn!("TLS server certificate verification is disabled (insecure_skip_verify=true)");
        let mut config = ClientTlsConfig::new();
        if let Some(identity) = identity {
            config = config.identity(identity);
        }
        // tonic rejects a custom verifier combined with any configured roots, so
        // we deliberately set none here.
        return Ok(TonicClientTls::Insecure(
            Box::new(config),
            Arc::new(NoCertificateVerification::new()),
        ));
    }

    let mut config = match &tls.ca_cert_path {
        Some(ca_path) => {
            let pem = std::fs::read(ca_path)
                .with_context(|| format!("failed to read CA cert: {ca_path}"))?;
            ClientTlsConfig::new().ca_certificate(Certificate::from_pem(pem))
        }
        // No CA → trust the well-known public roots, same fallback as
        // build_rustls_client_config. Supplied as trust anchors directly so we
        // don't need tonic's `tls-webpki-roots` feature.
        None => {
            ClientTlsConfig::new().trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
        }
    };
    if let Some(identity) = identity {
        config = config.identity(identity);
    }
    Ok(TonicClientTls::Verified(Box::new(config)))
}

/// Builds tonic server TLS from our config, mirroring [`build_rustls_server_config`].
/// tonic's server automatically advertises ALPN `h2`, so OTAP clients (ours and
/// OTel's) negotiate HTTP/2 properly during the handshake. A configured
/// `ca_cert_path` turns on mTLS: client certificates are required and verified
/// against that CA.
pub fn build_tonic_server_tls(tls: &config::Tls) -> Result<ServerTlsConfig> {
    let cert_path = tls
        .cert_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("server TLS requires 'cert_path'"))?;
    let key_path = tls
        .key_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("server TLS requires 'key_path'"))?;

    let cert_pem = std::fs::read(cert_path)
        .with_context(|| format!("failed to read server cert: {cert_path}"))?;
    let key_pem = load_private_key_pem(key_path, tls.key_password()?)?;

    let mut config = ServerTlsConfig::new().identity(Identity::from_pem(cert_pem, key_pem));
    if let Some(ca_path) = &tls.ca_cert_path {
        let ca_pem =
            std::fs::read(ca_path).with_context(|| format!("failed to read CA cert: {ca_path}"))?;
        config = config.client_ca_root(Certificate::from_pem(ca_pem));
    }
    Ok(config)
}

/// Builds tonic's client [`Identity`] (mTLS) when both cert and key are set.
fn build_tonic_identity(tls: &config::Tls) -> Result<Option<Identity>> {
    match (&tls.cert_path, &tls.key_path) {
        (Some(cert_path), Some(key_path)) => {
            let cert_pem = std::fs::read(cert_path)
                .with_context(|| format!("failed to read client cert: {cert_path}"))?;
            let key_pem = load_private_key_pem(key_path, tls.key_password()?)?;
            Ok(Some(Identity::from_pem(cert_pem, key_pem)))
        }
        (None, None) => Ok(None),
        _ => anyhow::bail!(
            "mTLS misconfiguration: both 'cert_path' and 'key_path' must be provided together"
        ),
    }
}

/// Returns plaintext PEM key bytes for tonic's [`Identity::from_pem`], which only
/// accepts unencrypted PEM. An encrypted PKCS#8 key is decrypted (matching
/// [`load_private_key`]) and re-emitted as plaintext PKCS#8 PEM.
pub(crate) fn load_private_key_pem(path: &str, password: Option<&str>) -> Result<Vec<u8>> {
    let key_pem =
        std::fs::read(path).with_context(|| format!("failed to read key file: {path}"))?;
    match password {
        Some(passphrase) => {
            let pem_str = std::str::from_utf8(&key_pem).context("key file is not valid UTF-8")?;
            let doc = pkcs8::SecretDocument::from_pkcs8_encrypted_pem(pem_str, passphrase)
                .map_err(|e| anyhow::anyhow!("failed to decrypt PKCS#8 private key: {e}"))?;
            let pem = doc
                .to_pem("PRIVATE KEY", pkcs8::LineEnding::LF)
                .map_err(|e| anyhow::anyhow!("failed to re-encode decrypted private key: {e}"))?;
            Ok(pem.as_bytes().to_vec())
        }
        None => Ok(key_pem),
    }
}

/// Reads a PEM private key from disk.
/// If `password` is provided, the key is treated as encrypted PKCS#8.
fn load_private_key(path: &str, password: Option<&str>) -> Result<PrivateKeyDer<'static>> {
    let key_pem =
        std::fs::read(path).with_context(|| format!("failed to read key file: {path}"))?;

    match password {
        Some(passphrase) => {
            // ── Encrypted PKCS#8 key ───────────────────────────────
            let pem_str = std::str::from_utf8(&key_pem).context("key file is not valid UTF-8")?;

            let doc = pkcs8::SecretDocument::from_pkcs8_encrypted_pem(pem_str, passphrase)
                .map_err(|e| anyhow::anyhow!("failed to decrypt PKCS#8 private key: {e}"))?;

            Ok(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                doc.as_bytes().to_vec(),
            )))
        }
        None => {
            // ── Unencrypted key (RSA / PKCS#8 / EC) ───────────────
            let key = rustls_pemfile::private_key(&mut &key_pem[..])
                .context("failed to parse PEM private key")?
                .ok_or_else(|| anyhow::anyhow!("no private key found in {path}"))?;

            Ok(key)
        }
    }
}
