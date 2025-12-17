//! Сервис кастомных знаний для LLM

use anyhow::Context;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::storage::Storage;

#[derive(Debug, Clone)]
pub struct KnowledgeEntry {
    pub id: Uuid,
    pub title: String,
    pub content: String,
    pub tags: Option<Vec<String>>,
    pub metadata: Option<Value>,
}

pub struct KnowledgeService {
    storage: Storage,
}

impl KnowledgeService {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    /// Возвращает последние записи из кастомной базы знаний
    pub async fn list_recent(&self, limit: i64) -> anyhow::Result<Vec<KnowledgeEntry>> {
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                let rows: Vec<sqlx::sqlite::SqliteRow> = sqlx::query(
                    "SELECT id, title, content, tags, metadata FROM custom_knowledge ORDER BY updated_at DESC LIMIT ?",
                )
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load custom knowledge (sqlite)")?;
                Ok(rows.into_iter().filter_map(Self::map_sqlite_row).collect())
            }
            crate::storage::database::Database::Postgres(pool) => {
                let rows: Vec<sqlx::postgres::PgRow> = sqlx::query(
                    "SELECT id, title, content, tags, metadata FROM custom_knowledge ORDER BY updated_at DESC LIMIT $1",
                )
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load custom knowledge (postgres)")?;
                Ok(rows.into_iter().filter_map(Self::map_postgres_row).collect())
            }
            crate::storage::database::Database::Mysql(pool) => {
                let rows: Vec<sqlx::mysql::MySqlRow> = sqlx::query(
                    "SELECT id, title, content, tags, metadata FROM custom_knowledge ORDER BY updated_at DESC LIMIT ?",
                )
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load custom knowledge (mysql)")?;
                Ok(rows.into_iter().filter_map(Self::map_mysql_row).collect())
            }
        }
    }

    fn map_sqlite_row(row: sqlx::sqlite::SqliteRow) -> Option<KnowledgeEntry> {
        Self::map_from_parts(
            row.try_get("id").ok()?,
            row.try_get("title").ok()?,
            row.try_get("content").ok()?,
            row.try_get("tags").ok(),
            row.try_get("metadata").ok(),
        )
    }

    fn map_postgres_row(row: sqlx::postgres::PgRow) -> Option<KnowledgeEntry> {
        Self::map_from_parts(
            row.try_get("id").ok()?,
            row.try_get("title").ok()?,
            row.try_get("content").ok()?,
            row.try_get("tags").ok(),
            row.try_get("metadata").ok(),
        )
    }

    fn map_mysql_row(row: sqlx::mysql::MySqlRow) -> Option<KnowledgeEntry> {
        Self::map_from_parts(
            row.try_get("id").ok()?,
            row.try_get("title").ok()?,
            row.try_get("content").ok()?,
            row.try_get("tags").ok(),
            row.try_get("metadata").ok(),
        )
    }

    fn map_from_parts(
        id_str: String,
        title: String,
        content: String,
        tags_str: Option<String>,
        metadata_str: Option<String>,
    ) -> Option<KnowledgeEntry> {
        let id = Uuid::parse_str(&id_str).ok()?;
        let tags = tags_str.and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok());
        let metadata = metadata_str.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        Some(KnowledgeEntry {
            id,
            title,
            content,
            tags,
            metadata,
        })
    }
}
