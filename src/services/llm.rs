//! LLM сервис (Grok или OpenAI)

use anyhow::Context;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{error, info};

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
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

impl LlmService {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            provider: LlmProvider::from_config(config)?,
            client: reqwest::Client::new(),
        })
    }

    pub fn new_grok(config: &GrokConfig) -> anyhow::Result<Self> {
        Ok(Self {
            provider: LlmProvider::Grok(config.clone()),
            client: reqwest::Client::new(),
        })
    }

    pub fn new_openai(config: &OpenAiLlmConfig) -> anyhow::Result<Self> {
        Ok(Self {
            provider: LlmProvider::OpenAi(config.clone()),
            client: reqwest::Client::new(),
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
        info!("Sending chat request to {} API: {}", provider_name, api_url);
        info!("Model: {}, Incoming messages: {}", model, messages.len());

        let api_key = api_key
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("{provider_name} API key is not configured"))?;

        let mut payload_messages = Vec::new();
        if let Some(prompt) = system_prompt
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            info!("Applying system prompt");
            payload_messages.push(Message {
                role: "system".to_string(),
                content: prompt.to_string(),
            });
        }

        for m in messages.into_iter() {
            let preview: String = m.content.chars().take(50).collect();
            info!("Message: role={}, content={}...", m.role, preview);
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
        info!("POST {}", url);

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
        info!("{} API response status: {}", provider_name, status);

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("{} API error: {} - {}", provider_name, status, error_text);
            anyhow::bail!("{} API error: {} - {}", provider_name, status, error_text);
        }

        let is_stream = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .map(|ct| ct.contains("text/event-stream"))
            .unwrap_or(false);

        if is_stream {
            info!("Processing {} streaming response", provider_name);
            return Self::read_streaming_response(response).await;
        }

        let resp: ChatCompletionsResponse = response
            .json()
            .await
            .with_context(|| format!("Failed to parse {provider_name} API response"))?;

        info!(
            "{} API response: {} choices (id={:?})",
            provider_name,
            resp.choices.len(),
            resp.id
        );

        let content = resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let preview: String = content.chars().take(100).collect();
        info!("Extracted content: '{}'", preview);

        Ok(content)
    }

    async fn read_streaming_response(response: reqwest::Response) -> anyhow::Result<String> {
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut result = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Failed to read Grok stream chunk")?;
            let text =
                std::str::from_utf8(&chunk).context("Grok stream chunk is not valid UTF-8")?;
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
                    info!("Grok streaming completed");
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

        Ok(result.trim().to_string())
    }

    fn extract_stream_text(payload: &str) -> anyhow::Result<Option<String>> {
        if payload.is_empty() {
            return Ok(None);
        }

        let value: Value =
            serde_json::from_str(payload).context("Failed to parse Grok streaming payload")?;

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
