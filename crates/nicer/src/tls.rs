use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};

use crate::error::{Error, Result};
use crate::identity::{Fingerprint, Identity};

/// The certificate carries no usable name, so the handshake is given a placeholder that
/// the verifier ignores (RFC R4).
pub const PLACEHOLDER_SERVER_NAME: &str = "nice.invalid";

fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .clone()
}

/// Reads the `SubjectPublicKeyInfo` out of a certificate and hashes it.
pub fn fingerprint_of(cert: &CertificateDer<'_>) -> Result<Fingerprint> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|e| Error::Certificate(format!("unparseable certificate: {e}")))?;
    Ok(Fingerprint::of_spki(parsed.public_key().raw))
}

fn validate_chain(
    end_entity: &CertificateDer<'_>,
    intermediates: &[CertificateDer<'_>],
    expected: Option<Fingerprint>,
) -> std::result::Result<Fingerprint, rustls::Error> {
    if !intermediates.is_empty() {
        return Err(rustls::Error::General(
            "NICE/1 peers present exactly one self-signed certificate".into(),
        ));
    }

    let (_, parsed) = x509_parser::parse_x509_certificate(end_entity.as_ref())
        .map_err(|e| rustls::Error::General(format!("unparseable certificate: {e}")))?;

    parsed
        .verify_signature(None)
        .map_err(|e| rustls::Error::General(format!("certificate is not self-signed: {e}")))?;

    let observed = Fingerprint::of_spki(parsed.public_key().raw);
    if let Some(expected) = expected {
        if observed != expected {
            return Err(rustls::Error::General(format!(
                "identity changed: expected {}, got {}",
                expected.short(),
                observed.short()
            )));
        }
    }
    Ok(observed)
}

#[derive(Debug)]
struct PinnedPeer {
    provider: Arc<CryptoProvider>,
    expected: Option<Fingerprint>,
}

impl PinnedPeer {
    fn new(expected: Option<Fingerprint>) -> Self {
        Self {
            provider: provider(),
            expected,
        }
    }

    fn schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }

    fn tls13(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
}

impl ServerCertVerifier for PinnedPeer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        validate_chain(end_entity, intermediates, self.expected)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls13RequiredForQuic,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.tls13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes()
    }
}

impl ClientCertVerifier for PinnedPeer {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        validate_chain(end_entity, intermediates, self.expected)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls13RequiredForQuic,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.tls13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes()
    }
}

#[derive(Debug)]
struct FileKeyLog(Mutex<std::fs::File>);

impl rustls::KeyLog for FileKeyLog {
    fn log(&self, label: &str, client_random: &[u8], secret: &[u8]) {
        if let Ok(mut file) = self.0.lock() {
            let _ = writeln!(
                file,
                "{label} {} {}",
                hex::encode(client_random),
                hex::encode(secret)
            );
        }
    }
}

fn key_log(path: Option<&Path>) -> Result<Option<Arc<dyn rustls::KeyLog>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(Some(Arc::new(FileKeyLog(Mutex::new(file)))))
}

fn base() -> rustls::ConfigBuilder<ClientConfig, rustls::WantsVerifier> {
    ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 is supported by the aws-lc-rs provider")
}

/// `expected` pins a peer whose identity is already stored; `None` is a first contact
/// whose fingerprint the caller confirms against the store after the handshake.
pub fn client_config(
    identity: &Identity,
    expected: Option<Fingerprint>,
    keylog: Option<&Path>,
) -> Result<ClientConfig> {
    let mut config = base()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedPeer::new(expected)))
        .with_client_auth_cert(
            vec![identity.certificate()],
            PrivateKeyDer::Pkcs8(identity.private_key()),
        )?;
    if let Some(log) = key_log(keylog)? {
        config.key_log = log;
    }
    Ok(config)
}

pub fn server_config(identity: &Identity, keylog: Option<&Path>) -> Result<ServerConfig> {
    let mut config = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 is supported by the aws-lc-rs provider")
        .with_client_cert_verifier(Arc::new(PinnedPeer::new(None)))
        .with_single_cert(
            vec![identity.certificate()],
            PrivateKeyDer::Pkcs8(identity.private_key()),
        )?;
    if let Some(log) = key_log(keylog)? {
        config.key_log = log;
    }
    Ok(config)
}
