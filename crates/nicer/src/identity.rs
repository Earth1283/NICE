use std::fmt;
use std::path::Path;
use std::str::FromStr;

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_ED25519};
use rustls_pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// SHA-256 over a device's `SubjectPublicKeyInfo` (RFC R4).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub fn of_spki(spki_der: &[u8]) -> Self {
        Self(Sha256::digest(spki_der).into())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn short(&self) -> String {
        let hex = hex::encode(&self.0[..8]);
        format!("{} {}", &hex[..8], &hex[8..])
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.short())
    }
}

impl FromStr for Fingerprint {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = hex::decode(&compact)
            .map_err(|e| Error::Identity(format!("fingerprint is not hex: {e}")))?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
            Error::Identity("fingerprint is not 32 octets; a short form cannot be used here".into())
        })?;
        Ok(Self(bytes))
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// This device's long-lived Ed25519 keypair and the certificate that carries it into TLS.
pub struct Identity {
    key_pkcs8: Vec<u8>,
    cert_der: CertificateDer<'static>,
    fingerprint: Fingerprint,
    device_name: String,
}

impl Identity {
    /// Loads the stored key, or creates one on first run. The certificate is rebuilt every
    /// time, so renaming the device does not disturb the identity.
    pub fn load_or_generate(path: &Path, device_name: &str) -> Result<Self> {
        let key_pkcs8 = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let generated = KeyPair::generate_for(&PKCS_ED25519)?.serialize_der();
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                write_private(path, &generated)?;
                generated
            }
            Err(e) => return Err(e.into()),
        };
        Self::from_pkcs8(key_pkcs8, device_name)
    }

    pub fn from_pkcs8(key_pkcs8: Vec<u8>, device_name: &str) -> Result<Self> {
        let key = PrivatePkcs8KeyDer::from(key_pkcs8.as_slice());
        let key_pair = KeyPair::from_pkcs8_der_and_sign_algo(&key, &PKCS_ED25519)
            .map_err(|e| Error::Identity(format!("stored key is not Ed25519 PKCS#8: {e}")))?;

        let fingerprint = Fingerprint::of_spki(&key_pair.public_key_der());

        let mut params = CertificateParams::new(Vec::new())?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, device_name);
        params.distinguished_name = name;
        let cert = params.self_signed(&key_pair)?;

        Ok(Self {
            key_pkcs8,
            cert_der: cert.der().clone(),
            fingerprint,
            device_name: device_name.to_string(),
        })
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn certificate(&self) -> CertificateDer<'static> {
        self.cert_der.clone()
    }

    pub fn private_key(&self) -> PrivatePkcs8KeyDer<'static> {
        PrivatePkcs8KeyDer::from(self.key_pkcs8.clone())
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("device_name", &self.device_name)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    Ok(())
}
