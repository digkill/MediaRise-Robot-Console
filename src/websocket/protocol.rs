//! WebSocket протокол

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    #[serde(rename = "hello")]
    Hello(HelloMessage),
    #[serde(rename = "listen")]
    Listen(ListenMessage),
    #[serde(rename = "stt")]
    Stt(SttMessage),
    #[serde(rename = "tts")]
    Tts(TtsMessage),
    #[serde(rename = "llm")]
    Llm(LlmMessage),
    #[serde(rename = "mcp")]
    Mcp(McpMessage),
    #[serde(rename = "system")]
    System(SystemMessage),
    #[serde(rename = "abort")]
    Abort(AbortMessage),
    #[serde(rename = "goodbye")]
    Goodbye(GoodbyeMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloMessage {
    pub version: Option<u32>,
    pub transport: Option<String>,
    pub features: Option<Features>,
    pub audio_params: Option<AudioParams>,
    pub session_id: Option<String>,
    #[serde(rename = "audio_format")]
    pub audio_format: Option<String>, // "opus" или "mp3"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Features {
    pub aec: Option<bool>,
    pub mcp: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioParams {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub frame_duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenMessage {
    pub session_id: String,
    pub state: String,        // "start", "stop", "detect"
    pub mode: Option<String>, // "manual", "auto", "realtime"
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttMessage {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsMessage {
    pub session_id: String,
    pub state: String, // "start", "stop", "sentence_start"
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub session_id: String,
    pub emotion: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpMessage {
    pub session_id: String,
    pub payload: serde_json::Value, // JSON-RPC 2.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMessage {
    pub session_id: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbortMessage {
    pub session_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoodbyeMessage {
    pub session_id: String,
}
