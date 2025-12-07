//! Бизнес-логика сервисов

pub mod audio;
pub mod device;
pub mod llm;
pub mod session;
pub mod stt;
pub mod tts;

use std::sync::Arc;

use crate::config::Config;
use crate::storage::Storage;

#[derive(Clone)]
pub struct Services {
    pub device: Arc<device::DeviceService>,
    pub session: Arc<session::SessionService>,
    pub audio: Arc<audio::AudioService>,
    pub stt: Arc<stt::SttService>,
    pub tts: Arc<tts::TtsService>,
    pub llm: Arc<llm::LlmService>,
}

impl Services {
    pub async fn new(config: &Config, storage: Storage) -> anyhow::Result<Self> {
        Ok(Self {
            device: Arc::new(device::DeviceService::new(storage.clone())),
            session: Arc::new(session::SessionService::new()),
            audio: Arc::new(audio::AudioService::new()?),
            stt: Arc::new(stt::SttService::new(&config.stt)?),
            tts: Arc::new(tts::TtsService::new(&config.tts)?),
            llm: Arc::new(llm::LlmService::new(&config.grok)?),
        })
    }
}
