use std::path::{Path, PathBuf};

use bytes::Bytes;
use nicer_proto::payload::{ALGORITHM_BLAKE3, ALGORITHM_SHA256};
use sha2::Digest as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{Error, Result};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Algorithm {
    Blake3,
    Sha256,
}

impl Algorithm {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            ALGORITHM_BLAKE3 => Some(Algorithm::Blake3),
            ALGORITHM_SHA256 => Some(Algorithm::Sha256),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Blake3 => ALGORITHM_BLAKE3,
            Algorithm::Sha256 => ALGORITHM_SHA256,
        }
    }
}

/// The digest algorithm is named in `FSCK`, which arrives after the last `DIFF`, so both
/// supported digests are computed as the object streams past (RFC R8).
#[derive(Default)]
pub struct Digests {
    blake3: blake3::Hasher,
    sha256: sha2::Sha256,
}

impl Digests {
    pub fn update(&mut self, data: &[u8]) {
        self.blake3.update(data);
        self.sha256.update(data);
    }

    pub fn finish(self, algorithm: Algorithm) -> Vec<u8> {
        match algorithm {
            Algorithm::Blake3 => self.blake3.finalize().as_bytes().to_vec(),
            Algorithm::Sha256 => self.sha256.finalize().to_vec(),
        }
    }
}

pub fn digest_matches(computed: &[u8], claimed: &[u8]) -> bool {
    if computed.len() != claimed.len() {
        return false;
    }
    computed
        .iter()
        .zip(claimed)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Reduces a sender-supplied name to a bare filename that cannot escape the download
/// directory (RFC R8).
pub fn sanitize_filename(name: &str) -> String {
    let tail = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .rsplit(':')
        .next()
        .unwrap_or(name);

    let cleaned: String = tail
        .chars()
        .filter(|c| !c.is_control() && *c != '\0')
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();

    if cleaned.is_empty() {
        return "received".to_string();
    }

    let mut truncated = String::new();
    for c in cleaned.chars() {
        if truncated.len() + c.len_utf8() > 200 {
            break;
        }
        truncated.push(c);
    }
    truncated
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// A name that already exists is disambiguated, never overwritten (RFC R8).
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = split_extension(name);
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({n}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!(
        "{stem} ({}){extension}",
        crate::pairing::now_millis()
    ))
}

pub enum Source {
    File(tokio::fs::File),
    Memory(Bytes),
}

/// Reads an object out in chunks while hashing it, so `FSCK` needs no second pass.
pub struct Sending {
    source: Source,
    digest: blake3::Hasher,
    offset: u64,
    size: u64,
}

impl Sending {
    pub async fn from_path(path: &Path) -> Result<(Self, u64)> {
        let file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        Ok((
            Self {
                source: Source::File(file),
                digest: blake3::Hasher::new(),
                offset: 0,
                size,
            },
            size,
        ))
    }

    pub fn from_bytes(bytes: Bytes) -> Self {
        let size = bytes.len() as u64;
        Self {
            source: Source::Memory(bytes),
            digest: blake3::Hasher::new(),
            offset: 0,
            size,
        }
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn is_complete(&self) -> bool {
        self.offset >= self.size
    }

    pub async fn next_chunk(&mut self, max: usize) -> Result<Option<(u64, Bytes)>> {
        if self.is_complete() {
            return Ok(None);
        }
        let remaining = (self.size - self.offset).min(max as u64) as usize;
        let chunk = match &mut self.source {
            Source::File(file) => {
                let mut buffer = vec![0u8; remaining];
                let read = file.read(&mut buffer).await?;
                if read == 0 {
                    return Err(Error::Transfer(format!(
                        "file ended at {} octets, {} short of the offer",
                        self.offset, self.size
                    )));
                }
                buffer.truncate(read);
                Bytes::from(buffer)
            }
            Source::Memory(bytes) => {
                bytes.slice(self.offset as usize..self.offset as usize + remaining)
            }
        };
        let at = self.offset;
        self.digest.update(&chunk);
        self.offset += chunk.len() as u64;
        Ok(Some((at, chunk)))
    }

    pub fn digest(&self) -> Vec<u8> {
        self.digest.finalize().as_bytes().to_vec()
    }
}

enum Destination {
    File {
        temp: PathBuf,
        target: PathBuf,
        handle: tokio::fs::File,
    },
    Memory(Vec<u8>),
}

pub enum Received {
    File(PathBuf),
    Clipboard(String),
}

/// An object is written to a temporary file and only named once `CLEAN` is decided, so a
/// failed transfer never leaves something that looks finished (RFC R8).
pub struct Receiving {
    destination: Destination,
    digests: Digests,
    written: u64,
    expected: u64,
}

impl Receiving {
    pub async fn into_file(dir: &Path, name: &str, expected: u64) -> Result<Self> {
        tokio::fs::create_dir_all(dir).await?;
        let target = unique_path(dir, &sanitize_filename(name));
        let temp = dir.join(format!(
            ".nicer-{}-{}.part",
            crate::pairing::now_millis(),
            std::process::id()
        ));
        let handle = tokio::fs::File::create(&temp).await?;
        Ok(Self {
            destination: Destination::File {
                temp,
                target,
                handle,
            },
            digests: Digests::default(),
            written: 0,
            expected,
        })
    }

    pub fn into_memory(expected: u64) -> Self {
        Self {
            destination: Destination::Memory(Vec::with_capacity(expected as usize)),
            digests: Digests::default(),
            written: 0,
            expected,
        }
    }

    pub fn written(&self) -> u64 {
        self.written
    }

    pub fn expected(&self) -> u64 {
        self.expected
    }

    /// `DIFF` frames are contiguous and ascending; a gap means a broken sender, and there
    /// is no retransmission to repair it (RFC R7).
    pub async fn write(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        if offset != self.written {
            return Err(Error::Transfer(format!(
                "DIFF at offset {offset} does not continue from {}",
                self.written
            )));
        }
        if self.written + data.len() as u64 > self.expected {
            return Err(Error::Transfer(format!(
                "DIFF carries the object past the offered size of {}",
                self.expected
            )));
        }
        match &mut self.destination {
            Destination::File { handle, .. } => handle.write_all(data).await?,
            Destination::Memory(buffer) => buffer.extend_from_slice(data),
        }
        self.digests.update(data);
        self.written += data.len() as u64;
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.written == self.expected
    }

    pub fn verify(&mut self, algorithm: Algorithm, claimed: &[u8]) -> bool {
        let computed = std::mem::take(&mut self.digests).finish(algorithm);
        digest_matches(&computed, claimed)
    }

    pub async fn commit(self) -> Result<Received> {
        match self.destination {
            Destination::File {
                temp,
                target,
                mut handle,
            } => {
                handle.flush().await?;
                handle.sync_all().await?;
                drop(handle);
                tokio::fs::rename(&temp, &target).await?;
                Ok(Received::File(target))
            }
            Destination::Memory(buffer) => {
                let text = String::from_utf8(buffer)
                    .map_err(|_| Error::Transfer("clipboard content is not UTF-8".into()))?;
                Ok(Received::Clipboard(text))
            }
        }
    }

    pub async fn discard(self) {
        if let Destination::File { temp, handle, .. } = self.destination {
            drop(handle);
            let _ = tokio::fs::remove_file(&temp).await;
        }
    }
}
