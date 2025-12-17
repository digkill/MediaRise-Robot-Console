//! База данных

use sqlx::{MySql, Pool, Postgres, Sqlite};
use std::sync::Arc;
use tracing::info;

pub enum Database {
    Sqlite(Pool<Sqlite>),
    Postgres(Pool<Postgres>),
    Mysql(Pool<MySql>),
}

impl Database {
    pub async fn new(url: &str) -> anyhow::Result<Arc<Self>> {
        let db = if url.starts_with("sqlite:") {
            let pool = sqlx::SqlitePool::connect(url).await?;
            info!("Connected to SQLite database");
            Self::Sqlite(pool)
        } else if url.starts_with("postgresql://") || url.starts_with("postgres://") {
            let pool = sqlx::PgPool::connect(url).await?;
            info!("Connected to PostgreSQL database");
            Self::Postgres(pool)
        } else if url.starts_with("mysql://") || url.starts_with("mariadb://") {
            let pool = sqlx::MySqlPool::connect(url).await?;
            info!("Connected to MySQL database");
            Self::Mysql(pool)
        } else {
            anyhow::bail!(
                "Unsupported database URL: {}. Supported formats: sqlite:, postgresql://, mysql://",
                url
            );
        };
        Ok(Arc::new(db))
    }
}
