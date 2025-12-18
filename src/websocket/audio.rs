//! Обработка аудио в WebSocket

use anyhow::{Context, Result};
use std::collections::VecDeque;
use tracing::{debug, error, info};

use crate::utils::audio::{
    utils::{apply_gain, bytes_to_pcm_samples, calculate_rms, pcm_samples_to_bytes},
    AudioFormat, AudioStreamProcessor, OPUS_FRAME_SIZE, OPUS_SAMPLE_RATE,
};

/// Параметры аудио для обработки
#[derive(Debug, Clone)]
pub struct AudioProcessingParams {
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub channels: u32,
    pub frame_duration_ms: u32,
    pub enable_aec: bool,
    pub gain_db: f32,
}

impl Default for AudioProcessingParams {
    fn default() -> Self {
        Self {
            format: AudioFormat::Opus,
            sample_rate: OPUS_SAMPLE_RATE as u32,
            channels: 1,
            frame_duration_ms: 20,
            enable_aec: false,
            gain_db: 0.0,
        }
    }
}

/// Простой AEC (Acoustic Echo Cancellation) буфер
struct AecBuffer {
    playback_buffer: VecDeque<i16>,
    max_delay_samples: usize,
}

impl AecBuffer {
    fn new(max_delay_ms: u32, sample_rate: u32) -> Self {
        let max_delay_samples = (sample_rate as usize * max_delay_ms as usize) / 1000;
        Self {
            playback_buffer: VecDeque::with_capacity(max_delay_samples * 2),
            max_delay_samples,
        }
    }

    /// Добавляет воспроизводимый аудио сигнал в буфер
    fn add_playback(&mut self, samples: &[i16]) {
        for &sample in samples {
            self.playback_buffer.push_back(sample);
            if self.playback_buffer.len() > self.max_delay_samples {
                self.playback_buffer.pop_front();
            }
        }
    }

    /// Применяет AEC к записанному сигналу
    fn apply_aec(&mut self, recorded: &mut [i16]) {
        if self.playback_buffer.len() < recorded.len() {
            return;
        }

        // Простое вычитание эха (упрощенный алгоритм)
        // В реальности нужен более сложный алгоритм с адаптивной фильтрацией
        for (i, sample) in recorded.iter_mut().enumerate() {
            if let Some(&playback_sample) = self.playback_buffer.get(i) {
                // Вычитаем часть воспроизводимого сигнала (с учетом затухания)
                let echo_reduction = 0.3; // Коэффициент подавления эха
                let echo_component = (playback_sample as f32 * echo_reduction) as i32;
                *sample = (*sample as i32 - echo_component).clamp(i16::MIN as i32, i16::MAX as i32)
                    as i16;
            }
        }
    }
}

/// Обработчик аудио для WebSocket
pub struct AudioProcessor {
    stream_processor: AudioStreamProcessor,
    aec_buffer: Option<AecBuffer>,
    params: AudioProcessingParams,
    input_buffer: Vec<u8>,
    output_buffer: Vec<u8>,
}

impl AudioProcessor {
    /// Создает новый обработчик аудио
    pub fn new(params: AudioProcessingParams) -> Result<Self> {
        let stream_processor =
            AudioStreamProcessor::new().context("Failed to create audio stream processor")?;

        let aec_buffer = if params.enable_aec {
            Some(AecBuffer::new(200, params.sample_rate)) // 200ms delay buffer
        } else {
            None
        };

        info!(
            "Created AudioProcessor: format={:?}, sample_rate={}, channels={}, aec={}",
            params.format, params.sample_rate, params.channels, params.enable_aec
        );

        Ok(Self {
            stream_processor,
            aec_buffer,
            params,
            input_buffer: Vec::new(),
            output_buffer: Vec::new(),
        })
    }

    /// Обрабатывает входящий аудио поток (Opus -> PCM)
    pub fn process_incoming_audio(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        debug!("Processing incoming audio: {} bytes", data.len());

        // Декодируем Opus в PCM
        let pcm_samples = self
            .stream_processor
            .process_opus_packet(data)
            .context("Failed to decode Opus packet")?;

        // Применяем гейн, если задан
        let mut processed_samples = pcm_samples;
        if self.params.gain_db != 0.0 {
            apply_gain(&mut processed_samples, self.params.gain_db);
        }

        // Применяем AEC, если включен
        if let Some(ref mut aec) = self.aec_buffer {
            aec.apply_aec(&mut processed_samples);
        }

        Ok(processed_samples)
    }

    /// Обрабатывает исходящий аудио поток (PCM -> Opus)
    pub fn process_outgoing_audio(&mut self, pcm_data: &[i16]) -> Result<Vec<u8>> {
        debug!("Processing outgoing audio: {} samples", pcm_data.len());

        // Если AEC включен, добавляем воспроизводимый сигнал в буфер
        if let Some(ref mut aec) = self.aec_buffer {
            aec.add_playback(pcm_data);
        }

        // Кодируем PCM в Opus
        let opus_data = self
            .stream_processor
            .encode_to_opus(pcm_data)
            .context("Failed to encode PCM to Opus")?;

        Ok(opus_data)
    }

    /// Обрабатывает аудио данные (автоматическое определение формата)
    pub fn process_audio(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        match self.params.format {
            AudioFormat::Opus => {
                // Входящий Opus -> декодируем в PCM, обрабатываем, кодируем обратно
                let pcm = self.process_incoming_audio(data)?;
                self.process_outgoing_audio(&pcm)
            }
            AudioFormat::Pcm16 => {
                // Входящий PCM -> конвертируем в samples, обрабатываем, кодируем в Opus
                let pcm_samples =
                    bytes_to_pcm_samples(data).context("Failed to convert bytes to PCM samples")?;

                // Применяем обработку
                let mut processed = pcm_samples;
                if self.params.gain_db != 0.0 {
                    apply_gain(&mut processed, self.params.gain_db);
                }

                // Кодируем в Opus для отправки
                self.stream_processor
                    .encode_to_opus(&processed)
                    .context("Failed to encode to Opus")
            }
        }
    }

    /// Обрабатывает поток аудио данных (для накопления неполных пакетов)
    pub fn process_audio_stream(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.input_buffer.extend_from_slice(data);
        let mut results = Vec::new();

        // Обрабатываем полные кадры
        let frame_size = match self.params.format {
            AudioFormat::Opus => {
                // Для Opus размер кадра может варьироваться
                // Обрабатываем каждый пакет отдельно
                // В реальности нужно парсить Opus пакеты по TOC байту
                if self.input_buffer.len() >= 1 {
                    // Минимальный размер Opus пакета - 1 байт
                    // Упрощенная версия: обрабатываем по 1 пакету за раз
                    let packet_len = self.input_buffer[0] as usize;
                    if self.input_buffer.len() >= packet_len + 1 {
                        let packet = self.input_buffer[1..=packet_len].to_vec();
                        self.input_buffer.drain(..=packet_len);

                        match self.process_audio(&packet) {
                            Ok(result) => results.push(result),
                            Err(e) => {
                                error!("Failed to process audio packet: {}", e);
                            }
                        }
                    }
                }
                return Ok(results);
            }
            AudioFormat::Pcm16 => {
                let samples_per_frame = (self.params.sample_rate as usize
                    * self.params.frame_duration_ms as usize)
                    / 1000;
                samples_per_frame * 2 // 2 bytes per sample
            }
        };

        // Для PCM обрабатываем полные кадры
        while self.input_buffer.len() >= frame_size {
            let frame = self.input_buffer[..frame_size].to_vec();
            self.input_buffer.drain(..frame_size);

            match self.process_audio(&frame) {
                Ok(result) => results.push(result),
                Err(e) => {
                    error!("Failed to process audio frame: {}", e);
                }
            }
        }

        Ok(results)
    }

    /// Обновляет параметры обработки
    pub fn update_params(&mut self, params: AudioProcessingParams) -> Result<()> {
        // Пересоздаем AEC буфер, если нужно
        if params.enable_aec != self.params.enable_aec {
            self.aec_buffer = if params.enable_aec {
                Some(AecBuffer::new(200, params.sample_rate))
            } else {
                None
            };
        }

        self.params = params;
        Ok(())
    }

    /// Получает статистику обработки
    pub fn get_stats(&self) -> AudioStats {
        AudioStats {
            input_buffer_size: self.input_buffer.len(),
            output_buffer_size: self.output_buffer.len(),
            aec_enabled: self.aec_buffer.is_some(),
        }
    }

    /// Очищает буферы
    pub fn clear_buffers(&mut self) {
        self.input_buffer.clear();
        self.output_buffer.clear();
    }
}

/// Статистика обработки аудио
#[derive(Debug, Clone)]
pub struct AudioStats {
    pub input_buffer_size: usize,
    pub output_buffer_size: usize,
    pub aec_enabled: bool,
}

/// Создает AudioProcessor с параметрами по умолчанию
impl Default for AudioProcessor {
    fn default() -> Self {
        Self::new(AudioProcessingParams::default())
            .expect("Failed to create default AudioProcessor")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_processor_creation() {
        let params = AudioProcessingParams::default();
        let processor = AudioProcessor::new(params);
        assert!(processor.is_ok());
    }

    #[test]
    fn test_process_opus_audio() {
        let mut params = AudioProcessingParams::default();
        params.format = AudioFormat::Opus;
        let mut processor = AudioProcessor::new(params).unwrap();

        // Создаем тестовый Opus пакет (в реальности это будет валидный Opus)
        // Для теста просто проверим, что метод не падает
        let test_data = vec![0u8; 100];
        let result = processor.process_audio(&test_data);
        // Может быть ошибка декодирования, но не паника
        let _ = result;
    }

    #[test]
    fn test_aec_buffer() {
        let mut aec = AecBuffer::new(200, OPUS_SAMPLE_RATE as u32);

        // Добавляем воспроизводимый сигнал
        let playback: Vec<i16> = (0..1000).map(|i| (i % 1000) as i16).collect();
        aec.add_playback(&playback);

        // Применяем AEC к записанному сигналу
        let mut recorded: Vec<i16> = vec![1000; 500];
        aec.apply_aec(&mut recorded);

        // Проверяем, что сигнал изменился
        assert!(recorded.iter().any(|&s| s != 1000));
    }
}
