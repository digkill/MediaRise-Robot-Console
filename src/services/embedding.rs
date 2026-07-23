//! Эмбеддинги текста для семантического поиска по памяти робота.
//!
//! Использует OpenAI /v1/embeddings (text-embedding-3-small). Векторы
//! хранятся в MySQL как little-endian f32 BLOB; поиск — косинусная близость
//! в памяти сервера (объёмы памяти робота — сотни записей, отдельная
//! векторная БД не нужна).

use anyhow::Context;
use tracing::{info, warn};

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "text-embedding-3-small";

pub struct EmbeddingService {
    client: reqwest::Client,
    api_url: String,
    api_key: Option<String>,
    model: String,
}

impl EmbeddingService {
    pub fn new(config: &crate::config::Config) -> Self {
        // Ключ: EMBEDDING_API_KEY > OpenAI LLM ключ > STT ключ (все OpenAI).
        let api_key = std::env::var("EMBEDDING_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| config.openai_llm.api_key.clone())
            .or_else(|| config.stt.api_key.clone());
        let api_url = std::env::var("EMBEDDING_API_URL")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| OPENAI_API_BASE.to_string());
        let model = std::env::var("EMBEDDING_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        if api_key.is_none() {
            warn!("EmbeddingService: no API key available, semantic memory disabled");
        } else {
            info!(model = %model, "EmbeddingService ready");
        }
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            api_url,
            api_key,
            model,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.api_key.is_some()
    }

    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Embedding API key not configured"))?;
        let endpoint = format!("{}/embeddings", self.api_url.trim_end_matches('/'));

        #[derive(serde::Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
        }
        #[derive(serde::Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingData>,
        }

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&serde_json::json!({ "model": self.model, "input": text }))
            .send()
            .await
            .context("Failed to send embedding request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Embedding API error: {} - {}",
                status,
                body.chars().take(300).collect::<String>()
            );
        }
        let mut body: EmbeddingResponse = response
            .json()
            .await
            .context("Failed to parse embedding response")?;
        body.data
            .pop()
            .map(|d| d.embedding)
            .ok_or_else(|| anyhow::anyhow!("Embedding response contained no data"))
    }
}

/// f32-вектор -> little-endian байты для BLOB.
pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for value in embedding {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// BLOB -> f32-вектор. None, если длина некратна 4 или пусто.
pub fn bytes_to_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Косинусная близость (-1..1); 0 при нулевой норме или разной размерности.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}
