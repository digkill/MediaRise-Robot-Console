//! Text-to-Speech сервис

use anyhow::Context;
use tracing::{error, info, instrument};

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

impl TtsService {
    pub fn new(config: &TtsConfig) -> anyhow::Result<Self> {
        Ok(Self {
            config: config.clone(),
            client: reqwest::Client::new(),
        })
    }

    #[instrument(skip_all, fields(chars = text.len(), provider = %self.config.provider, format = ?self.config.audio_format))]
    pub async fn synthesize(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        self.synthesize_with_format(text, None).await
    }

    #[instrument(skip_all, fields(chars = text.len(), provider = %self.config.provider))]
    pub async fn synthesize_with_format(
        &self,
        text: &str,
        format_override: Option<&str>,
    ) -> anyhow::Result<Vec<u8>> {
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
            "local" => {
                anyhow::bail!("Local TTS not implemented yet");
            }
            _ => {
                anyhow::bail!("Unsupported TTS provider: {}", self.config.provider);
            }
        }
    }

    #[instrument(skip_all, fields(chars = text.len()))]
    async fn synthesize_openai(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        self.synthesize_openai_with_format(text, &self.config.audio_format).await
    }

    #[instrument(skip_all, fields(chars = text.len()))]
    async fn synthesize_openai_with_format(
        &self,
        text: &str,
        audio_format: &crate::config::AudioFormat,
    ) -> anyhow::Result<Vec<u8>> {
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
            "model": "tts-1",
            "input": text,
            "voice": self.config.voice,
            "response_format": response_format,
        });

        info!("Sending TTS request to {}", endpoint);
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
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("TTS API error: {} - {}", status, error_text);
        }

        let audio_data = response
            .bytes()
            .await
            .context("Failed to read TTS response")?
            .to_vec();

        if convert_to_opus {
            // Конвертируем PCM в Opus для отправки устройству
            info!("Received TTS audio: {} bytes (PCM), converting to Opus", audio_data.len());
            
            let pcm_samples = utils::bytes_to_pcm_samples(&audio_data)
                .context("Failed to convert PCM bytes to samples")?;

            let mut processor =
                AudioStreamProcessor::new().context("Failed to create audio processor")?;
            let opus_audio = processor
                .encode_to_opus(&pcm_samples)
                .context("Failed to encode PCM to Opus")?;

            info!("Converted to Opus: {} bytes", opus_audio.len());
            Ok(opus_audio)
        } else {
            // Возвращаем MP3 напрямую
            info!("Received TTS audio: {} bytes (MP3), first bytes: {:02x?}", 
                audio_data.len(), 
                &audio_data[..audio_data.len().min(10)]);
            Ok(audio_data)
        }
    }
}
