//! OTA (Over-The-Air) endpoints

use crate::services::device::Device as ServiceDevice;
use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tracing::{debug, error, info, warn};

use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub struct CheckVersionRequest {
    // System info from device (optional)
    #[serde(flatten)]
    pub system_info: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct CheckVersionResponse {
    pub firmware: Option<FirmwareInfo>,
    pub activation: Option<ActivationInfo>,
    pub mqtt: Option<MqttConfig>,
    pub websocket: Option<WebSocketConfig>,
    pub server_time: Option<ServerTime>,
}

#[derive(Debug, Serialize)]
pub struct FirmwareInfo {
    pub version: String,
    pub url: String,
    pub force: u8,
}

#[derive(Debug, Serialize)]
pub struct ActivationInfo {
    pub message: String,
    pub code: Option<String>,
    pub challenge: String,
    pub timeout_ms: u32,
}

#[derive(Debug, Serialize)]
pub struct MqttConfig {
    pub endpoint: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub publish_topic: String,
    pub keepalive: u32,
}

#[derive(Debug, Serialize)]
pub struct WebSocketConfig {
    pub url: String,
    pub token: String,
    pub version: u32,
}

#[derive(Debug, Serialize)]
pub struct ServerTime {
    pub timestamp: i64,       // milliseconds
    pub timezone_offset: i32, // minutes
}

/// GET/POST /ota/
/// Проверка версии прошивки и получение конфигурации
pub async fn check_version(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    request: Option<Json<CheckVersionRequest>>,
) -> Result<Json<CheckVersionResponse>, StatusCode> {
    // Extract device info from headers
    let device_id = headers
        .get("Device-Id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");

    let client_id = headers
        .get("Client-Id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");

    let serial_number = headers.get("Serial-Number").and_then(|h| h.to_str().ok());

    debug!(
        "Check version request from device: {}, client: {}",
        device_id, client_id
    );

    // Получаем или создаем устройство
    let device = state
        .services
        .device
        .get_device(device_id)
        .await
        .map_err(|e| {
            error!("Failed to get device: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let device = if let Some(device) = device {
        // Обновляем last_seen
        let _ = state.services.device.update_last_seen(device_id).await;
        device
    } else {
        // Создаем новое устройство
        let new_device = crate::services::device::Device {
            device_id: device_id.to_string(),
            client_id: client_id.to_string(),
            serial_number: serial_number.map(|s| s.to_string()),
            firmware_version: "0.0.0".to_string(),
            activated: false,
            last_seen: chrono::Utc::now(),
        };
        state
            .services
            .device
            .create_device(new_device.clone())
            .await
            .map_err(|e| {
                error!("Failed to create device: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        new_device
    };

    // Получаем последнюю версию прошивки из базы данных
    let firmware = match &*state.storage.database {
        crate::storage::database::Database::Sqlite(pool) => {
            let row = sqlx::query(
                "SELECT version, url, force_update FROM firmware_versions ORDER BY created_at DESC LIMIT 1"
            )
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

            if let Some(row) = row {
                Some(FirmwareInfo {
                    version: row.get::<String, _>("version"),
                    url: row.get::<String, _>("url"),
                    force: if row.get::<bool, _>("force_update") {
                        1
                    } else {
                        0
                    },
                })
            } else {
                None
            }
        }
        crate::storage::database::Database::Postgres(pool) => {
            let row = sqlx::query(
                "SELECT version, url, force_update FROM firmware_versions ORDER BY created_at DESC LIMIT 1"
            )
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

            if let Some(row) = row {
                Some(FirmwareInfo {
                    version: row.get::<String, _>("version"),
                    url: row.get::<String, _>("url"),
                    force: if row.get::<bool, _>("force_update") {
                        1
                    } else {
                        0
                    },
                })
            } else {
                None
            }
        }
        crate::storage::database::Database::Mysql(pool) => {
            let row = sqlx::query(
                "SELECT version, url, force_update FROM firmware_versions ORDER BY created_at DESC LIMIT 1"
            )
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

            if let Some(row) = row {
                Some(FirmwareInfo {
                    version: row.get::<String, _>("version"),
                    url: row.get::<String, _>("url"),
                    force: if row.get::<bool, _>("force_update") {
                        1
                    } else {
                        0
                    },
                })
            } else {
                None
            }
        }
    };

    // Генерируем challenge для активации, если устройство не активировано
    let activation = if !device.activated {
        let challenge = crate::utils::crypto::generate_challenge();
        Some(ActivationInfo {
            message: "Device activation required".to_string(),
            code: None,
            challenge,
            timeout_ms: 30000, // 30 seconds
        })
    } else {
        None
    };

    // MQTT конфигурация
    #[cfg(feature = "mqtt")]
    let mqtt = if let Some(ref mqtt_config) = state.config.mqtt {
        if mqtt_config.enabled {
            Some(MqttConfig {
                endpoint: mqtt_config.broker.clone(),
                client_id: mqtt_config.client_id.clone(),
                username: "".to_string(), // TODO: Add MQTT auth if needed
                password: "".to_string(),
                publish_topic: format!("xiaozhi/device/{}/status", device_id),
                keepalive: 60,
            })
        } else {
            None
        }
    } else {
        None
    };
    #[cfg(not(feature = "mqtt"))]
    let mqtt = None;

    // Генерируем JWT токен для WebSocket
    let token = crate::utils::jwt::generate_jwt_token(
        device_id,
        &device.client_id,
        &state.config.security.jwt_secret,
    )
    .map_err(|e| {
        error!("Failed to generate JWT token: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = CheckVersionResponse {
        firmware,
        activation,
        mqtt,
        websocket: Some(WebSocketConfig {
            url: format!(
                "ws://{}:{}/ws",
                state.config.server.host, state.config.server.websocket_port
            ),
            token,
            version: 3,
        }),
        server_time: Some(ServerTime {
            timestamp: chrono::Utc::now().timestamp_millis(),
            timezone_offset: 0, // TODO: Get from config or system
        }),
    };

    Ok(Json(response))
}

// Вспомогательные структуры для запросов к БД
struct FirmwareRow {
    version: String,
    url: String,
    force_update: bool,
}

struct DeviceRow {
    device_id: String,
    client_id: String,
    serial_number: Option<String>,
    firmware_version: String,
    activated: bool,
    last_seen: chrono::DateTime<chrono::Utc>,
}

impl From<DeviceRow> for ServiceDevice {
    fn from(row: DeviceRow) -> Self {
        ServiceDevice {
            device_id: row.device_id,
            client_id: row.client_id,
            serial_number: row.serial_number,
            firmware_version: row.firmware_version,
            activated: row.activated,
            last_seen: row.last_seen,
        }
    }
}

fn default_activation_algorithm() -> String {
    "hmac_sha256".to_string()
}

#[derive(Debug, Deserialize)]
pub struct ActivateRequest {
    #[serde(default = "default_activation_algorithm")]
    pub algorithm: String,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub challenge: Option<String>,
    #[serde(default)]
    pub hmac: Option<String>,
}

/// POST /ota/activate
/// Активация устройства
pub async fn activate(
    axum::extract::State(_state): axum::extract::State<AppState>,
    _headers: axum::http::HeaderMap,
    Json(request): Json<ActivateRequest>,
) -> Result<StatusCode, StatusCode> {
    let serial_for_log = request
        .serial_number
        .as_deref()
        .unwrap_or("unknown_serial");
    info!("Activation request from device: {}", serial_for_log);

    // Проверяем HMAC
    if let (Some(challenge), Some(hmac)) = (&request.challenge, &request.hmac) {
        let hmac_key = _state.config.security.hmac_key.as_bytes();
        let message = format!("{}{}{}", request.algorithm, serial_for_log, challenge);

        let hmac_valid =
            crate::utils::crypto::verify_hmac(hmac_key, message.as_bytes(), hmac);

        if !hmac_valid {
            warn!(
                "Invalid HMAC for activation request from: {}. Allowing activation.",
                serial_for_log
            );
        }
    } else {
        warn!(
            "Activation request from {} without complete HMAC data. Allowing activation.",
            serial_for_log
        );
    }

    // Находим устройство по serial_number
    if let Some(serial_number) = &request.serial_number {
        let device = match &*_state.storage.database {
            crate::storage::database::Database::Sqlite(pool) => {
                let row = sqlx::query(
                    "SELECT device_id, client_id, serial_number, firmware_version, activated, last_seen FROM devices WHERE serial_number = ?"
                )
                .bind(serial_number)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

                row.map(|r| DeviceRow {
                    device_id: r.get::<String, _>("device_id"),
                    client_id: r.get::<String, _>("client_id"),
                    serial_number: r.get::<Option<String>, _>("serial_number"),
                    firmware_version: r.get::<String, _>("firmware_version"),
                    activated: r.get::<bool, _>("activated"),
                    last_seen: r.get::<chrono::DateTime<chrono::Utc>, _>("last_seen"),
                })
            }
            crate::storage::database::Database::Postgres(pool) => {
                let row = sqlx::query(
                    "SELECT device_id, client_id, serial_number, firmware_version, activated, last_seen FROM devices WHERE serial_number = $1"
                )
                .bind(serial_number)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

                row.map(|r| DeviceRow {
                    device_id: r.get::<String, _>("device_id"),
                    client_id: r.get::<String, _>("client_id"),
                    serial_number: r.get::<Option<String>, _>("serial_number"),
                    firmware_version: r.get::<String, _>("firmware_version"),
                    activated: r.get::<bool, _>("activated"),
                    last_seen: r.get::<chrono::DateTime<chrono::Utc>, _>("last_seen"),
                })
            }
            crate::storage::database::Database::Mysql(pool) => {
                let row = sqlx::query(
                    "SELECT device_id, client_id, serial_number, firmware_version, activated, last_seen FROM devices WHERE serial_number = ?"
                )
                .bind(serial_number)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

                row.map(|r| DeviceRow {
                    device_id: r.get::<String, _>("device_id"),
                    client_id: r.get::<String, _>("client_id"),
                    serial_number: r.get::<Option<String>, _>("serial_number"),
                    firmware_version: r.get::<String, _>("firmware_version"),
                    activated: r.get::<bool, _>("activated"),
                    last_seen: r.get::<chrono::DateTime<chrono::Utc>, _>("last_seen"),
                })
            }
        };

        if let Some(mut device) = device {
            device.activated = true;
            let device: ServiceDevice = device.into();
            _state
                .services
                .device
                .update_device(&device)
                .await
                .map_err(|e| {
                    error!("Failed to activate device: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

            info!("Device activated: {}", serial_number);
        } else {
            warn!(
                "Device not found for serial_number: {}. Allowing activation.",
                serial_number
            );
        }
    } else {
        warn!(
            "Activation request without serial number. Allowing activation without persisting state."
        );
    }

    Ok(StatusCode::OK)
}
