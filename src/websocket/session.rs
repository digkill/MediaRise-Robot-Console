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
    /// Директива поведения от устройства (язык ответа, режим переводчика):
    /// приходит в listen.start c префиксом [[directive]] и попадает в контекст LLM.
    pub reply_directive: Option<String>,
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

/// Буфер для накопления аудио перед отправкой в STT
/// 
/// OpenAI Whisper требует минимум 0.1 секунды аудио (100 мс), но лучше работает с более длинными сегментами.
/// Мы накапливаем кадры до достижения 0.5 секунды для баланса между качеством и задержкой.
/// При 48kHz это 24000 samples (0.5 секунды).
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Накопленные PCM samples
    samples: Vec<i16>,
    /// Частота дискретизации (для расчета длительности)
    sample_rate: u32,
    /// Минимальная длительность для отправки в STT (в секундах)
    min_duration_secs: f32,
    /// Пиковая амплитуда с момента последнего take/clear — грубый признак речи
    peak_amplitude: i16,
    /// Сумма квадратов сэмплов — для среднего RMS буфера (адаптивный порог)
    sum_squares: f64,
}

impl AudioBuffer {
    /// Конец фразы = столько секунд тишины в хвосте буфера.
    const TAIL_SILENCE_SECS: f32 = 0.6;
    /// Нижняя граница порога тишины (int16 PCM). Фактический порог адаптивный:
    /// max(этого, 25% среднего RMS буфера) — иначе на тихом микрофоне речь
    /// принимается за паузу и фразы рубятся на полуторасекундные огрызки.
    const SILENCE_RMS_FLOOR: f32 = 140.0;
    /// Амплитуда выше этого порога — в буфере точно была речь.
    const SPEECH_PEAK: i16 = 900;
    /// Защита: не копим фразу дольше этого.
    const MAX_UTTERANCE_SECS: f32 = 12.0;

    /// Создает новый буфер
    pub fn new(sample_rate: u32) -> Self {
        Self {
            samples: Vec::new(),
            sample_rate,
            // Минимальная длина осмысленной фразы. Раньше здесь было 0.5 с и
            // буфер уходил в STT, как только наполнялся, — предложения резались
            // на полусекундные огрызки. Теперь флаш происходит по КОНЦУ ФРАЗЫ
            // (тишина в хвосте), а это лишь нижняя граница.
            min_duration_secs: 0.8,
            peak_amplitude: 0,
            sum_squares: 0.0,
        }
    }

    /// Добавляет samples в буфер
    pub fn add_samples(&mut self, samples: &[i16]) {
        for &s in samples {
            let a = s.saturating_abs();
            if a > self.peak_amplitude {
                self.peak_amplitude = a;
            }
            self.sum_squares += (s as f64) * (s as f64);
        }
        self.samples.extend_from_slice(samples);
    }

    /// Средний RMS всего буфера
    fn average_rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        ((self.sum_squares / self.samples.len() as f64).sqrt()) as f32
    }

    /// Была ли в буфере речь (а не только шум/тишина)
    pub fn has_speech(&self) -> bool {
        self.peak_amplitude >= Self::SPEECH_PEAK
    }

    /// RMS хвоста буфера длиной secs; MAX, если данных меньше
    fn tail_rms(&self, secs: f32) -> f32 {
        let n = (secs * self.sample_rate as f32) as usize;
        if n == 0 || self.samples.len() < n {
            return f32::MAX;
        }
        let tail = &self.samples[self.samples.len() - n..];
        let sum: f64 = tail.iter().map(|&s| (s as f64) * (s as f64)).sum();
        ((sum / n as f64).sqrt()) as f32
    }

    /// Готов ли буфер к отправке в STT: фраза закончилась (речь была, а в
    /// хвосте — тишина) либо достигнут максимум длительности.
    pub fn is_ready(&self) -> bool {
        let duration_secs = self.duration_secs();
        if duration_secs >= Self::MAX_UTTERANCE_SECS {
            return true;
        }
        if duration_secs < self.min_duration_secs + Self::TAIL_SILENCE_SECS {
            return false;
        }
        // Порог тишины адаптируется к фактической громкости буфера: хвост
        // должен быть ощутимо тише средней громкости речи.
        let threshold = Self::SILENCE_RMS_FLOOR.max(self.average_rms() * 0.25);
        self.has_speech() && self.tail_rms(Self::TAIL_SILENCE_SECS) < threshold
    }

    /// Получает длительность накопленного аудио в секундах
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Получает минимальную требуемую длительность в секундах
    pub fn min_duration_secs(&self) -> f32 {
        self.min_duration_secs
    }

    /// Извлекает и очищает накопленные samples
    pub fn take_samples(&mut self) -> Vec<i16> {
        self.peak_amplitude = 0;
        self.sum_squares = 0.0;
        std::mem::take(&mut self.samples)
    }

    /// Очищает буфер
    pub fn clear(&mut self) {
        self.peak_amplitude = 0;
        self.sum_squares = 0.0;
        self.samples.clear();
    }

    /// Получает количество samples в буфере
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Проверяет, пуст ли буфер
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

pub struct SessionManager {
    sessions: RwLock<HashMap<SessionId, Session>>,
    senders: RwLock<HashMap<SessionId, MessageSender>>,
    /// Буферы для накопления аудио перед отправкой в STT
    /// Ключ - session_id, значение - буфер с накопленными PCM samples
    audio_buffers: RwLock<HashMap<SessionId, AudioBuffer>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            senders: RwLock::new(HashMap::new()),
            audio_buffers: RwLock::new(HashMap::new()),
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
            reply_directive: None,
        };

        self.sessions.write().await.insert(session_id, session);
        session_id
    }

    pub async fn get_session(&self, session_id: &SessionId) -> Option<Session> {
        self.sessions.read().await.get(session_id).cloned()
    }

    /// Обновляет поведенческую директиву сессии (язык ответа и т.п.).
    pub async fn set_reply_directive(&self, session_id: &SessionId, directive: Option<String>) {
        if let Some(session) = self.sessions.write().await.get_mut(session_id) {
            session.reply_directive = directive;
        }
    }

    pub async fn remove_session(&self, session_id: &SessionId) {
        self.sessions.write().await.remove(session_id);
        self.senders.write().await.remove(session_id);
        self.audio_buffers.write().await.remove(session_id);
    }

    /// Добавляет PCM samples в буфер сессии
    /// 
    /// Возвращает true, если буфер готов к отправке в STT (накоплено >= 0.5 секунды)
    pub async fn add_audio_samples(&self, session_id: &SessionId, samples: &[i16], sample_rate: u32) -> bool {
        let mut buffers = self.audio_buffers.write().await;
        
        // Получаем или создаем буфер для этой сессии
        let buffer = buffers.entry(*session_id).or_insert_with(|| AudioBuffer::new(sample_rate));

        // Добавляем samples
        buffer.add_samples(samples);

        // Затянувшаяся тишина без речи — выбрасываем, а не отправляем в STT:
        // Whisper на тишине галлюцинирует («Редактор субтитров…» и т.п.)
        if buffer.duration_secs() > 6.0 && !buffer.has_speech() {
            buffer.clear();
            return false;
        }

        // Проверяем, готов ли буфер к отправке
        buffer.is_ready()
    }

    /// Извлекает накопленные samples из буфера сессии
    /// 
    /// Если force=true, извлекает даже если буфер не готов (для отправки при закрытии)
    /// Возвращает None, если буфер пуст
    pub async fn take_audio_samples(&self, session_id: &SessionId) -> Option<Vec<i16>> {
        self.take_audio_samples_force(session_id, false).await
    }

    /// Извлекает накопленные samples из буфера сессии с возможностью принудительной отправки
    /// 
    /// Параметры:
    /// - session_id: ID сессии
    /// - force: если true, извлекает даже если буфер не готов (для отправки при закрытии)
    /// 
    /// Возвращает None, если буфер пуст
    pub async fn take_audio_samples_force(&self, session_id: &SessionId, force: bool) -> Option<Vec<i16>> {
        let mut buffers = self.audio_buffers.write().await;
        
        if let Some(buffer) = buffers.get_mut(session_id) {
            // Если force=true или буфер готов, извлекаем samples
            if force || buffer.is_ready() {
                let samples = buffer.take_samples();
                if !samples.is_empty() {
                    return Some(samples);
                }
            }
        }
        None
    }

    /// Очищает буфер сессии
    pub async fn clear_audio_buffer(&self, session_id: &SessionId) {
        let mut buffers = self.audio_buffers.write().await;
        if let Some(buffer) = buffers.get_mut(session_id) {
            buffer.clear();
        }
    }

    /// Получает длительность накопленного аудио в секундах
    pub async fn get_audio_buffer_duration(&self, session_id: &SessionId) -> f32 {
        let buffers = self.audio_buffers.read().await;
        if let Some(buffer) = buffers.get(session_id) {
            buffer.duration_secs()
        } else {
            0.0
        }
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
