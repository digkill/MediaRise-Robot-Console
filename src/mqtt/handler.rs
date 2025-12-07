//! MQTT обработчик

use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, QoS};
use std::time::Duration;
use tracing::{error, info, warn};

use crate::config::MqttConfig;

pub struct MqttHandler {
    client: AsyncClient,
    event_loop: EventLoop,
}

impl MqttHandler {
    pub fn new(config: &MqttConfig) -> anyhow::Result<Self> {
        // Парсим broker URL (может быть "localhost:1883" или "mqtt://localhost:1883")
        let (host, port) = Self::parse_broker_url(&config.broker)?;

        let mut mqtt_options = MqttOptions::new(&config.client_id, &host, port);
        mqtt_options.set_keep_alive(Duration::from_secs(60));
        mqtt_options.set_clean_session(true);

        let (client, event_loop) = AsyncClient::new(mqtt_options, 10);

        Ok(Self { client, event_loop })
    }

    /// Парсит URL брокера в формат (host, port)
    fn parse_broker_url(broker: &str) -> anyhow::Result<(String, u16)> {
        // Убираем протокол, если есть
        let broker = broker
            .strip_prefix("mqtt://")
            .or_else(|| broker.strip_prefix("mqtts://"))
            .unwrap_or(broker);

        // Разделяем на host и port
        if let Some((host, port_str)) = broker.split_once(':') {
            let port = port_str.parse::<u16>()?;
            Ok((host.to_string(), port))
        } else {
            // Если порт не указан, используем стандартный 1883
            Ok((broker.to_string(), 1883))
        }
    }

    /// Подключается к брокеру и подписывается на топики
    pub async fn connect(&mut self) -> anyhow::Result<()> {
        info!("Connecting to MQTT broker...");

        // Подписываемся на топики для устройств
        // Формат: xiaozhi/device/{device_id}/command
        // Формат: xiaozhi/device/{device_id}/status
        self.client
            .subscribe("xiaozhi/+/command", QoS::AtMostOnce)
            .await?;
        self.client
            .subscribe("xiaozhi/+/status", QoS::AtMostOnce)
            .await?;
        self.client
            .subscribe("xiaozhi/broadcast", QoS::AtMostOnce)
            .await?;

        info!("Subscribed to MQTT topics");

        Ok(())
    }

    /// Публикует сообщение в топик
    pub async fn publish(&self, topic: &str, payload: &[u8], qos: QoS) -> anyhow::Result<()> {
        self.client.publish(topic, qos, false, payload).await?;
        Ok(())
    }

    /// Публикует сообщение для конкретного устройства
    pub async fn publish_to_device(
        &self,
        device_id: &str,
        topic: &str,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        let full_topic = format!("xiaozhi/device/{}/{}", device_id, topic);
        self.publish(&full_topic, payload, QoS::AtLeastOnce).await
    }

    /// Обрабатывает входящие события MQTT
    pub async fn handle_events(&mut self) -> anyhow::Result<()> {
        self.handle_events_with_services(None, None).await
    }

    /// Обрабатывает входящие события MQTT с интеграцией сервисов
    pub async fn handle_events_with_services(
        &mut self,
        _services: Option<std::sync::Arc<crate::services::Services>>,
        _storage: Option<crate::storage::Storage>,
    ) -> anyhow::Result<()> {
        loop {
            match self.event_loop.poll().await {
                Ok(Event::Incoming(packet)) => {
                    match packet {
                        rumqttc::Packet::ConnAck(_) => {
                            info!("MQTT connection acknowledged");
                        }
                        rumqttc::Packet::Publish(publish) => {
                            let topic = publish.topic;
                            let payload = publish.payload;

                            info!(
                                "Received MQTT message on topic: {} ({} bytes)",
                                topic,
                                payload.len()
                            );

                            // Парсим топик для определения устройства
                            if let Some(device_id) = Self::extract_device_id(&topic) {
                                info!("Message for device: {}", device_id);

                                // Определяем тип сообщения по топику
                                if topic.ends_with("/command") {
                                    // Команда для устройства - можно отправить через WebSocket если есть активная сессия
                                    if let Ok(payload_str) = String::from_utf8(payload.clone()) {
                                        info!("Command for device {}: {}", device_id, payload_str);
                                        // TODO: Отправить команду через WebSocket сессию устройства
                                    }
                                } else if topic.ends_with("/status") {
                                    // Статус от устройства - обновляем в базе данных
                                    if let Ok(status_str) = String::from_utf8(payload.clone()) {
                                        info!(
                                            "Status update from device {}: {}",
                                            device_id, status_str
                                        );
                                        // TODO: Обновить статус устройства в БД
                                    }
                                }
                            } else if topic == "xiaozhi/broadcast" {
                                // Широковещательное сообщение
                                if let Ok(broadcast_str) = String::from_utf8(payload.clone()) {
                                    info!("Broadcast message: {}", broadcast_str);
                                    // TODO: Отправить всем активным устройствам
                                }
                            }
                        }
                        rumqttc::Packet::SubAck(_) => {
                            info!("MQTT subscription acknowledged");
                        }
                        _ => {}
                    }
                }
                Ok(Event::Outgoing(_)) => {
                    // Исходящие события обрабатываются автоматически
                }
                Err(e) => {
                    error!("MQTT event loop error: {}", e);
                    return Err(e.into());
                }
            }
        }
    }

    /// Извлекает device_id из топика
    /// Формат топика: xiaozhi/device/{device_id}/{action}
    fn extract_device_id(topic: &str) -> Option<String> {
        let parts: Vec<&str> = topic.split('/').collect();
        if parts.len() >= 3 && parts[0] == "xiaozhi" && parts[1] == "device" {
            Some(parts[2].to_string())
        } else {
            None
        }
    }
}
