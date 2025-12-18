use anyhow::{Context, Result};
use audiopus::{coder::Decoder, coder::Encoder, Channels, SampleRate};
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

pub const OPUS_SAMPLE_RATE: SampleRate = SampleRate::Hz48000;
pub const OPUS_CHANNELS: Channels = Channels::Mono;
pub const OPUS_FRAME_MS: usize = 20;

// Device (твой ESP) хочет 24000 Hz
pub const DEVICE_SAMPLE_RATE: usize = 24_000;

// 20ms frames:
pub const DEVICE_FRAME_SAMPLES: usize = DEVICE_SAMPLE_RATE * OPUS_FRAME_MS / 1000; // 480
pub const OPUS_FRAME_SAMPLES: usize = 48_000 * OPUS_FRAME_MS / 1000;              // 960

pub struct AudioConverter {
    encoder: Encoder,
    decoder: Decoder,

    // 24k -> 48k
    up_24_to_48: SincFixedIn<f32>,
    // 48k -> 24k
    down_48_to_24: SincFixedIn<f32>,
}

impl AudioConverter {
    pub fn new() -> Result<Self> {
        let encoder = Encoder::new(OPUS_SAMPLE_RATE, OPUS_CHANNELS, audiopus::Application::Voip)
            .context("Failed to create Opus encoder")?;
        let decoder = Decoder::new(OPUS_SAMPLE_RATE, OPUS_CHANNELS)
            .context("Failed to create Opus decoder")?;

        // параметры sinc ресемплера (баланс качество/CPU)
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 160,
            window: WindowFunction::BlackmanHarris2,
        };

        // Для mono: 1 канал
        let up_24_to_48 = SincFixedIn::<f32>::new(
            48_000 as f64 / 24_000 as f64,
            2.0,                // max ratio change
            params,
            DEVICE_FRAME_SAMPLES, // input chunk size @24k (480)
            1,
        ).context("Failed to create resampler 24->48")?;

        let down_48_to_24 = SincFixedIn::<f32>::new(
            24_000 as f64 / 48_000 as f64,
            2.0,
            params,
            OPUS_FRAME_SAMPLES, // input chunk size @48k (960)
            1,
        ).context("Failed to create resampler 48->24")?;

        Ok(Self { encoder, decoder, up_24_to_48, down_48_to_24 })
    }

    /// PCM(24k) -> Opus(48k)
    pub fn encode_pcm24k_to_opus(&mut self, pcm_24k: &[i16]) -> Result<Vec<Vec<u8>>> {
        // ВАЖНО: возвращаем Vec пакетов (каждый кадр отдельно!)
        let mut packets = Vec::new();

        for chunk in pcm_24k.chunks(DEVICE_FRAME_SAMPLES) {
            // дополним до ровно 20ms
            let mut frame_24 = vec![0i16; DEVICE_FRAME_SAMPLES];
            frame_24[..chunk.len()].copy_from_slice(chunk);

            // i16 -> f32
            let in_f32: Vec<f32> = frame_24.iter().map(|&s| s as f32 / i16::MAX as f32).collect();

            // resample 24k -> 48k
            let out = self.up_24_to_48.process(&[in_f32], None)
                .context("Resample 24->48 failed")?;
            let out_48 = &out[0];
            // ожидаем ~960
            // f32 -> i16
            let mut frame_48_i16 = vec![0i16; out_48.len()];
            for (i, &x) in out_48.iter().enumerate() {
                let v = (x * i16::MAX as f32).round() as i32;
                frame_48_i16[i] = v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            }

            // Encode one Opus packet
            let mut output = vec![0u8; 4000];
            let len = self.encoder.encode(&frame_48_i16, &mut output)
                .context("Failed to encode Opus frame")?;
            output.truncate(len);
            packets.push(output);
        }

        Ok(packets)
    }

    /// Opus packet(48k) -> PCM(24k, 20ms)
    pub fn decode_opus_packet_to_pcm24k(&mut self, opus_packet: &[u8]) -> Result<Vec<i16>> {
        let mut buffer_48 = vec![0i16; OPUS_FRAME_SAMPLES * 2]; // запас
        let decoded_len = self.decoder.decode(Some(opus_packet), &mut buffer_48, false)
            .context("Failed to decode Opus packet")?;
        buffer_48.truncate(decoded_len);

        // i16 -> f32
        let in_f32: Vec<f32> = buffer_48.iter().map(|&s| s as f32 / i16::MAX as f32).collect();

        // resample 48k -> 24k
        let out = self.down_48_to_24.process(&[in_f32], None)
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
