//! Управление WebSocket сессиями

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::websocket::protocol::Message;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub device_id: String,
    pub client_id: String,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub protocol_version: u32,
    pub audio_params: AudioParams,
    pub audio_format: Option<String>, // "opus" или "mp3"
}

#[derive(Debug, Clone)]
pub struct AudioParams {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub frame_duration: u32,
}

pub type SessionId = Uuid;
pub type MessageSender = mpsc::UnboundedSender<Message>;

pub struct SessionManager {
    sessions: RwLock<HashMap<SessionId, Session>>,
    senders: RwLock<HashMap<SessionId, MessageSender>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            senders: RwLock::new(HashMap::new()),
        }
    }

    pub async fn create_session(
        &self,
        device_id: String,
        client_id: String,
        protocol_version: u32,
        audio_params: AudioParams,
        audio_format: Option<String>,
    ) -> SessionId {
        let session_id = Uuid::new_v4();
        let session = Session {
            id: session_id,
            device_id,
            client_id,
            created_at: Utc::now(),
            last_activity: Utc::now(),
            protocol_version,
            audio_params,
            audio_format,
        };

        self.sessions.write().await.insert(session_id, session);
        session_id
    }

    pub async fn get_session(&self, session_id: &SessionId) -> Option<Session> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub async fn remove_session(&self, session_id: &SessionId) {
        self.sessions.write().await.remove(session_id);
        self.senders.write().await.remove(session_id);
    }

    pub async fn register_sender(&self, session_id: SessionId, sender: MessageSender) {
        self.senders.write().await.insert(session_id, sender);
    }

    pub async fn send_message(&self, session_id: &SessionId, message: Message) -> bool {
        if let Some(sender) = self.senders.read().await.get(session_id) {
            sender.send(message).is_ok()
        } else {
            false
        }
    }
}
