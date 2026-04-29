//! Speech-to-Text сервис

use anyhow::Context;
use std::time::Instant;
use tracing::{debug, error, info, instrument, warn};

use crate::config::SttConfig;
use crate::utils::audio::{utils, OPUS_SAMPLE_RATE};

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

    #[instrument(skip_all, fields(samples = pcm_samples.len(), sample_rate = sample_rate, channels = channels))]
    pub async fn transcribe_pcm(
        &self,
        pcm_samples: &[i16],
        sample_rate: u32,
        channels: u16,
    ) -> anyhow::Result<String> {
        let wav = utils::pcm_to_wav(pcm_samples, sample_rate, channels);
        let pcm_duration = pcm_samples.len() as f64 / sample_rate.max(1) as f64;
        debug!(
            pcm_samples = pcm_samples.len(),
            sample_rate_hz = sample_rate,
            channels = channels,
            pcm_duration_sec = pcm_duration,
            wav_bytes = wav.len(),
            "STT transcribe_pcm: built WAV for Whisper"
        );
        self.transcribe(&wav).await
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

        debug!(
            endpoint = %endpoint,
            api_key_len = api_key.len(),
            input_bytes = audio_data.len(),
            "STT OpenAI: starting HTTP request"
        );

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
            info!(
                wav_bytes = audio_data.len(),
                "STT input format: WAV (RIFF)"
            );
            (audio_data.to_vec(), "audio.wav", "audio/wav")
        } else if is_webm {
            // WebM файл - используем как есть
            info!(webm_bytes = audio_data.len(), "STT input format: WebM");
            (audio_data.to_vec(), "audio.webm", "audio/webm")
        } else if is_mp3 {
            // MP3 файл - используем как есть
            info!(mp3_bytes = audio_data.len(), "STT input format: MP3 (ID3)");
            (audio_data.to_vec(), "audio.mp3", "audio/mpeg")
        } else {
            // Сырые PCM байты - конвертируем в WAV
            // Предполагаем параметры Opus-пайплайна сервера (частота задаётся в заголовке WAV).
            let assumed_rate = OPUS_SAMPLE_RATE as u32;
            warn!(
                raw_bytes = audio_data.len(),
                assumed_rate_hz = assumed_rate,
                "STT input: treating payload as raw PCM16 LE (not RIFF/WebM/MP3) — verify client sends real PCM"
            );
            
            // Конвертируем байты в PCM samples
            let pcm_samples = utils::bytes_to_pcm_samples(audio_data)
                .context("Failed to convert bytes to PCM samples")?;
            
            // Конвертируем PCM samples в WAV файл
            let wav_data = utils::pcm_to_wav(&pcm_samples, assumed_rate, 1);
            
            info!(
                raw_pcm_bytes = audio_data.len(),
                wav_bytes = wav_data.len(),
                "converted assumed-PCM to WAV for STT"
            );
            (wav_data, "audio.wav", "audio/wav")
        };

        let upload_len = audio_file.len();

        // Создаем multipart форму для отправки
        let form = reqwest::multipart::Form::new()
            .text("model", "whisper-1")
            .part(
                "file",
                reqwest::multipart::Part::bytes(audio_file)
                    .file_name(file_name)
                    .mime_str(mime_type)?,
            );

        let started = Instant::now();
        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .send()
            .await
            .context("Failed to send STT request to OpenAI")?;

        let status = response.status();
        let req_id = response
            .headers()
            .get("x-request-id")
            .or_else(|| response.headers().get("openai-request-id"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let elapsed_ms = started.elapsed().as_millis();

        info!(
            status = %status,
            elapsed_ms = elapsed_ms,
            request_id = ?req_id,
            file = file_name,
            mime = mime_type,
            upload_bytes = upload_len,
            "OpenAI STT HTTP round-trip done"
        );

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            let trimmed = error_text.chars().take(900).collect::<String>();
            error!(
                status = %status,
                elapsed_ms = elapsed_ms,
                request_id = ?req_id,
                body_prefix = %trimmed,
                "OpenAI STT API error"
            );
            anyhow::bail!("STT API error: {} - {}", status, trimmed);
        }

        let result: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse STT response from OpenAI")?;

        debug!(
            elapsed_ms = elapsed_ms,
            response_keys = ?result.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
            "OpenAI STT JSON body (keys)"
        );

        let text = result["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No text in STT response"))?
            .to_string();

        info!(
            elapsed_ms = elapsed_ms,
            transcript_len = text.len(),
            "STT transcribed successfully"
        );
        debug!(transcript = %text, "STT transcript");
        Ok(text)
    }
}
