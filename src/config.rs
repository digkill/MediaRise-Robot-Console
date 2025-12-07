//! Конфигурация сервера

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub grok: GrokConfig,
    pub stt: SttConfig,
    pub tts: TtsConfig,
    pub storage: StorageConfig,
    pub security: SecurityConfig,
    #[cfg(feature = "mqtt")]
    pub mqtt: Option<MqttConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub websocket_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokConfig {
    pub api_key: String,
    pub api_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    pub provider: String,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AudioFormat {
    #[serde(rename = "opus")]
    Opus,
    #[serde(rename = "mp3")]
    Mp3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    pub provider: String,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub voice: String,
    pub audio_format: AudioFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub base_path: PathBuf,
    pub firmware_path: PathBuf,
    pub assets_path: PathBuf,
    pub uploads_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub jwt_secret: String,
    pub hmac_key: String,
}

#[cfg(feature = "mqtt")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    pub enabled: bool,
    pub broker: String,
    pub client_id: String,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        // Load from environment variables
        // Пытаемся загрузить .env файл, но если есть ошибки парсинга - загружаем вручную
        match dotenv::dotenv() {
            Ok(path) => {
                tracing::info!("Loaded .env file from: {:?}", path);
            }
            Err(dotenv::Error::Io(_)) => {
                // Файл не найден - это нормально, используем переменные окружения
                tracing::debug!(".env file not found, using environment variables");
            }
            Err(dotenv::Error::LineParse(problem_line, _err)) => {
                // Ошибка парсинга конкретной строки - загружаем файл вручную, пропуская проблемные строки
                tracing::warn!("Failed to parse line in .env file: '{}' (will try to load other variables)", problem_line);
                if let Ok(content) = std::fs::read_to_string(".env") {
                    let mut loaded = 0;
                    for line in content.lines() {
                        let line = line.trim();
                        // Пропускаем пустые строки, комментарии и проблемную строку
                        if line.is_empty() || line.starts_with('#') || line == problem_line {
                            continue;
                        }
                        // Пытаемся распарсить KEY=VALUE
                        if let Some((key, value)) = line.split_once('=') {
                            let key = key.trim();
                            let value = value.trim();
                            // Проверяем, что это валидная переменная (не содержит пробелы в ключе)
                            if !key.is_empty() && !value.is_empty() && !key.contains(' ') {
                                std::env::set_var(key, value);
                                loaded += 1;
                            }
                        }
                    }
                    tracing::info!("Loaded {} environment variables from .env file (skipped problematic lines)", loaded);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load .env file: {} (will use environment variables)", e);
            }
        }

        // For now, use default config with env overrides
        let mut cfg = Self::default();

        // Server configuration
        if let Ok(host) = std::env::var("SERVER_HOST") {
            cfg.server.host = host;
        }
        if let Ok(port) = std::env::var("SERVER_PORT") {
            cfg.server.port = port.parse().unwrap_or(8080);
        }
        if let Ok(port) = std::env::var("WEBSOCKET_PORT") {
            cfg.server.websocket_port = port.parse().unwrap_or(8081);
        }

        // Database configuration
        if let Ok(url) = std::env::var("DATABASE_URL") {
            cfg.database.url = url;
        }

        // Grok/LLM configuration
        if let Ok(key) = std::env::var("GROK_API_KEY") {
            cfg.grok.api_key = key;
        }
        if let Ok(url) = std::env::var("GROK_API_URL") {
            cfg.grok.api_url = url;
        }
        if let Ok(model) = std::env::var("GROK_MODEL") {
            cfg.grok.model = model;
        }
        if let Ok(tokens) = std::env::var("GROK_MAX_TOKENS") {
            if let Ok(val) = tokens.parse::<u32>() {
                cfg.grok.max_tokens = val;
            }
        }
        if let Ok(temp) = std::env::var("GROK_TEMPERATURE") {
            if let Ok(val) = temp.parse::<f32>() {
                cfg.grok.temperature = val;
            }
        }
        if let Ok(prompt) = std::env::var("GROK_SYSTEM_PROMPT") {
            if !prompt.trim().is_empty() {
                cfg.grok.system_prompt = Some(prompt);
            }
        }

        // STT configuration
        if let Ok(provider) = std::env::var("STT_PROVIDER") {
            cfg.stt.provider = provider;
        }
        if let Ok(url) = std::env::var("STT_API_URL") {
            cfg.stt.api_url = Some(url);
        }
        if let Ok(key) = std::env::var("STT_API_KEY") {
            if !key.trim().is_empty() {
                cfg.stt.api_key = Some(key.trim().to_string());
                tracing::debug!("STT API key loaded (length: {})", cfg.stt.api_key.as_ref().unwrap().len());
            } else {
                tracing::warn!("STT_API_KEY is empty");
            }
        } else {
            tracing::warn!("STT_API_KEY environment variable not found");
        }

        // TTS configuration
        if let Ok(provider) = std::env::var("TTS_PROVIDER") {
            cfg.tts.provider = provider;
        }
        if let Ok(url) = std::env::var("TTS_API_URL") {
            cfg.tts.api_url = Some(url);
        }
        if let Ok(key) = std::env::var("TTS_API_KEY") {
            cfg.tts.api_key = Some(key);
        }
        if let Ok(voice) = std::env::var("TTS_VOICE") {
            cfg.tts.voice = voice;
        }
        if let Ok(format_str) = std::env::var("TTS_AUDIO_FORMAT") {
            cfg.tts.audio_format = match format_str.to_lowercase().as_str() {
                "mp3" => AudioFormat::Mp3,
                "opus" | _ => AudioFormat::Opus,
            };
        }

        // Provide sane defaults/fallbacks for STT/OpenAI usage
        if cfg.stt.api_url.is_none() {
            cfg.stt.api_url = Some(OPENAI_API_BASE.to_string());
        }
        if cfg.tts.api_url.is_none() {
            cfg.tts.api_url = Some(OPENAI_API_BASE.to_string());
        }
        if cfg.stt.api_key.is_none() {
            if let Some(ref key) = cfg.tts.api_key {
                cfg.stt.api_key = Some(key.clone());
            }
        }
        if cfg.tts.api_key.is_none() {
            if let Some(ref key) = cfg.stt.api_key {
                cfg.tts.api_key = Some(key.clone());
            }
        }

        // Storage configuration
        if let Ok(path) = std::env::var("STORAGE_BASE_PATH") {
            cfg.storage.base_path = PathBuf::from(path);
            // Update sub-paths relative to base
            cfg.storage.firmware_path = cfg.storage.base_path.join("firmware");
            cfg.storage.assets_path = cfg.storage.base_path.join("assets");
            cfg.storage.uploads_path = cfg.storage.base_path.join("uploads");
        }
        if let Ok(path) = std::env::var("STORAGE_FIRMWARE_PATH") {
            cfg.storage.firmware_path = PathBuf::from(path);
        }
        if let Ok(path) = std::env::var("STORAGE_ASSETS_PATH") {
            cfg.storage.assets_path = PathBuf::from(path);
        }
        if let Ok(path) = std::env::var("STORAGE_UPLOADS_PATH") {
            cfg.storage.uploads_path = PathBuf::from(path);
        }

        // Security configuration
        if let Ok(secret) = std::env::var("JWT_SECRET") {
            cfg.security.jwt_secret = secret;
        }
        if let Ok(key) = std::env::var("HMAC_KEY") {
            cfg.security.hmac_key = key;
        }

        #[cfg(feature = "mqtt")]
        {
            if let Ok(enabled) = std::env::var("MQTT_ENABLED") {
                if enabled.parse::<bool>().unwrap_or(false) {
                    cfg.mqtt = Some(MqttConfig {
                        enabled: true,
                        broker: std::env::var("MQTT_BROKER")
                            .unwrap_or_else(|_| "localhost:1883".to_string()),
                        client_id: std::env::var("MQTT_CLIENT_ID")
                            .unwrap_or_else(|_| "mediarise-robot-console".to_string()),
                    });
                }
            }
        }

        Ok(cfg)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
                websocket_port: 8081,
            },
            database: DatabaseConfig {
                url: "sqlite:xiaozhi.db".to_string(),
            },
            grok: GrokConfig {
                api_key: String::new(),
                api_url: "https://api.x.ai/v1".to_string(),
                model: "grok-4".to_string(),
                max_tokens: 2048,
                temperature: 0.7,
                system_prompt: Some(
                    "You are Grok, a highly intelligent, helpful AI assistant.".to_string(),
                ),
            },
            stt: SttConfig {
                provider: "whisper".to_string(),
                api_url: Some(OPENAI_API_BASE.to_string()),
                api_key: None,
            },
            tts: TtsConfig {
                provider: "openai".to_string(),
                api_url: Some(OPENAI_API_BASE.to_string()),
                api_key: None,
                voice: "alloy".to_string(),
                audio_format: AudioFormat::Opus,
            },
            storage: StorageConfig {
                base_path: PathBuf::from("./storage"),
                firmware_path: PathBuf::from("./storage/firmware"),
                assets_path: PathBuf::from("./storage/assets"),
                uploads_path: PathBuf::from("./storage/uploads"),
            },
            security: SecurityConfig {
                jwt_secret: "change-me".to_string(),
                hmac_key: "change-me".to_string(),
            },
            #[cfg(feature = "mqtt")]
            mqtt: None,
        }
    }
}
