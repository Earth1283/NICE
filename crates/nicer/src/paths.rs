use std::path::{Path, PathBuf};

use directories::{ProjectDirs, UserDirs};

use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "nicer")
            .ok_or_else(|| Error::Config("no home directory for this user".into()))?;
        let runtime_dir = dirs
            .runtime_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dirs.data_dir().to_path_buf());
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            runtime_dir,
        })
    }

    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            runtime_dir: root.join("run"),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn identity_file(&self) -> PathBuf {
        self.data_dir.join("identity.key")
    }

    pub fn peers_file(&self) -> PathBuf {
        self.data_dir.join("peers.json")
    }

    pub fn control_socket(&self) -> PathBuf {
        crate::control::default_endpoint(&self.runtime_dir)
    }

    pub fn create_all(&self) -> Result<()> {
        for dir in [&self.config_dir, &self.data_dir, &self.runtime_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

pub fn default_download_dir(data_dir: &Path) -> PathBuf {
    UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .map(|downloads| downloads.join("nicer"))
        .unwrap_or_else(|| data_dir.join("received"))
}
