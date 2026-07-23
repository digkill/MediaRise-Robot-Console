//! LLM сервис (Grok или OpenAI)

use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, error, info, warn};

use crate::config::{Config, GrokConfig, OpenAiLlmConfig};

pub struct LlmService {
    provider: LlmProvider,
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<Message>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<Choice>,
    id: Option<String>,
    /// Модель, фактически обработавшая запрос (часто приходит у OpenAI / xAI).
    model: Option<String>,
    /// Счётчики токенов (OpenAI-совместимый API).
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    finish_reason: Option<String>,
}

/// Клиент с разумным таймаутом: без него упавшая сеть/провайдер подвешивает
/// обработку реплики навсегда, и устройство молчит без ошибки.
fn default_llm_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl LlmService {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            provider: LlmProvider::from_config(config)?,
            client: default_llm_client(),
        })
    }

    pub fn new_grok(config: &GrokConfig) -> anyhow::Result<Self> {
        Ok(Self {
            provider: LlmProvider::Grok(config.clone()),
            client: default_llm_client(),
        })
    }

    pub fn new_openai(config: &OpenAiLlmConfig) -> anyhow::Result<Self> {
        Ok(Self {
            provider: LlmProvider::OpenAi(config.clone()),
            client: default_llm_client(),
        })
    }

    /// Создает новый сервис с кастомным HTTP клиентом (для тестирования)
    pub fn new_grok_with_client(config: &GrokConfig, client: reqwest::Client) -> Self {
        Self {
            provider: LlmProvider::Grok(config.clone()),
            client,
        }
    }

    /// Создает новый сервис с кастомным HTTP клиентом (для тестирования)
    pub fn new_openai_with_client(config: &OpenAiLlmConfig, client: reqwest::Client) -> Self {
        Self {
            provider: LlmProvider::OpenAi(config.clone()),
            client,
        }
    }

    pub async fn chat(&self, messages: Vec<ChatMessage>) -> anyhow::Result<String> {
        match &self.provider {
            LlmProvider::Grok(cfg) => self.chat_grok(cfg, messages).await,
            LlmProvider::OpenAi(cfg) => self.chat_openai(cfg, messages).await,
        }
    }

    /// Стриминговый чат: возвращает канал, в который прилетают дельты текста
    /// по мере генерации. Позволяет начинать TTS с первого предложения, не
    /// дожидаясь всего ответа (ключ к «алекса-скорости» диалога).
    pub async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<String>> {
        let (provider_name, api_url, api_key, model, max_tokens, temperature, system_prompt) =
            match &self.provider {
                LlmProvider::Grok(cfg) => (
                    "Grok",
                    cfg.api_url.clone(),
                    cfg.api_key.clone(),
                    cfg.model.clone(),
                    cfg.max_tokens,
                    cfg.temperature,
                    cfg.system_prompt.clone(),
                ),
                LlmProvider::OpenAi(cfg) => (
                    "OpenAI",
                    cfg.api_url.clone(),
                    cfg.api_key.clone().unwrap_or_default(),
                    cfg.model.clone(),
                    cfg.max_tokens,
                    cfg.temperature,
                    cfg.system_prompt.clone(),
                ),
            };
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            anyhow::bail!("{provider_name} API key is not configured");
        }

        let mut payload_messages = Vec::new();
        if let Some(prompt) = system_prompt.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            payload_messages.push(Message {
                role: "system".to_string(),
                content: prompt.to_string(),
            });
        }
        for m in messages {
            payload_messages.push(Message {
                role: m.role,
                content: m.content,
            });
        }

        let request = ChatCompletionsRequest {
            model: model.clone(),
            messages: payload_messages,
            max_tokens,
            temperature,
            stream: true,
        };
        let url = format!("{}/chat/completions", api_url.trim_end_matches('/'));
        info!(provider = provider_name, %url, request_model = %model, "LLM chat/completions POST (stream)");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to send stream request to {provider_name}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "{provider_name} stream API error: {} - {}",
                status,
                body.chars().take(500).collect::<String>()
            );
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut byte_stream = response.bytes_stream();
            let mut pending = String::new();
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("LLM stream chunk error: {}", e);
                        break;
                    }
                };
                pending.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = pending.find('\n') {
                    let line = pending[..pos].trim().to_string();
                    pending.drain(..=pos);
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data == "[DONE]" {
                        return;
                    }
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(delta) = value
                            .pointer("/choices/0/delta/content")
                            .and_then(|d| d.as_str())
                        {
                            if !delta.is_empty() && tx.send(delta.to_string()).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });
        Ok(rx)
    }

    pub async fn describe_robot_view(&self, jpeg: &[u8]) -> anyhow::Result<String> {
        match &self.provider {
            LlmProvider::Grok(cfg) => {
                let model = std::env::var("HOMEBOT_VISION_MODEL")
                    .ok()
                    .filter(|model| !model.trim().is_empty())
                    .unwrap_or_else(|| cfg.model.clone());
                self.describe_image_chat_completions(
                    "Grok",
                    &cfg.api_url,
                    Some(cfg.api_key.as_str()),
                    &model,
                    jpeg,
                )
                .await
            }
            LlmProvider::OpenAi(cfg) => {
                let model = std::env::var("HOMEBOT_VISION_MODEL")
                    .ok()
                    .filter(|model| !model.trim().is_empty())
                    .unwrap_or_else(|| cfg.model.clone());
                self.describe_image_chat_completions(
                    "OpenAI",
                    &cfg.api_url,
                    cfg.api_key.as_deref(),
                    &model,
                    jpeg,
                )
                .await
            }
        }
    }

    async fn describe_image_chat_completions(
        &self,
        provider_name: &str,
        api_url: &str,
        api_key: Option<&str>,
        model: &str,
        jpeg: &[u8],
    ) -> anyhow::Result<String> {
        let api_key = api_key
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| anyhow::anyhow!("{provider_name} API key is not configured"))?;
        let image_url = format!("data:image/jpeg;base64,{}", BASE64.encode(jpeg));
        let request = serde_json::json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": "You analyze the robot cat camera. Describe only clearly visible facts in one short Russian sentence. Mention people, pets, notable objects or possible hazards. Do not guess identity or emotions."
                },
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "Что сейчас видно перед роботом?"},
                        {"type": "image_url", "image_url": {"url": image_url, "detail": "low"}}
                    ]
                }
            ],
            "max_tokens": 120,
            "temperature": 0.1,
            "stream": false
        });
        let url = format!("{}/chat/completions", api_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to send vision request to {provider_name}"))?;
        let status = response.status();
        let body = response.text().await.context("Failed to read vision response")?;
        if !status.is_success() {
            anyhow::bail!("{provider_name} vision API error: {} - {}", status, body);
        }
        let json: Value = serde_json::from_str(&body).context("Failed to parse vision response")?;
        json.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("{provider_name} vision response contained no text"))
    }

    async fn chat_grok(&self, config: &GrokConfig, messages: Vec<ChatMessage>) -> anyhow::Result<String> {
        self.chat_chat_completions(
            "Grok",
            &config.api_url,
            Some(config.api_key.as_str()),
            &config.model,
            config.max_tokens,
            config.temperature,
            config.system_prompt.as_deref(),
            messages,
        )
        .await
    }

    async fn chat_openai(
        &self,
        config: &OpenAiLlmConfig,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<String> {
        let api_key = config
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty());

        if api_key.is_none() {
            anyhow::bail!(
                "OpenAI API key is not configured. Set OPENAI_LLM_API_KEY (or OPENAI_API_KEY / STT_API_KEY / TTS_API_KEY)."
            );
        }

        self.chat_chat_completions(
            "OpenAI",
            &config.api_url,
            api_key,
            &config.model,
            config.max_tokens,
            config.temperature,
            config.system_prompt.as_deref(),
            messages,
        )
        .await
    }

    async fn chat_chat_completions(
        &self,
        provider_name: &str,
        api_url: &str,
        api_key: Option<&str>,
        model: &str,
        max_tokens: u32,
        temperature: f32,
        system_prompt: Option<&str>,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<String> {
        debug!(
            provider = provider_name,
            api_url = %api_url,
            request_model = %model,
            incoming_messages = messages.len(),
            "LLM chat request (incoming)"
        );

        let api_key = api_key
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("{provider_name} API key is not configured"))?;

        let mut payload_messages = Vec::new();
        if let Some(prompt) = system_prompt
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            debug!(provider = provider_name, "LLM: applying system prompt");
            payload_messages.push(Message {
                role: "system".to_string(),
                content: prompt.to_string(),
            });
        }

        for m in messages.into_iter() {
            let preview: String = m.content.chars().take(80).collect();
            debug!(
                provider = provider_name,
                role = %m.role,
                content_preview = %preview,
                content_len = m.content.len(),
                "LLM payload message"
            );
            payload_messages.push(Message {
                role: m.role,
                content: m.content,
            });
        }

        let request = ChatCompletionsRequest {
            model: model.to_string(),
            messages: payload_messages,
            max_tokens,
            temperature,
            stream: false,
        };

        let base = api_url.trim_end_matches('/');
        let url = format!("{}/chat/completions", base);
        info!(
            provider = provider_name,
            %url,
            request_model = %model,
            messages_out = request.messages.len(),
            max_tokens,
            temperature,
            "LLM chat/completions POST"
        );

        let started = Instant::now();
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {provider_name}"))?;

        let status = response.status();
        let req_id = response
            .headers()
            .get("x-request-id")
            .or_else(|| response.headers().get("openai-request-id"))
            .or_else(|| response.headers().get("x-correlation-id"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        debug!(
            provider = provider_name,
            status = %status,
            elapsed_ms = started.elapsed().as_millis(),
            request_id = ?req_id,
            content_type = ?content_type,
            "LLM HTTP response headers"
        );

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            let trimmed = error_text.chars().take(1200).collect::<String>();
            error!(
                provider = provider_name,
                status = %status,
                elapsed_ms = started.elapsed().as_millis(),
                request_id = ?req_id,
                body_prefix = %trimmed,
                "LLM API error response"
            );
            anyhow::bail!("{} API error: {} - {}", provider_name, status, trimmed);
        }

        let is_stream = content_type
            .as_deref()
            .map(|ct| ct.contains("text/event-stream"))
            .unwrap_or(false);

        if is_stream {
            info!(
                provider = provider_name,
                request_id = ?req_id,
                "LLM streaming response (SSE)"
            );
            return Self::read_streaming_response(provider_name, response, started).await;
        }

        let resp: ChatCompletionsResponse = response
            .json()
            .await
            .with_context(|| format!("Failed to parse {provider_name} API response"))?;

        let elapsed_ms = started.elapsed().as_millis();
        let finish_reason = resp
            .choices
            .first()
            .and_then(|c| c.finish_reason.clone());
        let usage_prompt = resp.usage.as_ref().and_then(|u| u.prompt_tokens);
        let usage_completion = resp.usage.as_ref().and_then(|u| u.completion_tokens);
        let usage_total = resp.usage.as_ref().and_then(|u| u.total_tokens);

        let content = resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        if resp.choices.is_empty() {
            warn!(
                provider = provider_name,
                elapsed_ms,
                response_id = ?resp.id,
                response_model = ?resp.model,
                "LLM returned empty choices[]"
            );
        }

        let preview: String = content.chars().take(160).collect();
        info!(
            provider = provider_name,
            elapsed_ms,
            request_id = ?req_id,
            response_id = ?resp.id,
            response_model = ?resp.model,
            request_model = %model,
            choices = resp.choices.len(),
            finish_reason = ?finish_reason,
            prompt_tokens = ?usage_prompt,
            completion_tokens = ?usage_completion,
            total_tokens = ?usage_total,
            assistant_chars = content.len(),
            assistant_preview = %preview,
            "LLM JSON response parsed"
        );
        debug!(provider = provider_name, assistant_full = %content, "LLM assistant message (full)");

        Ok(content)
    }

    async fn read_streaming_response(
        provider_name: &str,
        response: reqwest::Response,
        started: Instant,
    ) -> anyhow::Result<String> {
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut result = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Failed to read LLM SSE byte chunk")?;
            let text =
                std::str::from_utf8(&chunk).context("LLM SSE chunk is not valid UTF-8")?;
            buffer.push_str(text);

            while let Some(idx) = buffer.find('\n') {
                let line = buffer[..idx].trim().to_string();
                buffer.drain(..=idx);
                if line.is_empty() {
                    continue;
                }
                let payload = line
                    .strip_prefix("data:")
                    .map(|s| s.trim())
                    .unwrap_or(line.as_str());

                if payload.is_empty() {
                    continue;
                }

                if payload == "[DONE]" {
                    info!(
                        provider = provider_name,
                        elapsed_ms = started.elapsed().as_millis(),
                        assembled_chars = result.len(),
                        "LLM streaming completed ([DONE])"
                    );
                    debug!(
                        provider = provider_name,
                        assembled = %result,
                        "LLM streaming assembled text"
                    );
                    return Ok(result.trim().to_string());
                }

                if let Some(part) = Self::extract_stream_text(payload)? {
                    result.push_str(&part);
                }
            }
        }

        let leftover = buffer.trim();
        if !leftover.is_empty() && leftover != "[DONE]" {
            if let Some(part) = Self::extract_stream_text(leftover)? {
                result.push_str(&part);
            }
        }

        info!(
            provider = provider_name,
            elapsed_ms = started.elapsed().as_millis(),
            assembled_chars = result.len(),
            "LLM streaming finished (end of stream, no [DONE])"
        );
        debug!(
            provider = provider_name,
            assembled = %result,
            "LLM streaming assembled text"
        );
        Ok(result.trim().to_string())
    }

    fn extract_stream_text(payload: &str) -> anyhow::Result<Option<String>> {
        if payload.is_empty() {
            return Ok(None);
        }

        let value: Value = serde_json::from_str(payload).context("Failed to parse LLM SSE JSON chunk")?;

        if let Some(choices) = value.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                if let Some(text) = Self::extract_text_from_choice(choice) {
                    return Ok(Some(text));
                }
            }
        }

        Ok(None)
    }

    fn extract_text_from_choice(choice: &Value) -> Option<String> {
        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta.get("content") {
                if let Some(text) = Self::flatten_content(content) {
                    return Some(text);
                }
            }
        }

        if let Some(message) = choice.get("message") {
            if let Some(content) = message.get("content") {
                if let Some(text) = Self::flatten_content(content) {
                    return Some(text);
                }
            }
        }

        if let Some(text) = choice.get("text").and_then(|v| v.as_str()) {
            return Some(text.to_string());
        }

        None
    }

    fn flatten_content(value: &Value) -> Option<String> {
        match value {
            Value::String(s) => Some(s.clone()),
            Value::Array(items) => {
                let mut acc = String::new();
                for item in items {
                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                        acc.push_str(text);
                    } else if let Some(text) = item.as_str() {
                        acc.push_str(text);
                    }
                }
                if acc.is_empty() {
                    None
                } else {
                    Some(acc)
                }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
enum LlmProvider {
    Grok(GrokConfig),
    OpenAi(OpenAiLlmConfig),
}

impl LlmProvider {
    fn from_config(config: &Config) -> anyhow::Result<Self> {
        let provider = config.llm_provider.trim().to_lowercase();
        match provider.as_str() {
            "grok" | "xai" => Ok(Self::Grok(config.grok.clone())),
            "openai" => Ok(Self::OpenAi(config.openai_llm.clone())),
            other => anyhow::bail!(
                "Unsupported LLM provider: {other}. Use LLM_PROVIDER=grok or LLM_PROVIDER=openai."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatMessage, LlmService};
    use crate::config::GrokConfig;
    use wiremock::{
        matchers::{body_json, header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    fn create_test_config(api_url: String) -> GrokConfig {
        GrokConfig {
            api_key: "test-api-key".to_string(),
            api_url,
            model: "grok-4".to_string(),
            max_tokens: 2048,
            temperature: 0.7,
            system_prompt: None,
        }
    }

    #[tokio::test]
    async fn test_grok_api_success() {
        // Создаем мок-сервер
        let mock_server = MockServer::start().await;

        let config = create_test_config(mock_server.uri());

        // Настраиваем мок для успешного ответа
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("Authorization", "Bearer test-api-key"))
            .and(header("Content-Type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "Hello! How can I help you today?"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        // Создаем сервис с клиентом, который будет использовать мок-сервер
        let client = reqwest::Client::new();
        let service = LlmService::new_grok_with_client(&config, client);

        // Выполняем запрос
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }];

        let result = service.chat(messages).await;

        // Проверяем результат
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Hello! How can I help you today?");
    }

    #[tokio::test]
    async fn test_grok_api_multiple_messages() {
        let mock_server = MockServer::start().await;
        let config = create_test_config(mock_server.uri());

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "I understand you want to know about Rust."
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let service = LlmService::new_grok_with_client(&config, client);

        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: "What is Rust?".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "Rust is a systems programming language.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Tell me more".to_string(),
            },
        ];

        let result = service.chat(messages).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "I understand you want to know about Rust.");
    }

    #[tokio::test]
    async fn test_grok_api_error_response() {
        let mock_server = MockServer::start().await;
        let config = create_test_config(mock_server.uri());

        // Мокируем ошибку 401 (Unauthorized)
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {
                    "message": "Invalid API key",
                    "type": "invalid_request_error"
                }
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let service = LlmService::new_grok_with_client(&config, client);

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }];

        let result = service.chat(messages).await;

        // Проверяем, что получили ошибку
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Grok API error"));
        assert!(error_msg.contains("401"));
    }

    #[tokio::test]
    async fn test_grok_api_empty_choices() {
        let mock_server = MockServer::start().await;
        let config = create_test_config(mock_server.uri());

        // Мокируем ответ с пустым массивом choices
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": []
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let service = LlmService::new_grok_with_client(&config, client);

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }];

        let result = service.chat(messages).await;

        // Должен вернуться пустой ответ, так как choices пустой
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[tokio::test]
    async fn test_grok_api_request_format() {
        let mock_server = MockServer::start().await;
        let config = create_test_config(mock_server.uri());

        // Проверяем, что запрос содержит правильные поля
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_json(serde_json::json!({
                "model": "grok-4",
                "max_tokens": 2048,
                "temperature": 0.7,
                "stream": false,
                "messages": [{
                    "role": "user",
                    "content": "Test message"
                }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "Response"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let service = LlmService::new_grok_with_client(&config, client);

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Test message".to_string(),
        }];

        let result = service.chat(messages).await;

        assert!(result.is_ok());
    }
}
