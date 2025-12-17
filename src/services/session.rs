//! Управление сессиями и их логами

use anyhow::Context;
use chrono::Utc;
use uuid::Uuid;

use crate::storage::Storage;

pub enum MessageDirection {
    Incoming,
    Outgoing,
}

impl MessageDirection {
    fn as_str(&self) -> &'static str {
        match self {
            MessageDirection::Incoming => "incoming",
            MessageDirection::Outgoing => "outgoing",
        }
    }
}

pub struct SessionService {
    storage: Storage,
}

impl SessionService {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn persist_session(&self, session_id: &Uuid, device_id: &str) -> anyhow::Result<()> {
        let now = Utc::now();
        let session_id = session_id.to_string();
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO sessions (id, device_id, created_at, last_activity) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET device_id = excluded.device_id, last_activity = excluded.last_activity",
                )
                .bind(&session_id)
                .bind(device_id)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to upsert session (sqlite)")?;
            }
            crate::storage::database::Database::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO sessions (id, device_id, created_at, last_activity) VALUES ($1, $2, $3, $3) ON CONFLICT (id) DO UPDATE SET device_id = EXCLUDED.device_id, last_activity = EXCLUDED.last_activity",
                )
                .bind(&session_id)
                .bind(device_id)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to upsert session (postgres)")?;
            }
            crate::storage::database::Database::Mysql(pool) => {
                sqlx::query(
                    "INSERT INTO sessions (id, device_id, created_at, last_activity) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE device_id = VALUES(device_id), last_activity = VALUES(last_activity)",
                )
                .bind(&session_id)
                .bind(device_id)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to upsert session (mysql)")?;
            }
        }
        Ok(())
    }

    pub async fn close_session(&self, session_id: &Uuid) -> anyhow::Result<()> {
        let now = Utc::now();
        self.update_last_activity(session_id, now).await
    }

    pub async fn log_message(
        &self,
        session_id: &Uuid,
        direction: MessageDirection,
        message_type: &str,
        payload: &str,
    ) -> anyhow::Result<()> {
        let id = Uuid::new_v4().to_string();
        let session_id_str = session_id.to_string();
        let now = Utc::now();

        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO session_messages (id, session_id, direction, message_type, payload, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(id)
                .bind(&session_id_str)
                .bind(direction.as_str())
                .bind(message_type)
                .bind(payload)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to insert session message (sqlite)")?;
            }
            crate::storage::database::Database::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO session_messages (id, session_id, direction, message_type, payload, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(id)
                .bind(&session_id_str)
                .bind(direction.as_str())
                .bind(message_type)
                .bind(payload)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to insert session message (postgres)")?;
            }
            crate::storage::database::Database::Mysql(pool) => {
                sqlx::query(
                    "INSERT INTO session_messages (id, session_id, direction, message_type, payload, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(id)
                .bind(&session_id_str)
                .bind(direction.as_str())
                .bind(message_type)
                .bind(payload)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to insert session message (mysql)")?;
            }
        }

        self.update_last_activity(session_id, now).await?;

        Ok(())
    }

    async fn update_last_activity(
        &self,
        session_id: &Uuid,
        timestamp: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let session_id = session_id.to_string();
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                sqlx::query("UPDATE sessions SET last_activity = ? WHERE id = ?")
                    .bind(timestamp)
                    .bind(&session_id)
                    .execute(pool)
                    .await
                    .context("Failed to update session (sqlite)")?;
            }
            crate::storage::database::Database::Postgres(pool) => {
                sqlx::query("UPDATE sessions SET last_activity = $1 WHERE id = $2")
                    .bind(timestamp)
                    .bind(&session_id)
                    .execute(pool)
                    .await
                    .context("Failed to update session (postgres)")?;
            }
            crate::storage::database::Database::Mysql(pool) => {
                sqlx::query("UPDATE sessions SET last_activity = ? WHERE id = ?")
                    .bind(timestamp)
                    .bind(&session_id)
                    .execute(pool)
                    .await
                    .context("Failed to update session (mysql)")?;
            }
        }
        Ok(())
    }
}
