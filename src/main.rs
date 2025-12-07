//! Xiaozhi Backend Server
//!
//! Backend сервер для управления устройствами Xiaozhi ESP32

mod config;
mod handlers;
mod mcp;
mod server;
mod services;
mod storage;
mod utils;
mod websocket;

#[cfg(feature = "mqtt")]
mod mqtt;

use anyhow::Result;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "xiaozhi_backend=info,tower_http=debug".into()),
        )
        .init();

    info!("Starting MediaRise Robot Console Backend Server...");

    // Load configuration
    let config = config::Config::load()?;
    info!(
        "Configuration loaded: STT provider={}, url={:?}, key_present={}, TTS provider={}, url={:?}, key_present={}",
        config.stt.provider,
        config.stt.api_url,
        config.stt.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false),
        config.tts.provider,
        config.tts.api_url,
        config.tts.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false),
    );

    // Initialize storage
    let storage = storage::Storage::new(&config).await?;
    info!("Storage initialized");

    // Initialize services
    let services = services::Services::new(&config, storage.clone()).await?;
    info!("Services initialized");

    // Start MQTT service if enabled
    #[cfg(feature = "mqtt")]
    {
        if let Some(mqtt_config) = &config.mqtt {
            if mqtt_config.enabled {
                info!("Starting MQTT service...");
                let mut mqtt_service = mqtt::MqttService::new(mqtt_config)?
                    .with_services(services.clone())
                    .with_storage(storage.clone());

                // Запускаем MQTT в отдельной задаче
                tokio::spawn(async move {
                    if let Err(e) = mqtt_service.start().await {
                        error!("MQTT service error: {}", e);
                    }
                });
                info!("MQTT service started");
            }
        }
    }

    // Start server
    if let Err(e) = server::start(config, services, storage).await {
        error!("Server error: {}", e);
        return Err(e);
    }

    Ok(())
}
