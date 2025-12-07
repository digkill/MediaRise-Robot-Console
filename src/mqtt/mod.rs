//! MQTT поддержка (опционально)

#[cfg(feature = "mqtt")]
pub mod handler;

#[cfg(feature = "mqtt")]
pub use handler::MqttHandler;

#[cfg(feature = "mqtt")]
pub struct MqttService {
    handler: MqttHandler,
    services: Option<std::sync::Arc<crate::services::Services>>,
    storage: Option<crate::storage::Storage>,
}

#[cfg(feature = "mqtt")]
impl MqttService {
    pub fn new(config: &crate::config::MqttConfig) -> anyhow::Result<Self> {
        let handler = MqttHandler::new(config)?;
        Ok(Self {
            handler,
            services: None,
            storage: None,
        })
    }

    pub fn with_services(mut self, services: std::sync::Arc<crate::services::Services>) -> Self {
        self.services = Some(services);
        self
    }

    pub fn with_storage(mut self, storage: crate::storage::Storage) -> Self {
        self.storage = Some(storage);
        self
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        self.handler.connect().await?;

        // Запускаем обработку событий с интеграцией сервисов
        let services_clone = self.services.clone();
        let storage_clone = self.storage.clone();

        // Обновляем handler для использования сервисов
        self.handler
            .handle_events_with_services(services_clone, storage_clone)
            .await?;
        Ok(())
    }

    pub fn get_handler(&self) -> &MqttHandler {
        &self.handler
    }
}
