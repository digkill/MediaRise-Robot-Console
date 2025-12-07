//! Управление сессиями

use std::collections::HashMap;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct SessionService {
    sessions: RwLock<HashMap<Uuid, Session>>,
}

impl SessionService {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    pub async fn create_session(&self, device_id: String) -> Uuid {
        let session_id = Uuid::new_v4();
        let session = Session {
            id: session_id,
            device_id,
            created_at: chrono::Utc::now(),
        };
        self.sessions.write().await.insert(session_id, session);
        session_id
    }

    pub async fn get_session(&self, session_id: &Uuid) -> Option<Session> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub async fn remove_session(&self, session_id: &Uuid) {
        self.sessions.write().await.remove(session_id);
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub device_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
