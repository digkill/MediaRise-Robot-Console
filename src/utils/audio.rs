//! Аудио утилиты (совместимый API)

use anyhow::{Context, Result};
use audiopus::{coder::Decoder, coder::Encoder, Channels, SampleRate};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

/// Opus всегда 48k
pub const OPUS_SAMPLE_RATE: SampleRate = SampleRate::Hz48000;
pub const OPUS_CHANNELS: Channels = Channels::Mono;

/// Длительность кадра (ms)
pub const OPUS_FRAME_MS: usize = 20;

/// 20ms @48k = 960 samples
pub const OPUS_FRAME_SIZE: usize = (48_000 * OPUS_FRAME_MS) / 1000;

/// Частота “устройства” для PCM (если хочешь 24k)
pub const DEVICE_SAMPLE_RATE: usize = 24_000;
pub const DEVICE_FRAME_SIZE: usize = (DEVICE_SAMPLE_RATE * OPUS_FRAME_MS) / 1000; // 480

/// Формат аудио (вернули как ожидали сервисы)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// PCM 16-bit little-endian
    Pcm16,
    /// Opus encoded (один пакет или length-delimited поток пакетов)
    Opus,
}

/// Параметры sinc ресемплера (rubato 0.16.x не даёт Clone для SincInterpolationParameters)
fn sinc_params() -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 160,
        window: WindowFunction::BlackmanHarris2,
    }
}

/// Конвертер аудио: PCM(24k) <-> Opus(48k)
pub struct AudioConverter {
    encoder: Encoder,
    decoder: Decoder,

    up_24_to_48: SincFixedIn<f32>,
    down_48_to_24: SincFixedIn<f32>,
}

impl AudioConverter {
    pub fn new() -> Result<Self> {
        let encoder = Encoder::new(OPUS_SAMPLE_RATE, OPUS_CHANNELS, audiopus::Application::Voip)
            .context("Failed to create Opus encoder")?;

        let decoder = Decoder::new(OPUS_SAMPLE_RATE, OPUS_CHANNELS)
            .context("Failed to create Opus decoder")?;

        let up_24_to_48 = SincFixedIn::<f32>::new(
            48_000.0 / 24_000.0,
            2.0,
            sinc_params(),
            DEVICE_FRAME_SIZE,
            1,
        )
        .context("Failed to create resampler 24->48")?;

        let down_48_to_24 = SincFixedIn::<f32>::new(
            24_000.0 / 48_000.0,
            2.0,
            sinc_params(),
            OPUS_FRAME_SIZE,
            1,
        )
        .context("Failed to create resampler 48->24")?;

        Ok(Self {
            encoder,
            decoder,
            up_24_to_48,
            down_48_to_24,
        })
    }

    /// PCM(24k) -> Vec<OpusPacket>
    fn encode_pcm_to_opus_packets(&mut self, pcm_24k: &[i16]) -> Result<Vec<Vec<u8>>> {
        let mut packets = Vec::new();

        for chunk in pcm_24k.chunks(DEVICE_FRAME_SIZE) {
            let mut frame_24 = vec![0i16; DEVICE_FRAME_SIZE];
            frame_24[..chunk.len()].copy_from_slice(chunk);

            // i16 -> f32 [-1..1]
            let in_f32: Vec<f32> = frame_24
                .iter()
                .map(|&s| s as f32 / i16::MAX as f32)
                .collect();

            // 24k -> 48k
            let out = self
                .up_24_to_48
                .process(&[in_f32], None)
                .context("Resample 24->48 failed")?;
            let out_48 = &out[0];

            // f32 -> i16
            let mut frame_48 = vec![0i16; out_48.len()];
            for (i, &x) in out_48.iter().enumerate() {
                let v = (x * i16::MAX as f32).round() as i32;
                frame_48[i] = v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            }

            // Opus encode
            let mut output = vec![0u8; 4000];
            let encoded_len = self
                .encoder
                .encode(&frame_48, &mut output)
                .context("Failed to encode Opus frame")?;
            output.truncate(encoded_len);

            packets.push(output);
        }

        Ok(packets)
    }

    /// Opus packet -> PCM(24k)
    fn decode_opus_packet_to_pcm(&mut self, opus_packet: &[u8]) -> Result<Vec<i16>> {
        // запас по размеру
        let mut buffer_48 = vec![0i16; OPUS_FRAME_SIZE * 2];
        let decoded_len = self
            .decoder
            .decode(Some(opus_packet), &mut buffer_48, false)
            .context("Failed to decode Opus packet")?;
        buffer_48.truncate(decoded_len);

        // i16 -> f32
        let in_f32: Vec<f32> = buffer_48
            .iter()
            .map(|&s| s as f32 / i16::MAX as f32)
            .collect();

        // 48k -> 24k
        let out = self
            .down_48_to_24
            .process(&[in_f32], None)
            .context("Resample 48->24 failed")?;
        let out_24 = &out[0];

        // f32 -> i16
        let mut pcm_24 = vec![0i16; out_24.len()];
        for (i, &x) in out_24.iter().enumerate() {
            let v = (x * i16::MAX as f32).round() as i32;
            pcm_24[i] = v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        Ok(pcm_24)
    }
}

impl Default for AudioConverter {
    fn default() -> Self {
        Self::new().expect("Failed to create audio converter")
    }
}

/// Обработчик аудио потоков (как ждут сервисы)
pub struct AudioStreamProcessor {
    converter: AudioConverter,
}

impl AudioStreamProcessor {
    pub fn new() -> Result<Self> {
        Ok(Self {
            converter: AudioConverter::new()?,
        })
    }

    /// Старый контракт: вернуть Vec<u8>
    /// - Для Pcm16: возвращаем length-delimited Opus stream: [u16 len][packet]...
    /// - Для Opus: ожидаем один Opus packet и возвращаем PCM bytes
    pub fn process_stream(&mut self, data: &[u8], format: AudioFormat) -> Result<Vec<u8>> {
        match format {
            AudioFormat::Pcm16 => {
                let samples = utils::bytes_to_pcm_samples(data)?;
                let packets = self.converter.encode_pcm_to_opus_packets(&samples)?;

                let mut out = Vec::new();
                for p in packets {
                    let len = p.len().min(u16::MAX as usize) as u16;
                    out.extend_from_slice(&len.to_le_bytes());
                    out.extend_from_slice(&p[..len as usize]);
                }
                Ok(out)
            }
            AudioFormat::Opus => {
                let pcm = self.converter.decode_opus_packet_to_pcm(data)?;
                Ok(utils::pcm_samples_to_bytes(&pcm))
            }
        }
    }

    /// Opus packet -> PCM(i16)
    pub fn process_opus_packet(&mut self, packet: &[u8]) -> Result<Vec<i16>> {
        self.converter.decode_opus_packet_to_pcm(packet)
    }

    /// PCM(i16, 24k) -> Opus (length-delimited stream as Vec<u8>)
    pub fn encode_to_opus(&mut self, pcm_data: &[i16]) -> Result<Vec<u8>> {
        let packets = self.converter.encode_pcm_to_opus_packets(pcm_data)?;
        let mut out = Vec::new();
        for p in packets {
            let len = p.len().min(u16::MAX as usize) as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&p[..len as usize]);
        }
        Ok(out)
    }
}

impl Default for AudioStreamProcessor {
    fn default() -> Self {
        Self::new()
            .expect("Failed to create audio stream processor")
    }
}

/// Утилиты (как ждут imports crate::utils::audio::utils::...)
pub mod utils {
    use super::*;

    /// bytes (LE i16) -> Vec<i16>
    pub fn bytes_to_pcm_samples(data: &[u8]) -> Result<Vec<i16>> {
        if data.len() % 2 != 0 {
            anyhow::bail!("PCM data must be even number of bytes");
        }
        Ok(data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect())
    }

    /// Vec<i16> -> bytes (LE i16)
    pub fn pcm_samples_to_bytes(samples: &[i16]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        bytes
    }

    /// PCM -> WAV (RIFF) bytes
    pub fn pcm_to_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
        let pcm_data = pcm_samples_to_bytes(samples);
        let data_size = pcm_data.len() as u32;

        // file_size = 36 + data_size (без первых 8 байт RIFF)
        let file_size = 36 + data_size;
        let fmt_size = 16u32;

        let mut wav = Vec::with_capacity(44 + pcm_data.len());

        // RIFF header
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&file_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // fmt chunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&fmt_size.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes()); // ByteRate
        wav.extend_from_slice(&(channels * 2).to_le_bytes()); // BlockAlign
        wav.extend_from_slice(&16u16.to_le_bytes()); // BitsPerSample

        // data chunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend_from_slice(&pcm_data);

        wav
    }

    /// Apply gain in dB
    pub fn apply_gain(samples: &mut [i16], gain_db: f32) {
        let gain_linear = 10.0_f32.powf(gain_db / 20.0);
        for s in samples.iter_mut() {
            let v = (*s as f32 * gain_linear) as i32;
            *s = v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    /// RMS
    pub fn calculate_rms(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum_sq / samples.len() as f64).sqrt() as f32
    }

    /// dBFS (optional)
    pub fn calculate_db_level(samples: &[i16]) -> f32 {
        let rms = calculate_rms(samples);
        if rms <= 0.0 {
            return f32::NEG_INFINITY;
        }
        20.0 * (rms / i16::MAX as f32).log10()
    }
}
