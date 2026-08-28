use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use nicer_proto::payload::TransportMode;
use nicer_proto::{DEFAULT_MAX_FRAME_SIZE, DEFAULT_PORT, HARD_MAX_FRAME_SIZE, MIN_MAX_FRAME_SIZE};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::{default_download_dir, Paths};

pub const PLAINTEXT_WARNING: &str = "WARNING: YOU ASKED WIRESHARK TO READ YOUR SHITPOSTS";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device_name: String,
    pub listen: SocketAddr,
    pub transport: Transport,
    pub max_frame_size: u32,
    pub download_dir: PathBuf,
    pub discovery: bool,
    pub keylog: Option<PathBuf>,
    pub limits: Limits,
    pub timeouts: Timeouts,
}

/// RFC R14: PLAINTEXT is unreachable without a second, deliberate acknowledgement.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Transport {
    pub mode: Mode,
    pub i_know_wireshark_can_read_this: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Secure,
    Plaintext,
}

impl From<Mode> for TransportMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Secure => TransportMode::Secure,
            Mode::Plaintext => TransportMode::Plaintext,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_streams_per_connection: usize,
    pub max_pending_offers: usize,
    pub max_connections_per_address: usize,
    pub max_object_size: u64,
    pub offers_per_minute: u32,
    pub chat_per_minute: u32,
    pub frames_per_second: u32,
    pub chunk_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_streams_per_connection: 64,
            max_pending_offers: 8,
            max_connections_per_address: 4,
            max_object_size: 8 * 1024 * 1024 * 1024,
            offers_per_minute: 30,
            chat_per_minute: 120,
            frames_per_second: 512,
            chunk_size: MIN_MAX_FRAME_SIZE as usize,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Timeouts {
    pub keepalive_secs: u64,
    pub idle_secs: u64,
    pub handshake_secs: u64,
    pub frame_secs: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            keepalive_secs: 30,
            idle_secs: 90,
            handshake_secs: 10,
            frame_secs: 30,
        }
    }
}

impl Timeouts {
    pub fn keepalive(&self) -> Duration {
        Duration::from_secs(self.keepalive_secs)
    }

    pub fn idle(&self) -> Duration {
        Duration::from_secs(self.idle_secs)
    }

    pub fn handshake(&self) -> Duration {
        Duration::from_secs(self.handshake_secs)
    }

    pub fn frame(&self) -> Duration {
        Duration::from_secs(self.frame_secs)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_name: default_device_name(),
            listen: SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)),
            transport: Transport::default(),
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            download_dir: PathBuf::new(),
            discovery: true,
            keylog: None,
            limits: Limits::default(),
            timeouts: Timeouts::default(),
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.config_file();
        let mut config = match std::fs::read_to_string(&file) {
            Ok(text) => toml::from_str(&text)
                .map_err(|e| Error::Config(format!("{}: {e}", file.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e.into()),
        };
        if config.download_dir.as_os_str().is_empty() {
            config.download_dir = default_download_dir(&paths.data_dir);
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.device_name.trim().is_empty() {
            return Err(Error::Config("device_name is empty".into()));
        }
        if self.max_frame_size < MIN_MAX_FRAME_SIZE {
            return Err(Error::Config(format!(
                "max_frame_size {} is below the mandatory floor of {MIN_MAX_FRAME_SIZE}",
                self.max_frame_size
            )));
        }
        if self.max_frame_size > HARD_MAX_FRAME_SIZE {
            return Err(Error::Config(format!(
                "max_frame_size {} is above the ceiling of {HARD_MAX_FRAME_SIZE}",
                self.max_frame_size
            )));
        }
        if self.limits.chunk_size == 0 || self.limits.chunk_size > self.max_frame_size as usize {
            return Err(Error::Config(format!(
                "chunk_size {} does not fit in max_frame_size {}",
                self.limits.chunk_size, self.max_frame_size
            )));
        }
        if self.mode() == Mode::Plaintext && !self.transport.i_know_wireshark_can_read_this {
            return Err(Error::Config(
                "PLAINTEXT requires transport.i_know_wireshark_can_read_this = true".into(),
            ));
        }
        Ok(())
    }

    pub fn mode(&self) -> Mode {
        self.transport.mode
    }

    pub fn is_secure(&self) -> bool {
        self.mode() == Mode::Secure
    }

    /// The chunk a peer will accept, honouring both ends of the negotiation (RFC R2).
    pub fn chunk_size_for(&self, peer_limit: u32) -> usize {
        self.limits
            .chunk_size
            .min(peer_limit.saturating_sub(nicer_proto::Diff::OFFSET_LEN as u32) as usize)
            .max(1)
    }
}

fn default_device_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "nicer".to_string())
}

/// Remote-supplied names reach a terminal, a notification, and a log. None of them
/// should have to cope with control characters (RFC R13).
pub fn sanitize_display(name: &str, limit: usize) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect();
    if cleaned.trim().is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}
