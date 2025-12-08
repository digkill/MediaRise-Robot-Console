//! Speech-to-Text сервис

use anyhow::Context;
use tracing::{error, info, instrument, warn};

use crate::config::SttConfig;
use crate::utils::audio::utils;

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

        // ============================================
        // КОНВЕРТАЦИЯ PCM В WAV ФОРМАТ
        // ============================================
        // OpenAI Whisper API требует аудио файл с заголовком (WAV, MP3, WebM и т.д.)
        // Сырые PCM байты не принимаются - нужен полноценный WAV файл
        
        // Проверяем, является ли это уже WAV файлом (начинается с "RIFF")
        let is_wav = audio_data.len() >= 4 && &audio_data[0..4] == b"RIFF";
        let is_webm = audio_data.len() >= 4 && &audio_data[0..4] == b"\x1a\x45\xdf\xa3";
        let is_mp3 = audio_data.len() >= 3 && &audio_data[0..3] == b"ID3";
        
        let (audio_file, file_name, mime_type) = if is_wav {
            // Уже WAV файл - используем как есть
            info!("Audio is already in WAV format");
            (audio_data.to_vec(), "audio.wav", "audio/wav")
        } else if is_webm {
            // WebM файл - используем как есть
            info!("Audio is in WebM format");
            (audio_data.to_vec(), "audio.webm", "audio/webm")
        } else if is_mp3 {
            // MP3 файл - используем как есть
            info!("Audio is in MP3 format");
            (audio_data.to_vec(), "audio.mp3", "audio/mpeg")
        } else {
            // Сырые PCM байты - конвертируем в WAV
            // Предполагаем стандартные параметры: 48kHz, моно, 16-bit
            info!("Converting raw PCM to WAV format (assuming 48kHz, mono, 16-bit)");
            
            // Конвертируем байты в PCM samples
            let pcm_samples = utils::bytes_to_pcm_samples(audio_data)
                .context("Failed to convert bytes to PCM samples")?;
            
            // Конвертируем PCM samples в WAV файл
            // Параметры: 48kHz (стандарт для Opus), моно (1 канал), 16-bit
            let wav_data = utils::pcm_to_wav(&pcm_samples, 48000, 1);
            
            info!("Converted PCM to WAV: {} bytes -> {} bytes", audio_data.len(), wav_data.len());
            (wav_data, "audio.wav", "audio/wav")
        };

        // Создаем multipart форму для отправки
        let form = reqwest::multipart::Form::new()
            .text("model", "whisper-1")
            .part(
                "file",
                reqwest::multipart::Part::bytes(audio_file)
                    .file_name(file_name)
                    .mime_str(mime_type)?,
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
