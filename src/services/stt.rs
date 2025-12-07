//! Speech-to-Text сервис

use anyhow::Context;
use tracing::{error, info, instrument};

use crate::config::SttConfig;

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

fn build_endpoint(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("audio/transcriptions") {
        trimmed.to_string()
    } else {
        format!("{}/audio/transcriptions", trimmed)
    }
}

pub struct SttService {
    config: SttConfig,
    client: reqwest::Client,
}

impl SttService {
    pub fn new(config: &SttConfig) -> anyhow::Result<Self> {
        Ok(Self {
            config: config.clone(),
            client: reqwest::Client::new(),
        })
    }

    #[instrument(skip_all, fields(bytes = audio_data.len(), provider = %self.config.provider))]
    pub async fn transcribe(&self, audio_data: &[u8]) -> anyhow::Result<String> {
        info!(
            "Transcribing audio: {} bytes, provider: {}",
            audio_data.len(),
            self.config.provider
        );

        match self.config.provider.as_str() {
            "whisper" | "openai" => self.transcribe_openai(audio_data).await,
            "local" => {
                // Для локального STT можно использовать другую библиотеку
                anyhow::bail!("Local STT not implemented yet");
            }
            _ => {
                anyhow::bail!("Unsupported STT provider: {}", self.config.provider);
            }
        }
    }

    #[instrument(skip_all, fields(bytes = audio_data.len()))]
    async fn transcribe_openai(&self, audio_data: &[u8]) -> anyhow::Result<String> {
        let api_url = self.config.api_url.as_deref().unwrap_or(OPENAI_API_BASE);
        let endpoint = build_endpoint(api_url);

        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("STT API key not configured"))?;

        info!(
            "Sending audio to OpenAI Whisper API: {} bytes",
            audio_data.len()
        );
        info!("STT endpoint: {}", endpoint);

        // Конвертируем аудио в формат, который понимает OpenAI
        // OpenAI Whisper принимает различные форматы, включая PCM
        let form = reqwest::multipart::Form::new()
            .text("model", "whisper-1")
            .part(
                "file",
                reqwest::multipart::Part::bytes(audio_data.to_vec())
                    .file_name("audio.webm")
                    .mime_str("audio/webm")?,
            );

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .send()
            .await
            .context("Failed to send STT request to OpenAI")?;

        let status = response.status();
        info!("OpenAI STT API response status: {}", status);

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("OpenAI STT API error: {} - {}", status, error_text);
            anyhow::bail!("STT API error: {} - {}", status, error_text);
        }

        let result: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse STT response from OpenAI")?;

        info!("OpenAI STT response: {:?}", result);

        let text = result["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No text in STT response"))?
            .to_string();

        info!("✅ Transcribed text: '{}'", text);
        Ok(text)
    }
}
