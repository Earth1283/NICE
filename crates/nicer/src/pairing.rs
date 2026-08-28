use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nicer_proto::payload::OfferKind;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::identity::Fingerprint;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairedPeer {
    pub fingerprint: Fingerprint,
    pub device: String,
    pub paired_at: u64,
    #[serde(default)]
    pub last_seen: Option<u64>,
    #[serde(default)]
    pub last_address: Option<IpAddr>,
    #[serde(default)]
    pub auto_accept_files: bool,
    #[serde(default)]
    pub auto_accept_clipboard: bool,
}

impl PairedPeer {
    /// Auto-acceptance is per peer and per kind, and is never on by default (RFC R6).
    pub fn auto_accepts(&self, kind: OfferKind) -> bool {
        match kind {
            OfferKind::File => self.auto_accept_files,
            OfferKind::Clipboard => self.auto_accept_clipboard,
        }
    }
}

/// Trust on first use, confirmed by a human, persisted as JSON (RFC R5).
#[derive(Debug)]
pub struct PairingStore {
    path: PathBuf,
    peers: BTreeMap<Fingerprint, PairedPeer>,
}

impl PairingStore {
    pub fn load(path: &Path) -> Result<Self> {
        let peers = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<Vec<PairedPeer>>(&bytes)
                .unwrap_or_default()
                .into_iter()
                .map(|peer| (peer.fingerprint, peer))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path: path.to_path_buf(),
            peers,
        })
    }

    pub fn get(&self, fingerprint: &Fingerprint) -> Option<&PairedPeer> {
        self.peers.get(fingerprint)
    }

    pub fn is_paired(&self, fingerprint: &Fingerprint) -> bool {
        self.peers.contains_key(fingerprint)
    }

    pub fn list(&self) -> impl Iterator<Item = &PairedPeer> {
        self.peers.values()
    }

    pub fn pair(&mut self, fingerprint: Fingerprint, device: &str) -> Result<&PairedPeer> {
        self.peers.entry(fingerprint).or_insert_with(|| PairedPeer {
            fingerprint,
            device: device.to_string(),
            paired_at: now(),
            last_seen: None,
            last_address: None,
            auto_accept_files: false,
            auto_accept_clipboard: false,
        });
        self.save()?;
        Ok(&self.peers[&fingerprint])
    }

    pub fn unpair(&mut self, fingerprint: &Fingerprint) -> Result<bool> {
        let removed = self.peers.remove(fingerprint).is_some();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn set_auto_accept(
        &mut self,
        fingerprint: &Fingerprint,
        kind: OfferKind,
        enabled: bool,
    ) -> Result<bool> {
        let Some(peer) = self.peers.get_mut(fingerprint) else {
            return Ok(false);
        };
        match kind {
            OfferKind::File => peer.auto_accept_files = enabled,
            OfferKind::Clipboard => peer.auto_accept_clipboard = enabled,
        }
        self.save()?;
        Ok(true)
    }

    pub fn observe(&mut self, fingerprint: &Fingerprint, address: IpAddr, device: &str) {
        if let Some(peer) = self.peers.get_mut(fingerprint) {
            peer.last_seen = Some(now());
            peer.last_address = Some(address);
            if !device.is_empty() {
                peer.device = device.to_string();
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let peers: Vec<_> = self.peers.values().collect();
        let json = serde_json::to_vec_pretty(&peers).map_err(std::io::Error::other)?;
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, &self.path)?;
        Ok(())
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}
