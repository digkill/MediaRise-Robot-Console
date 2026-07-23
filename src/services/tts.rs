//! Text-to-Speech сервис

use anyhow::Context;
use std::time::Instant;
use tracing::{debug, error, info, instrument};

use crate::config::TtsConfig;
use crate::utils::audio::{utils, AudioStreamProcessor};

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

fn build_endpoint(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("audio/speech") {
        trimmed.to_string()
    } else {
        format!("{}/audio/speech", trimmed)
    }
}

pub struct TtsService {
    config: TtsConfig,
    client: reqwest::Client,
}

#[derive(Debug)]
pub enum SynthesizedAudio {
    /// Набор Opus кадров (каждый кадр = отдельный WebSocket пакет)
    OpusFrames(Vec<Vec<u8>>),
    /// Любой другой бинарный формат (MP3 и т.д.)
    Binary(Vec<u8>),
}

impl SynthesizedAudio {
    pub fn total_bytes(&self) -> usize {
        match self {
            SynthesizedAudio::OpusFrames(frames) => frames.iter().map(|f| f.len()).sum(),
            SynthesizedAudio::Binary(data) => data.len(),
        }
    }
}

impl TtsService {
    pub fn new(config: &TtsConfig) -> anyhow::Result<Self> {
        Ok(Self {
            config: config.clone(),
            client: reqwest::Client::new(),
        })
    }

    #[instrument(skip_all, fields(chars = text.len(), provider = %self.config.provider, format = ?self.config.audio_format))]
    pub async fn synthesize(&self, text: &str) -> anyhow::Result<SynthesizedAudio> {
        self.synthesize_with_format(text, None).await
    }

    #[instrument(skip_all, fields(chars = text.len(), provider = %self.config.provider))]
    pub async fn synthesize_with_format(
        &self,
        text: &str,
        format_override: Option<&str>,
    ) -> anyhow::Result<SynthesizedAudio> {
        let audio_format = format_override
            .and_then(|f| match f.to_lowercase().as_str() {
                "mp3" => Some(crate::config::AudioFormat::Mp3),
                "opus" => Some(crate::config::AudioFormat::Opus),
                _ => None,
            })
            .unwrap_or_else(|| self.config.audio_format.clone());

        info!(
            "Synthesizing speech for text: {} ({} chars), provider: {}, format: {:?}",
            text, text.len(), self.config.provider, audio_format
        );

        match self.config.provider.as_str() {
            "openai" => self.synthesize_openai_with_format(text, &audio_format).await,
            "grok" | "xai" => self.synthesize_xai_with_format(text, &audio_format).await,
            "local" => {
                anyhow::bail!("Local TTS not implemented yet");
            }
            _ => {
                anyhow::bail!("Unsupported TTS provider: {}", self.config.provider);
            }
        }
    }

    /// xAI (Grok) TTS: POST {base}/tts, ответ — JSON с base64-аудио.
    /// Поддерживает голоса (eve, ara, carina, ...) и инлайн speech-теги вида
    /// [pause] [laugh] [sigh] и <whisper>...</whisper> прямо в тексте.
    #[instrument(skip_all, fields(chars = text.len()))]
    async fn synthesize_xai_with_format(
        &self,
        text: &str,
        audio_format: &crate::config::AudioFormat,
    ) -> anyhow::Result<SynthesizedAudio> {
        const XAI_API_BASE: &str = "https://api.x.ai/v1";

        let base = self.config.api_url.as_deref().unwrap_or(XAI_API_BASE);
        let trimmed = base.trim_end_matches('/');
        let endpoint = if trimmed.ends_with("/tts") {
            trimmed.to_string()
        } else {
            format!("{}/tts", trimmed)
        };

        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TTS API key not configured"))?;

        let (codec, convert_to_opus) = match audio_format {
            crate::config::AudioFormat::Opus => ("pcm", true),
            crate::config::AudioFormat::Mp3 => ("mp3", false),
        };

        let language = self.config.language.as_deref().unwrap_or("auto");
        let request_body = serde_json::json!({
            "text": text,
            "language": language,
            "voice_id": self.config.voice,
            "output_format": {
                "codec": codec,
                "sample_rate": 24000,
            },
        });

        info!(
            endpoint = %endpoint,
            voice = %self.config.voice,
            language = %language,
            codec,
            input_chars = text.len(),
            "TTS xAI: POST /tts"
        );

        let started = Instant::now();
        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send xAI TTS request")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            let trimmed_err = error_text.chars().take(1000).collect::<String>();
            error!(
                status = %status,
                elapsed_ms = started.elapsed().as_millis(),
                body_prefix = %trimmed_err,
                "xAI TTS API error"
            );
            anyhow::bail!("xAI TTS API error: {} - {}", status, trimmed_err);
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();

        // xAI отдаёт сырые байты (audio/pcm, audio/mpeg). JSON с base64 приходит
        // только в расширенных режимах (например, with_timestamps) — поддерживаем оба.
        let raw_body = response
            .bytes()
            .await
            .context("Failed to read xAI TTS response")?
            .to_vec();

        let audio_data = if content_type.starts_with("application/json") {
            #[derive(serde::Deserialize)]
            struct XaiTtsResponse {
                audio: String,
            }
            let body: XaiTtsResponse = serde_json::from_slice(&raw_body)
                .context("Failed to parse xAI TTS response JSON")?;
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(body.audio.as_bytes())
                .context("Failed to decode base64 audio from xAI TTS")?
        } else {
            raw_body
        };

        info!(
            elapsed_ms = started.elapsed().as_millis(),
            audio_bytes = audio_data.len(),
            content_type = %content_type,
            convert_to_opus,
            "TTS xAI: audio received"
        );

        if convert_to_opus {
            let pcm_samples = utils::bytes_to_pcm_samples(&audio_data)
                .context("Failed to convert PCM bytes to samples")?;
            let mut processor =
                AudioStreamProcessor::new().context("Failed to create audio processor")?;
            let opus_frames = processor
                .encode_to_opus_frames(&pcm_samples)
                .context("Failed to encode PCM to Opus")?;
            let total_bytes: usize = opus_frames.iter().map(|f| f.len()).sum();
            info!(
                opus_frames = opus_frames.len(),
                opus_payload_bytes = total_bytes,
                "TTS xAI: Opus frames ready for client"
            );
            Ok(SynthesizedAudio::OpusFrames(opus_frames))
        } else {
            Ok(SynthesizedAudio::Binary(audio_data))
        }
    }

    #[instrument(skip_all, fields(chars = text.len()))]
    async fn synthesize_openai(&self, text: &str) -> anyhow::Result<SynthesizedAudio> {
        self.synthesize_openai_with_format(text, &self.config.audio_format).await
    }

    #[instrument(skip_all, fields(chars = text.len()))]
    async fn synthesize_openai_with_format(
        &self,
        text: &str,
        audio_format: &crate::config::AudioFormat,
    ) -> anyhow::Result<SynthesizedAudio> {
        let api_url = self.config.api_url.as_deref().unwrap_or(OPENAI_API_BASE);
        let endpoint = build_endpoint(api_url);

        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TTS API key not configured"))?;

        // Выбираем формат ответа в зависимости от переданного формата
        let (response_format, convert_to_opus) = match audio_format {
            crate::config::AudioFormat::Opus => ("pcm", true),  // Получаем PCM и конвертируем в Opus
            crate::config::AudioFormat::Mp3 => ("mp3", false),   // Получаем MP3 напрямую
        };

        let request_body = serde_json::json!({
            "model": self.config.model,
            "input": text,
            "voice": self.config.voice,
            "response_format": response_format,
        });

        info!(
            endpoint = %endpoint,
            model = %self.config.model,
            voice = %self.config.voice,
            response_format,
            input_chars = text.len(),
            "TTS OpenAI: POST audio/speech"
        );

        let started = Instant::now();
        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send TTS request")?;

        let status = response.status();
        let req_id = response
            .headers()
            .get("x-request-id")
            .or_else(|| response.headers().get("openai-request-id"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        debug!(
            status = %status,
            elapsed_ms = started.elapsed().as_millis(),
            request_id = ?req_id,
            content_type = ?content_type,
            "TTS HTTP response headers"
        );

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            let trimmed = error_text.chars().take(1000).collect::<String>();
            error!(
                status = %status,
                elapsed_ms = started.elapsed().as_millis(),
                request_id = ?req_id,
                body_prefix = %trimmed,
                "TTS API error"
            );
            anyhow::bail!("TTS API error: {} - {}", status, trimmed);
        }

        let audio_data = response
            .bytes()
            .await
            .context("Failed to read TTS response")?
            .to_vec();

        let elapsed_ms = started.elapsed().as_millis();
        info!(
            elapsed_ms,
            request_id = ?req_id,
            content_type = ?content_type,
            audio_bytes = audio_data.len(),
            convert_to_opus = convert_to_opus,
            "TTS OpenAI: audio body received"
        );

        if convert_to_opus {
            // Конвертируем PCM в Opus для отправки устройству
            debug!(
                pcm_bytes = audio_data.len(),
                "TTS: PCM from OpenAI, encoding to Opus frames"
            );

            let pcm_samples = utils::bytes_to_pcm_samples(&audio_data)
                .context("Failed to convert PCM bytes to samples")?;

            let mut processor =
                AudioStreamProcessor::new().context("Failed to create audio processor")?;
            let opus_frames = processor
                .encode_to_opus_frames(&pcm_samples)
                .context("Failed to encode PCM to Opus")?;
            let total_bytes: usize = opus_frames.iter().map(|f| f.len()).sum();

            info!(
                opus_frames = opus_frames.len(),
                opus_payload_bytes = total_bytes,
                "TTS: Opus frames ready for client"
            );
            Ok(SynthesizedAudio::OpusFrames(opus_frames))
        } else {
            // Возвращаем MP3 напрямую
            let preview_len = audio_data.len().min(10);
            debug!(
                mp3_bytes = audio_data.len(),
                first_bytes = ?&audio_data[..preview_len],
                "TTS: MP3 from OpenAI"
            );
            Ok(SynthesizedAudio::Binary(audio_data))
        }
    }
}
