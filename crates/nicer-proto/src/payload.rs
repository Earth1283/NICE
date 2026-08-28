use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::ProtoError;

pub const MAX_PREVIEW: usize = 256;
pub const MAX_CHAT_TEXT: usize = 4096;
pub const MAX_REASON: usize = 256;
pub const MAX_DEVICE_NAME: usize = 63;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    Secure,
    Plaintext,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfferKind {
    File,
    Clipboard,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub version: u8,
    pub device: String,
    pub fingerprint: String,
    pub transport: TransportMode,
    pub max_frame_size: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Merged {
    pub version: u8,
    pub device: String,
    pub fingerprint: String,
    pub transport: TransportMode,
    pub max_frame_size: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub kind: OfferKind,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

impl PullRequest {
    /// RFC R6: `name` belongs to files, `preview` to clipboard, and neither crosses over.
    pub fn validate(&self) -> Result<(), ProtoError> {
        match self.kind {
            OfferKind::File if self.name.is_none() => {
                return Err(ProtoError::MissingPayload("PULL_REQUEST name"))
            }
            OfferKind::Clipboard if self.name.is_some() => {
                return Err(ProtoError::FieldTooLong {
                    field: "PULL_REQUEST name on a clipboard offer",
                    len: 1,
                    limit: 0,
                })
            }
            _ => {}
        }
        if let Some(preview) = &self.preview {
            bound("PULL_REQUEST preview", preview.len(), MAX_PREVIEW)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FuckOff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BigDiff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShutUpScope {
    Stream,
    Kind,
    #[default]
    Connection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShutUp {
    pub retry_after_ms: u64,
    #[serde(default)]
    pub scope: ShutUpScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

pub const MAX_RETRY_AFTER_MS: u64 = 3_600_000;

impl ShutUp {
    pub fn retry_after_ms(&self) -> u64 {
        self.retry_after_ms.min(MAX_RETRY_AFTER_MS)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fsck {
    pub algorithm: String,
    #[serde(with = "serde_bytes")]
    pub digest: Vec<u8>,
    pub total_size: u64,
}

pub const ALGORITHM_BLAKE3: &str = "BLAKE3";
pub const ALGORITHM_SHA256: &str = "SHA-256";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lkml {
    pub message_id: String,
    pub timestamp: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<BTreeMap<String, String>>,
}

impl Lkml {
    pub fn validate(&self) -> Result<(), ProtoError> {
        bound("LKML text", self.text.len(), MAX_CHAT_TEXT)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Reason {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Reason {
    pub fn new(reason: impl Into<String>) -> Self {
        let mut reason = reason.into();
        reason.truncate(MAX_REASON);
        Self {
            reason: Some(reason),
        }
    }

    pub fn display(&self) -> &str {
        self.reason.as_deref().unwrap_or("no reason given")
    }
}

fn bound(field: &'static str, len: usize, limit: usize) -> Result<(), ProtoError> {
    if len > limit {
        return Err(ProtoError::FieldTooLong { field, len, limit });
    }
    Ok(())
}
