//! Память робота на каждое устройство.
//!
//! Два источника контекста для LLM:
//! 1. Долговременные факты (`device_memory`) — заметки, которые LLM сам решает
//!    сохранить через поле "remember" в JSON-ответе (имя хозяина, предпочтения…).
//! 2. Недавний диалог — последние реплики пользователя ("stt") и робота ("llm")
//!    из `session_messages`, связанные с устройством через `sessions.device_id`.

use anyhow::Context;
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::storage::Storage;

/// Одна реплика недавнего диалога.
#[derive(Debug, Clone)]
pub struct DialogTurn {
    /// "user" (распознанная речь) или "assistant" (ответ робота)
    pub role: String,
    pub content: String,
}

pub struct MemoryService {
    storage: Storage,
}

impl MemoryService {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    /// Сохраняет долговременный факт для устройства (с опциональным эмбеддингом
    /// для семантического поиска).
    pub async fn add_memory(
        &self,
        device_id: &str,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> anyhow::Result<()> {
        let content = content.trim();
        if content.is_empty() {
            return Ok(());
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let embedding_bytes =
            embedding.map(crate::services::embedding::embedding_to_bytes);
        const SQL_QM: &str = "INSERT INTO device_memory (id, device_id, content, created_at, embedding) VALUES (?, ?, ?, ?, ?)";
        const SQL_PG: &str = "INSERT INTO device_memory (id, device_id, content, created_at, embedding) VALUES ($1, $2, $3, $4, $5)";
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                sqlx::query(SQL_QM)
                    .bind(id)
                    .bind(device_id)
                    .bind(content)
                    .bind(now)
                    .bind(embedding_bytes)
                    .execute(pool)
                    .await
                    .context("Failed to insert device memory (sqlite)")?;
            }
            crate::storage::database::Database::Postgres(pool) => {
                sqlx::query(SQL_PG)
                    .bind(id)
                    .bind(device_id)
                    .bind(content)
                    .bind(now)
                    .bind(embedding_bytes)
                    .execute(pool)
                    .await
                    .context("Failed to insert device memory (postgres)")?;
            }
            crate::storage::database::Database::Mysql(pool) => {
                sqlx::query(SQL_QM)
                    .bind(id)
                    .bind(device_id)
                    .bind(content)
                    .bind(now)
                    .bind(embedding_bytes)
                    .execute(pool)
                    .await
                    .context("Failed to insert device memory (mysql)")?;
            }
        }
        Ok(())
    }

    /// Семантический поиск: топ-K фактов, ближайших по смыслу к запросу.
    /// Записи без эмбеддинга получают score 0 и попадают в хвост.
    pub async fn search_memories(
        &self,
        device_id: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<String>> {
        const SQL_QM: &str =
            "SELECT content, embedding FROM device_memory WHERE device_id = ? ORDER BY created_at DESC LIMIT 500";
        const SQL_PG: &str =
            "SELECT content, embedding FROM device_memory WHERE device_id = $1 ORDER BY created_at DESC LIMIT 500";

        fn collect(rows: Vec<(String, Option<Vec<u8>>)>, query: &[f32], limit: usize) -> Vec<String> {
            let mut scored: Vec<(f32, String)> = rows
                .into_iter()
                .map(|(content, blob)| {
                    let score = blob
                        .as_deref()
                        .and_then(crate::services::embedding::bytes_to_embedding)
                        .map(|e| crate::services::embedding::cosine_similarity(query, &e))
                        .unwrap_or(0.0);
                    (score, content)
                })
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            scored.into_iter().take(limit).map(|(_, c)| c).collect()
        }

        let rows: Vec<(String, Option<Vec<u8>>)> = match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => sqlx::query(SQL_QM)
                .bind(device_id)
                .fetch_all(pool)
                .await
                .context("Failed to search device memory (sqlite)")?
                .into_iter()
                .filter_map(|row| {
                    Some((row.try_get("content").ok()?, row.try_get("embedding").ok()))
                })
                .collect(),
            crate::storage::database::Database::Postgres(pool) => sqlx::query(SQL_PG)
                .bind(device_id)
                .fetch_all(pool)
                .await
                .context("Failed to search device memory (postgres)")?
                .into_iter()
                .filter_map(|row| {
                    Some((row.try_get("content").ok()?, row.try_get("embedding").ok()))
                })
                .collect(),
            crate::storage::database::Database::Mysql(pool) => sqlx::query(SQL_QM)
                .bind(device_id)
                .fetch_all(pool)
                .await
                .context("Failed to search device memory (mysql)")?
                .into_iter()
                .filter_map(|row| {
                    Some((row.try_get("content").ok()?, row.try_get("embedding").ok()))
                })
                .collect(),
        };
        Ok(collect(rows, query_embedding, limit))
    }

    /// Последние сохранённые факты устройства (свежие первыми).
    pub async fn list_memories(&self, device_id: &str, limit: i64) -> anyhow::Result<Vec<String>> {
        const SQL_QM: &str =
            "SELECT content FROM device_memory WHERE device_id = ? ORDER BY created_at DESC LIMIT ?";
        const SQL_PG: &str =
            "SELECT content FROM device_memory WHERE device_id = $1 ORDER BY created_at DESC LIMIT $2";
        let contents: Vec<String> = match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => sqlx::query(SQL_QM)
                .bind(device_id)
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load device memory (sqlite)")?
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("content").ok())
                .collect(),
            crate::storage::database::Database::Postgres(pool) => sqlx::query(SQL_PG)
                .bind(device_id)
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load device memory (postgres)")?
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("content").ok())
                .collect(),
            crate::storage::database::Database::Mysql(pool) => sqlx::query(SQL_QM)
                .bind(device_id)
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load device memory (mysql)")?
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("content").ok())
                .collect(),
        };
        Ok(contents)
    }

    /// Последние реплики диалога устройства в хронологическом порядке.
    /// Берёт "stt" (пользователь) и "llm" (робот) из логов сессий.
    pub async fn recent_dialog(
        &self,
        device_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<DialogTurn>> {
        const SQL_QM: &str = "SELECT sm.message_type, sm.payload \
             FROM session_messages sm JOIN sessions s ON s.id = sm.session_id \
             WHERE s.device_id = ? AND sm.message_type IN ('stt', 'llm') \
             ORDER BY sm.created_at DESC LIMIT ?";
        const SQL_PG: &str = "SELECT sm.message_type, sm.payload \
             FROM session_messages sm JOIN sessions s ON s.id = sm.session_id \
             WHERE s.device_id = $1 AND sm.message_type IN ('stt', 'llm') \
             ORDER BY sm.created_at DESC LIMIT $2";

        fn map_turn(message_type: String, payload: String) -> DialogTurn {
            DialogTurn {
                role: if message_type == "stt" { "user" } else { "assistant" }.to_string(),
                content: payload,
            }
        }

        let mut turns: Vec<DialogTurn> = match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => sqlx::query(SQL_QM)
                .bind(device_id)
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load recent dialog (sqlite)")?
                .into_iter()
                .filter_map(|row| {
                    Some(map_turn(
                        row.try_get("message_type").ok()?,
                        row.try_get("payload").ok()?,
                    ))
                })
                .collect(),
            crate::storage::database::Database::Postgres(pool) => sqlx::query(SQL_PG)
                .bind(device_id)
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load recent dialog (postgres)")?
                .into_iter()
                .filter_map(|row| {
                    Some(map_turn(
                        row.try_get("message_type").ok()?,
                        row.try_get("payload").ok()?,
                    ))
                })
                .collect(),
            crate::storage::database::Database::Mysql(pool) => sqlx::query(SQL_QM)
                .bind(device_id)
                .bind(limit)
                .fetch_all(pool)
                .await
                .context("Failed to load recent dialog (mysql)")?
                .into_iter()
                .filter_map(|row| {
                    Some(map_turn(
                        row.try_get("message_type").ok()?,
                        row.try_get("payload").ok()?,
                    ))
                })
                .collect(),
        };
        turns.reverse(); // запрос вернул свежие первыми — разворачиваем в хронологию
        Ok(turns)
    }
}
