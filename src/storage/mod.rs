//! Хранилище данных

pub mod database;
pub mod files;

use std::sync::Arc;

use crate::config::Config;

pub struct Storage {
    pub database: Arc<database::Database>,
    pub files: Arc<files::FileStorage>,
}

impl Storage {
    pub async fn new(config: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            database: database::Database::new(&config.database.url).await?,
            files: Arc::new(files::FileStorage::new(&config.storage)?),
        })
    }
}

impl Clone for Storage {
    fn clone(&self) -> Self {
        Self {
            database: self.database.clone(),
            files: self.files.clone(),
        }
    }
}
