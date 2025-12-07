//! Управление устройствами

use anyhow::Context;
use chrono::Utc;
use sqlx::Row;
use tracing::info;

use crate::storage::Storage;

pub struct DeviceService {
    storage: Storage,
}

impl DeviceService {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn get_device(&self, device_id: &str) -> anyhow::Result<Option<Device>> {
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                let row = sqlx::query(
                    "SELECT device_id, client_id, serial_number, firmware_version, activated, last_seen FROM devices WHERE device_id = ?"
                )
                .bind(device_id)
                .fetch_optional(pool)
                .await
                .context("Failed to query device")?;

                Ok(row.map(|r| {
                    DeviceRow {
                        device_id: r.get::<String, _>("device_id"),
                        client_id: r.get::<String, _>("client_id"),
                        serial_number: r.get::<Option<String>, _>("serial_number"),
                        firmware_version: r.get::<String, _>("firmware_version"),
                        activated: r.get::<bool, _>("activated"),
                        last_seen: r.get::<chrono::DateTime<chrono::Utc>, _>("last_seen"),
                    }
                    .into()
                }))
            }
            crate::storage::database::Database::Postgres(pool) => {
                let row = sqlx::query(
                    "SELECT device_id, client_id, serial_number, firmware_version, activated, last_seen FROM devices WHERE device_id = $1"
                )
                .bind(device_id)
                .fetch_optional(pool)
                .await
                .context("Failed to query device")?;

                Ok(row.map(|r| {
                    DeviceRow {
                        device_id: r.get::<String, _>("device_id"),
                        client_id: r.get::<String, _>("client_id"),
                        serial_number: r.get::<Option<String>, _>("serial_number"),
                        firmware_version: r.get::<String, _>("firmware_version"),
                        activated: r.get::<bool, _>("activated"),
                        last_seen: r.get::<chrono::DateTime<chrono::Utc>, _>("last_seen"),
                    }
                    .into()
                }))
            }
            crate::storage::database::Database::Mysql(pool) => {
                let row = sqlx::query(
                    "SELECT device_id, client_id, serial_number, firmware_version, activated, last_seen FROM devices WHERE device_id = ?"
                )
                .bind(device_id)
                .fetch_optional(pool)
                .await
                .context("Failed to query device")?;

                Ok(row.map(|r| {
                    DeviceRow {
                        device_id: r.get::<String, _>("device_id"),
                        client_id: r.get::<String, _>("client_id"),
                        serial_number: r.get::<Option<String>, _>("serial_number"),
                        firmware_version: r.get::<String, _>("firmware_version"),
                        activated: r.get::<bool, _>("activated"),
                        last_seen: r.get::<chrono::DateTime<chrono::Utc>, _>("last_seen"),
                    }
                    .into()
                }))
            }
        }
    }

    pub async fn create_device(&self, device: Device) -> anyhow::Result<()> {
        info!("Creating device: {}", device.device_id);

        let now = Utc::now();
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                sqlx::query(
                    "INSERT OR REPLACE INTO devices (device_id, client_id, serial_number, firmware_version, activated, last_seen, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
                )
                .bind(&device.device_id)
                .bind(&device.client_id)
                .bind(&device.serial_number)
                .bind(&device.firmware_version)
                .bind(device.activated)
                .bind(device.last_seen)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to create device")?;
            }
            crate::storage::database::Database::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO devices (device_id, client_id, serial_number, firmware_version, activated, last_seen, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (device_id) DO UPDATE SET client_id = $2, serial_number = $3, firmware_version = $4, activated = $5, last_seen = $6, updated_at = $7"
                )
                .bind(&device.device_id)
                .bind(&device.client_id)
                .bind(&device.serial_number)
                .bind(&device.firmware_version)
                .bind(device.activated)
                .bind(device.last_seen)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to create device")?;
            }
            crate::storage::database::Database::Mysql(pool) => {
                sqlx::query(
                    "INSERT INTO devices (device_id, client_id, serial_number, firmware_version, activated, last_seen, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON DUPLICATE KEY UPDATE client_id = VALUES(client_id), serial_number = VALUES(serial_number), firmware_version = VALUES(firmware_version), activated = VALUES(activated), last_seen = VALUES(last_seen), updated_at = VALUES(updated_at)"
                )
                .bind(&device.device_id)
                .bind(&device.client_id)
                .bind(&device.serial_number)
                .bind(&device.firmware_version)
                .bind(device.activated)
                .bind(device.last_seen)
                .bind(now)
                .execute(pool)
                .await
                .context("Failed to create device")?;
            }
        }

        Ok(())
    }

    pub async fn update_device(&self, device: &Device) -> anyhow::Result<()> {
        info!("Updating device: {}", device.device_id);

        let now = Utc::now();
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE devices SET client_id = ?, serial_number = ?, firmware_version = ?, activated = ?, last_seen = ?, updated_at = ? WHERE device_id = ?"
                )
                .bind(&device.client_id)
                .bind(&device.serial_number)
                .bind(&device.firmware_version)
                .bind(device.activated)
                .bind(device.last_seen)
                .bind(now)
                .bind(&device.device_id)
                .execute(pool)
                .await
                .context("Failed to update device")?;
            }
            crate::storage::database::Database::Postgres(pool) => {
                sqlx::query(
                    "UPDATE devices SET client_id = $1, serial_number = $2, firmware_version = $3, activated = $4, last_seen = $5, updated_at = $6 WHERE device_id = $7"
                )
                .bind(&device.client_id)
                .bind(&device.serial_number)
                .bind(&device.firmware_version)
                .bind(device.activated)
                .bind(device.last_seen)
                .bind(now)
                .bind(&device.device_id)
                .execute(pool)
                .await
                .context("Failed to update device")?;
            }
            crate::storage::database::Database::Mysql(pool) => {
                sqlx::query(
                    "UPDATE devices SET client_id = ?, serial_number = ?, firmware_version = ?, activated = ?, last_seen = ?, updated_at = ? WHERE device_id = ?"
                )
                .bind(&device.client_id)
                .bind(&device.serial_number)
                .bind(&device.firmware_version)
                .bind(device.activated)
                .bind(device.last_seen)
                .bind(now)
                .bind(&device.device_id)
                .execute(pool)
                .await
                .context("Failed to update device")?;
            }
        }

        Ok(())
    }

    pub async fn update_last_seen(&self, device_id: &str) -> anyhow::Result<()> {
        let now = Utc::now();
        match &*self.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                sqlx::query("UPDATE devices SET last_seen = ?, updated_at = ? WHERE device_id = ?")
                    .bind(now)
                    .bind(now)
                    .bind(device_id)
                    .execute(pool)
                    .await
                    .context("Failed to update last_seen")?;
            }
            crate::storage::database::Database::Postgres(pool) => {
                sqlx::query(
                    "UPDATE devices SET last_seen = $1, updated_at = $2 WHERE device_id = $3",
                )
                .bind(now)
                .bind(now)
                .bind(device_id)
                .execute(pool)
                .await
                .context("Failed to update last_seen")?;
            }
            crate::storage::database::Database::Mysql(pool) => {
                sqlx::query("UPDATE devices SET last_seen = ?, updated_at = ? WHERE device_id = ?")
                    .bind(now)
                    .bind(now)
                    .bind(device_id)
                    .execute(pool)
                    .await
                    .context("Failed to update last_seen")?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub device_id: String,
    pub client_id: String,
    pub serial_number: Option<String>,
    pub firmware_version: String,
    pub activated: bool,
    pub last_seen: chrono::DateTime<chrono::Utc>,
}

// Вспомогательная структура для запросов к БД
struct DeviceRow {
    device_id: String,
    client_id: String,
    serial_number: Option<String>,
    firmware_version: String,
    activated: bool,
    last_seen: chrono::DateTime<chrono::Utc>,
}

impl From<DeviceRow> for Device {
    fn from(row: DeviceRow) -> Self {
        Self {
            device_id: row.device_id,
            client_id: row.client_id,
            serial_number: row.serial_number,
            firmware_version: row.firmware_version,
            activated: row.activated,
            last_seen: row.last_seen,
        }
    }
}
