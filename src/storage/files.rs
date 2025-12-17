//! Файловое хранилище

use std::path::PathBuf;
use tracing::info;

use crate::config::StorageConfig;

pub struct FileStorage {
    config: StorageConfig,
}

impl FileStorage {
    pub fn new(config: &StorageConfig) -> anyhow::Result<Self> {
        // Create directories
        std::fs::create_dir_all(&config.firmware_path)?;
        std::fs::create_dir_all(&config.assets_path)?;
        std::fs::create_dir_all(&config.uploads_path)?;

        info!("File storage initialized at: {:?}", config.base_path);

        Ok(Self {
            config: config.clone(),
        })
    }

    pub fn firmware_path(&self) -> &PathBuf {
        &self.config.firmware_path
    }

    pub fn assets_path(&self) -> &PathBuf {
        &self.config.assets_path
    }

    pub fn uploads_path(&self) -> &PathBuf {
        &self.config.uploads_path
    }
}
