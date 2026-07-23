//! Конфигурация сервера
//!
//! Этот модуль отвечает за загрузку и хранение всех настроек приложения.
//! Настройки загружаются из переменных окружения или .env файла.
//! Все конфигурационные структуры можно сериализовать в JSON и обратно.

// Импортируем нужные типы и трейты
use serde::{Deserialize, Serialize};  // Трейты для сериализации/десериализации (JSON, YAML и т.д.)
use std::path::PathBuf;  // Тип для работы с путями к файлам (кроссплатформенно)

// Константа - базовый URL для OpenAI API
// Это стандартный адрес, который используется по умолчанию для STT и TTS
const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

/// Главная структура конфигурации приложения
/// 
/// Содержит все настройки для работы сервера:
/// - server: настройки HTTP/WebSocket сервера (адрес, порт)
/// - database: настройки подключения к базе данных
/// - grok: настройки для работы с Grok AI (xAI API)
/// - stt: настройки для распознавания речи (Speech-to-Text)
/// - tts: настройки для синтеза речи (Text-to-Speech)
/// - storage: пути к директориям для хранения файлов
/// - security: секретные ключи для JWT и HMAC
/// - mqtt: опциональные настройки MQTT (только если включена фича)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Настройки HTTP и WebSocket сервера
    pub server: ServerConfig,
    /// Настройки подключения к базе данных (SQLite, PostgreSQL, MySQL)
    pub database: DatabaseConfig,
    /// Выбор LLM провайдера ("grok" или "openai")
    pub llm_provider: String,
    /// Настройки для работы с Grok AI (языковая модель от xAI)
    pub grok: GrokConfig,
    /// Настройки для работы с OpenAI Chat Completions (LLM)
    pub openai_llm: OpenAiLlmConfig,
    /// Настройки для распознавания речи (STT - Speech-to-Text)
    pub stt: SttConfig,
    /// Настройки для синтеза речи (TTS - Text-to-Speech)
    pub tts: TtsConfig,
    /// Настройки путей для хранения файлов (прошивки, ассеты, загрузки)
    pub storage: StorageConfig,
    /// Секретные ключи для безопасности (JWT токены, HMAC подписи)
    pub security: SecurityConfig,
    /// Опциональные настройки MQTT (только если включена фича "mqtt")
    /// Option означает, что это поле может быть None (отсутствовать)
    #[cfg(feature = "mqtt")]
    pub mqtt: Option<MqttConfig>,
}

/// Настройки HTTP и WebSocket сервера
/// 
/// Определяет на каком адресе и порту будет работать сервер.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// IP адрес или hostname для прослушивания
    /// "0.0.0.0" означает слушать на всех сетевых интерфейсах
    /// "127.0.0.1" или "localhost" означает только локальные подключения
    pub host: String,
    /// Порт для HTTP запросов (обычно 8080)
    pub port: u16,
    /// Устаревшее поле: WebSocket в этом сервисе обслуживается на том же порту, что и HTTP (`port`).
    /// Оставлено для совместимости с WEBSOCKET_PORT в .env.
    pub websocket_port: u16,
    /// Публичный hostname/IP для ответа OTA (устройство не может подключиться к 0.0.0.0).
    /// Задаётся через PUBLIC_HOST.
    pub public_host: Option<String>,
    /// Полный базовый URL WebSocket для клиентов, например `ws://90.156.254.46:8080`.
    /// Если задан, имеет приоритет над public_host/host. Переменная: WEBSOCKET_PUBLIC_URL.
    pub websocket_public_url: Option<String>,
}

impl ServerConfig {
    /// URL WebSocket для ответов OTA и логов: путь `/ws`, тот же TCP-порт, что и HTTP.
    pub fn ota_websocket_url(&self) -> String {
        if let Some(ref base) = self.websocket_public_url {
            let b = base.trim().trim_end_matches('/');
            return format!("{}/ws", b);
        }
        let host = self
            .public_host
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .or_else(|| {
                if self.host == "0.0.0.0" || self.host == "::" {
                    None
                } else {
                    Some(self.host.as_str())
                }
            })
            .unwrap_or("127.0.0.1");
        format!("ws://{}:{}/ws", host, self.port)
    }
}

/// Настройки подключения к базе данных
/// 
/// URL базы данных в формате:
/// - SQLite: "sqlite:./database.db"
/// - PostgreSQL: "postgresql://user:password@localhost/dbname"
/// - MySQL: "mysql://user:password@localhost/dbname"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Строка подключения к базе данных (connection string)
    pub url: String,
}

/// Настройки для работы с Grok AI (xAI API)
/// 
/// Grok - это языковая модель от xAI, которая используется для генерации ответов.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokConfig {
    /// API ключ для доступа к xAI API
    /// Получить можно на https://x.ai
    pub api_key: String,
    /// URL API сервера xAI (обычно "https://api.x.ai/v1")
    pub api_url: String,
    /// Название модели Grok (например, "grok-beta", "grok-2", "grok-4")
    pub model: String,
    /// Максимальное количество токенов в ответе
    /// Токен - это примерно одно слово или часть слова
    /// Больше токенов = более длинный ответ, но дороже
    pub max_tokens: u32,
    /// Температура генерации (0.0 - 2.0)
    /// 0.0 = детерминированные ответы (всегда одинаковые)
    /// 1.0 = креативные ответы (разные каждый раз)
    /// 2.0 = очень креативные (может быть бессмыслица)
    pub temperature: f32,
    /// Системный промпт - инструкции для AI о том, как себя вести
    /// Например: "You are a helpful assistant" или "You are Miko, a friendly robot"
    /// Option<String> означает, что это поле может быть None (не задано)
    pub system_prompt: Option<String>,
}

/// Настройки для работы с OpenAI Chat Completions (LLM)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiLlmConfig {
    /// API ключ для OpenAI
    pub api_key: Option<String>,
    /// URL API сервера OpenAI (обычно "https://api.openai.com/v1")
    pub api_url: String,
    /// Название модели (например, "gpt-4o-mini")
    pub model: String,
    /// Максимальное количество токенов в ответе
    pub max_tokens: u32,
    /// Температура генерации
    pub temperature: f32,
    /// Системный промпт
    pub system_prompt: Option<String>,
}

/// Настройки для распознавания речи (STT - Speech-to-Text)
/// 
/// STT преобразует аудио в текст. Например, когда пользователь говорит,
/// STT распознает что он сказал и возвращает текст.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    /// Провайдер STT сервиса (например, "whisper", "openai")
    /// "whisper" и "openai" - это одно и то же (OpenAI Whisper API)
    pub provider: String,
    /// URL API для STT (обычно "https://api.openai.com/v1")
    /// Option означает, что если не задано, будет использован дефолтный
    pub api_url: Option<String>,
    /// API ключ для доступа к STT сервису
    /// Получить можно на https://platform.openai.com
    pub api_key: Option<String>,
    /// Язык распознавания (ISO-639-1, например "ru").
    /// None = автоопределение языка Whisper'ом.
    #[serde(default)]
    pub language: Option<String>,
    /// Контекстная подсказка для Whisper (уменьшает галлюцинации на коротком
    /// или шумном аудио — выдуманные имена, «Субтитры…» и т.п.)
    #[serde(default)]
    pub prompt: Option<String>,
}

/// Формат аудио для TTS ответов
/// 
/// Enum (перечисление) - это тип, который может быть одним из нескольких вариантов.
/// В данном случае аудио может быть либо в формате Opus, либо MP3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AudioFormat {
    /// Opus - современный аудио кодек с хорошим сжатием
    /// Хорошо подходит для голосовых сообщений, меньше размер файла
    /// Но требует декодирования на клиенте (браузер может не поддерживать напрямую)
    #[serde(rename = "opus")]  // При сериализации в JSON будет строка "opus"
    Opus,
    /// MP3 - старый, но широко поддерживаемый формат
    /// Браузеры поддерживают MP3 напрямую, не нужен декодер
    /// Но файлы обычно больше по размеру
    #[serde(rename = "mp3")]  // При сериализации в JSON будет строка "mp3"
    Mp3,
}

/// Настройки для синтеза речи (TTS - Text-to-Speech)
/// 
/// TTS преобразует текст в аудио. Например, когда AI генерирует ответ,
/// TTS превращает этот текст в речь, которую можно проиграть.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    /// Провайдер TTS сервиса (например, "openai")
    pub provider: String,
    /// URL API для TTS (обычно "https://api.openai.com/v1")
    pub api_url: Option<String>,
    /// API ключ для доступа к TTS сервису
    pub api_key: Option<String>,
    /// Модель TTS (например, "tts-1" или "gpt-4o-mini-tts")
    pub model: String,
    /// Голос для синтеза речи
    /// OpenAI поддерживает: "alloy", "echo", "fable", "onyx", "nova", "shimmer"
    /// xAI (provider "grok"/"xai") поддерживает: "eve", "ara", "carina" и другие
    pub voice: String,
    /// Формат аудио для ответов (Opus или MP3)
    pub audio_format: AudioFormat,
    /// Язык синтеза (BCP-47, например "ru"). None = "auto" (для xAI).
    #[serde(default)]
    pub language: Option<String>,
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
    /// Загружает конфигурацию из переменных окружения
    /// 
    /// Этот метод:
    /// 1. Пытается загрузить .env файл (если есть)
    /// 2. Читает переменные окружения
    /// 3. Создает объект Config с настройками по умолчанию
    /// 4. Перезаписывает дефолтные значения значениями из переменных окружения
    /// 
    /// Возвращает Result<Config> - либо успешно загруженную конфигурацию,
    /// либо ошибку (но в данном случае ошибок быть не должно, т.к. есть дефолты)
    pub fn load() -> anyhow::Result<Self> {
        // ============================================
        // ШАГ 1: Загрузка .env файла
        // ============================================
        // .env файл - это текстовый файл с переменными окружения в формате KEY=VALUE
        // Обычно находится в корне проекта и содержит секретные ключи (API ключи и т.д.)
        // dotenv::dotenv() пытается загрузить этот файл и установить переменные окружения
        
        match dotenv::dotenv() {
            // Успешно загружен - выводим путь к файлу
            Ok(path) => {
                tracing::info!("Loaded .env file from: {:?}", path);
            }
            // Файл не найден - это нормально, не критическая ошибка
            // Просто будем использовать переменные окружения, которые уже установлены в системе
            Err(dotenv::Error::Io(_)) => {
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
            cfg.server.websocket_port = port.parse().unwrap_or(cfg.server.port);
        } else {
            // Один listener: HTTP и /ws на одном порту
            cfg.server.websocket_port = cfg.server.port;
        }
        if let Ok(h) = std::env::var("PUBLIC_HOST") {
            let h = h.trim().to_string();
            if !h.is_empty() {
                cfg.server.public_host = Some(h);
            }
        }
        if let Ok(u) = std::env::var("WEBSOCKET_PUBLIC_URL") {
            let u = u.trim().to_string();
            if !u.is_empty() {
                cfg.server.websocket_public_url = Some(u);
            }
        }

        // Database configuration
        if let Ok(url) = std::env::var("DATABASE_URL") {
            cfg.database.url = url;
        }

        // LLM provider switch
        if let Ok(provider) = std::env::var("LLM_PROVIDER") {
            if !provider.trim().is_empty() {
                cfg.llm_provider = provider.trim().to_string();
            }
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

        // OpenAI LLM configuration
        if let Ok(key) = std::env::var("OPENAI_LLM_API_KEY") {
            if !key.trim().is_empty() {
                cfg.openai_llm.api_key = Some(key.trim().to_string());
            }
        } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.trim().is_empty() {
                cfg.openai_llm.api_key = Some(key.trim().to_string());
            }
        }
        if let Ok(url) = std::env::var("OPENAI_LLM_API_URL") {
            if !url.trim().is_empty() {
                cfg.openai_llm.api_url = url.trim().to_string();
            }
        }
        if let Ok(model) = std::env::var("OPENAI_LLM_MODEL") {
            if !model.trim().is_empty() {
                cfg.openai_llm.model = model.trim().to_string();
            }
        }
        if let Ok(tokens) = std::env::var("OPENAI_LLM_MAX_TOKENS") {
            if let Ok(val) = tokens.parse::<u32>() {
                cfg.openai_llm.max_tokens = val;
            }
        }
        if let Ok(temp) = std::env::var("OPENAI_LLM_TEMPERATURE") {
            if let Ok(val) = temp.parse::<f32>() {
                cfg.openai_llm.temperature = val;
            }
        }
        if let Ok(prompt) = std::env::var("OPENAI_LLM_SYSTEM_PROMPT") {
            if !prompt.trim().is_empty() {
                cfg.openai_llm.system_prompt = Some(prompt);
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
        if let Ok(lang) = std::env::var("STT_LANGUAGE") {
            if !lang.trim().is_empty() {
                cfg.stt.language = Some(lang.trim().to_string());
            }
        }
        if let Ok(prompt) = std::env::var("STT_PROMPT") {
            if !prompt.trim().is_empty() {
                cfg.stt.prompt = Some(prompt.trim().to_string());
            }
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
        if let Ok(model) = std::env::var("TTS_MODEL") {
            if !model.trim().is_empty() {
                cfg.tts.model = model.trim().to_string();
            }
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
        if let Ok(lang) = std::env::var("TTS_LANGUAGE") {
            if !lang.trim().is_empty() {
                cfg.tts.language = Some(lang.trim().to_string());
            }
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
        if cfg.openai_llm.api_key.is_none() {
            if let Some(ref key) = cfg.stt.api_key {
                cfg.openai_llm.api_key = Some(key.clone());
            } else if let Some(ref key) = cfg.tts.api_key {
                cfg.openai_llm.api_key = Some(key.clone());
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
                websocket_port: 8080,
                public_host: None,
                websocket_public_url: None,
            },
            database: DatabaseConfig {
                url: "sqlite:xiaozhi.db".to_string(),
            },
            llm_provider: "grok".to_string(),
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
            openai_llm: OpenAiLlmConfig {
                api_key: None,
                api_url: OPENAI_API_BASE.to_string(),
                model: "gpt-4o-mini".to_string(),
                max_tokens: 2048,
                temperature: 0.7,
                system_prompt: Some("You are a helpful AI assistant.".to_string()),
            },
            stt: SttConfig {
                provider: "whisper".to_string(),
                api_url: Some(OPENAI_API_BASE.to_string()),
                api_key: None,
                language: None,
                prompt: None,
            },
            tts: TtsConfig {
                provider: "openai".to_string(),
                api_url: Some(OPENAI_API_BASE.to_string()),
                api_key: None,
                model: "tts-1".to_string(),
                voice: "alloy".to_string(),
                audio_format: AudioFormat::Opus,
                language: None,
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
