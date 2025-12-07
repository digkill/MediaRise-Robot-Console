//! Обработка аудио

use crate::utils::audio::{AudioFormat, AudioStreamProcessor};

pub struct AudioService {
    // Не храним процессор, так как он не thread-safe
}

impl AudioService {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {})
    }

    pub async fn process_audio_stream(
        &self,
        data: &[u8],
        format: AudioFormat,
    ) -> anyhow::Result<Vec<u8>> {
        // Создаем новый процессор для каждого запроса (так как он не thread-safe)
        let mut processor = AudioStreamProcessor::new()
            .map_err(|e| anyhow::anyhow!("Failed to create audio processor: {}", e))?;
        processor
            .process_stream(data, format)
            .map_err(|e| anyhow::anyhow!("Failed to process audio: {}", e))
    }

    pub async fn process_opus_packet(&self, packet: &[u8]) -> anyhow::Result<Vec<i16>> {
        // Создаем новый процессор для каждого запроса
        let mut processor = AudioStreamProcessor::new()
            .map_err(|e| anyhow::anyhow!("Failed to create audio processor: {}", e))?;
        processor
            .process_opus_packet(packet)
            .map_err(|e| anyhow::anyhow!("Failed to process Opus packet: {}", e))
    }
}
