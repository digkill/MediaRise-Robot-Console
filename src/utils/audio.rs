//! Аудио утилиты

use anyhow::{Context, Result};
use audiopus::{coder::Decoder, coder::Encoder, Channels, SampleRate};

/// Параметры аудио для Opus
pub const OPUS_SAMPLE_RATE: SampleRate = SampleRate::Hz48000;
pub const OPUS_CHANNELS: Channels = Channels::Mono;
pub const OPUS_FRAME_SIZE_MS: i32 = 20; // 20ms frames
pub const OPUS_FRAME_SIZE: usize = (OPUS_SAMPLE_RATE as usize * OPUS_FRAME_SIZE_MS as usize) / 1000; // 960 samples

/// Формат аудио
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// PCM 16-bit little-endian
    Pcm16,
    /// Opus encoded
    Opus,
}

/// Конвертер аудио форматов
pub struct AudioConverter {
    encoder: Encoder,
    decoder: Decoder,
}

impl AudioConverter {
    /// Создает новый конвертер аудио
    pub fn new() -> Result<Self> {
        let encoder = Encoder::new(OPUS_SAMPLE_RATE, OPUS_CHANNELS, audiopus::Application::Voip)
            .context("Failed to create Opus encoder")?;

        let decoder = Decoder::new(OPUS_SAMPLE_RATE, OPUS_CHANNELS)
            .context("Failed to create Opus decoder")?;

        Ok(Self { encoder, decoder })
    }

    /// Кодирует PCM аудио в Opus
    pub fn encode_pcm_to_opus(&mut self, pcm_data: &[i16]) -> Result<Vec<u8>> {
        let frame_size = OPUS_FRAME_SIZE;
        let mut encoded = Vec::new();

        // Обрабатываем данные по кадрам
        for chunk in pcm_data.chunks(frame_size) {
            // Дополняем последний чанк нулями, если он неполный
            let mut frame = vec![0i16; frame_size];
            let copy_len = chunk.len().min(frame_size);
            frame[..copy_len].copy_from_slice(&chunk[..copy_len]);

            // Кодируем кадр
            let mut output = vec![0u8; 4000]; // Максимальный размер Opus кадра
            let encoded_len = self
                .encoder
                .encode(&frame, &mut output)
                .context("Failed to encode audio frame")?;

            encoded.extend_from_slice(&output[..encoded_len]);
        }

        Ok(encoded)
    }

    /// Декодирует Opus аудио в PCM
    pub fn decode_opus_to_pcm(&mut self, opus_data: &[u8]) -> Result<Vec<i16>> {
        let frame_size = OPUS_FRAME_SIZE;
        let mut decoded = Vec::new();
        let mut buffer = vec![0i16; frame_size];

        // Обрабатываем данные по кадрам
        // Примечание: для реального использования нужно парсить Opus пакеты
        // Здесь упрощенная версия, которая пытается декодировать весь буфер
        let decoded_len = self
            .decoder
            .decode(Some(opus_data), &mut buffer, false)
            .context("Failed to decode audio frame")?;

        decoded.extend_from_slice(&buffer[..decoded_len]);

        Ok(decoded)
    }

    /// Декодирует Opus пакет в PCM
    pub fn decode_opus_packet(&mut self, opus_packet: &[u8]) -> Result<Vec<i16>> {
        let frame_size = OPUS_FRAME_SIZE;
        let mut buffer = vec![0i16; frame_size];

        let decoded_len = self
            .decoder
            .decode(Some(opus_packet), &mut buffer, false)
            .context("Failed to decode Opus packet")?;

        Ok(buffer[..decoded_len].to_vec())
    }
}

impl Default for AudioConverter {
    fn default() -> Self {
        Self::new().expect("Failed to create audio converter")
    }
}

/// Обработчик аудио потоков
pub struct AudioStreamProcessor {
    converter: AudioConverter,
    buffer: Vec<i16>,
}

impl AudioStreamProcessor {
    /// Создает новый обработчик потоков
    pub fn new() -> Result<Self> {
        Ok(Self {
            converter: AudioConverter::new()?,
            buffer: Vec::new(),
        })
    }

    /// Обрабатывает входящий аудио поток
    pub fn process_stream(&mut self, data: &[u8], format: AudioFormat) -> Result<Vec<u8>> {
        match format {
            AudioFormat::Pcm16 => {
                // Конвертируем байты в i16 samples
                let samples: Vec<i16> = data
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                // Кодируем в Opus
                self.converter.encode_pcm_to_opus(&samples)
            }
            AudioFormat::Opus => {
                // Декодируем Opus в PCM
                let pcm = self.converter.decode_opus_to_pcm(data)?;

                // Конвертируем обратно в байты
                let mut bytes = Vec::with_capacity(pcm.len() * 2);
                for sample in pcm {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
                Ok(bytes)
            }
        }
    }

    /// Обрабатывает Opus пакет и возвращает PCM
    pub fn process_opus_packet(&mut self, packet: &[u8]) -> Result<Vec<i16>> {
        self.converter.decode_opus_packet(packet)
    }

    /// Кодирует PCM данные в Opus
    pub fn encode_to_opus(&mut self, pcm_data: &[i16]) -> Result<Vec<u8>> {
        self.converter.encode_pcm_to_opus(pcm_data)
    }
}

impl Default for AudioStreamProcessor {
    fn default() -> Self {
        Self::new().expect("Failed to create audio stream processor")
    }
}

/// Утилиты для работы с аудио
pub mod utils {
    use super::*;

    /// Конвертирует байты в PCM samples (i16)
    pub fn bytes_to_pcm_samples(data: &[u8]) -> Result<Vec<i16>> {
        if data.len() % 2 != 0 {
            anyhow::bail!("PCM data must be even number of bytes");
        }

        Ok(data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect())
    }

    /// Конвертирует PCM samples (i16) в байты
    pub fn pcm_samples_to_bytes(samples: &[i16]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    /// Нормализует аудио данные (приводит к диапазону -1.0..1.0)
    pub fn normalize_audio(samples: &mut [i16]) {
        let max_amplitude = samples.iter().map(|&s| s.abs() as u16).max().unwrap_or(1) as f32;

        if max_amplitude > 0.0 {
            let scale = (i16::MAX as f32) / max_amplitude;
            for sample in samples.iter_mut() {
                *sample = (*sample as f32 * scale.min(1.0)) as i16;
            }
        }
    }

    /// Применяет гейн к аудио данным
    pub fn apply_gain(samples: &mut [i16], gain_db: f32) {
        let gain_linear = 10.0_f32.powf(gain_db / 20.0);
        for sample in samples.iter_mut() {
            let value = (*sample as f32 * gain_linear) as i32;
            *sample = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    /// Обрезает тишину в начале и конце аудио
    pub fn trim_silence(samples: &[i16], threshold: i16) -> Vec<i16> {
        let start = samples
            .iter()
            .position(|&s| s.abs() > threshold)
            .unwrap_or(0);

        let end = samples
            .iter()
            .rposition(|&s| s.abs() > threshold)
            .map(|pos| pos + 1)
            .unwrap_or(samples.len());

        samples[start..end].to_vec()
    }

    /// Вычисляет RMS (Root Mean Square) для аудио данных
    pub fn calculate_rms(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        let sum_squares: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();

        (sum_squares / samples.len() as f64).sqrt() as f32
    }

    /// Вычисляет уровень громкости в дБ
    pub fn calculate_db_level(samples: &[i16]) -> f32 {
        let rms = calculate_rms(samples);
        if rms <= 0.0 {
            return f32::NEG_INFINITY;
        }
        20.0 * (rms / i16::MAX as f32).log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_converter_creation() {
        let converter = AudioConverter::new();
        assert!(converter.is_ok());
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let mut converter = AudioConverter::new().unwrap();

        // Создаем тестовый PCM сигнал (синусоида)
        let sample_rate = OPUS_SAMPLE_RATE as usize;
        let duration_ms = 100;
        let samples_count = (sample_rate * duration_ms) / 1000;
        let frequency = 440.0; // A4 note

        let mut pcm: Vec<i16> = (0..samples_count)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (f32::sin(2.0 * std::f32::consts::PI * frequency * t) * i16::MAX as f32) as i16
            })
            .collect();

        // Кодируем
        let encoded = converter.encode_pcm_to_opus(&pcm).unwrap();
        assert!(!encoded.is_empty());

        // Декодируем
        let decoded = converter.decode_opus_to_pcm(&encoded).unwrap();
        assert!(!decoded.is_empty());
    }

    #[test]
    fn test_bytes_to_pcm_samples() {
        let bytes = vec![0x00, 0x00, 0xFF, 0x7F, 0x00, 0x80];
        let samples = utils::bytes_to_pcm_samples(&bytes).unwrap();
        assert_eq!(samples, vec![0, 32767, -32768]);
    }

    #[test]
    fn test_pcm_samples_to_bytes() {
        let samples = vec![0, 32767, -32768];
        let bytes = utils::pcm_samples_to_bytes(&samples);
        assert_eq!(bytes.len(), 6);
        assert_eq!(bytes[0..2], [0x00, 0x00]);
    }

    #[test]
    fn test_apply_gain() {
        let mut samples = vec![1000i16, -1000i16];
        utils::apply_gain(&mut samples, 6.0); // +6dB = удвоение амплитуды
        assert!(samples[0].abs() > 1500);
    }

    #[test]
    fn test_calculate_rms() {
        let samples = vec![1000i16, -1000i16, 1000i16, -1000i16];
        let rms = utils::calculate_rms(&samples);
        assert!((rms - 1000.0).abs() < 1.0);
    }
}
