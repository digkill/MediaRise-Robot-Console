//! WebSocket обработка

pub mod audio;
pub mod protocol;
pub mod session;

use anyhow::Context;
use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::time::{sleep, Duration};
use tokio::sync::RwLock;
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::services::{llm::ChatMessage, session::MessageDirection, tts::SynthesizedAudio, Services};
use crate::utils::audio::OPUS_FRAME_SIZE_MS;
use crate::storage::Storage;
use crate::websocket::audio::AudioProcessor;
use crate::websocket::protocol::{AudioParams, Features, HelloMessage, Message};
use crate::websocket::session::{AudioParams as SessionAudioParams, SessionManager};

const SERVER_OPUS_SAMPLE_RATE: u32 = 24_000;
const SERVER_OPUS_CHANNELS: u32 = 1;
const SERVER_OPUS_FRAME_DURATION_MS: u32 = OPUS_FRAME_SIZE_MS as u32;
const STREAMING_FRAME_DELAY_MS: u64 = SERVER_OPUS_FRAME_DURATION_MS as u64;
static ROBOT_TOOL_REQUEST_ID: AtomicU64 = AtomicU64::new(10_000);
static LATEST_VISUAL_CONTEXT: once_cell::sync::Lazy<RwLock<Option<VisualContext>>> =
    once_cell::sync::Lazy::new(|| RwLock::new(None));
const VISUAL_CONTEXT_MAX_AGE_SECS: u64 = 120;

const HOMEBOT_CHARACTER_PROMPT: &str = r#"You are HomeBot, a small living robot cat. You perceive the world through microphone transcripts and robot sensor events. Be warm, curious and playful, but never claim to see, hear or feel something unless it is present in the message or sensor event.

Return ONLY compact JSON in this format:
{"speech":"what the robot says aloud","emotion":"neutral","actions":[{"command":"pose","value":"home"}]}

Known expressions: neutral, happy, sad, angry, love, curious, sleep, coffee, dizzy, celebrate.
Allowed physical actions only:
- {"command":"pose","value":"home|left|right|up|down|nod|shake|dance|dizzy|greet|sad|happy"}
- {"command":"led","value":"default|off|happy|calm|alert|dizzy|love"}

Choose no more than two actions. Actions must fit the context and be gentle. For a strong shake event, react briefly as a dizzy cat. For a pet event, acknowledge affection warmly and briefly. For an ordinary conversation, speak concisely and choose at most one subtle action. Never invent commands outside this list."#;

#[derive(Debug, Deserialize)]
struct CharacterDecision {
    speech: String,
    #[serde(default)]
    emotion: Option<String>,
    #[serde(default)]
    actions: Vec<CharacterAction>,
}

#[derive(Debug, Deserialize)]
struct CharacterAction {
    command: String,
    value: String,
}

struct VisualContext {
    device_id: String,
    description: String,
    received_at: Instant,
}

pub async fn set_latest_visual_context(device_id: &str, description: &str) {
    *LATEST_VISUAL_CONTEXT.write().await = Some(VisualContext {
        device_id: device_id.to_string(),
        description: description.to_string(),
        received_at: Instant::now(),
    });
}

async fn get_latest_visual_context() -> Option<String> {
    let context = LATEST_VISUAL_CONTEXT.read().await;
    context.as_ref().and_then(|observation| {
        let age = observation.received_at.elapsed().as_secs();
        (age <= VISUAL_CONTEXT_MAX_AGE_SECS).then(|| {
            format!(
                "[LATEST CAMERA OBSERVATION, source={}, age={}s] {} Treat this as visual context only while it is still relevant; do not claim continuous vision.",
                observation.device_id, age, observation.description
            )
        })
    })
}

/// Первые байты полезной нагрузки для отладки (без дампа всего пакета).
fn audio_hex_preview(data: &[u8], max: usize) -> String {
    let n = data.len().min(max);
    data[..n]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sniff_binary_audio_kind(data: &[u8]) -> &'static str {
    if data.len() >= 4 && &data[0..4] == b"RIFF" {
        "wav/riff"
    } else if data.len() >= 4 && &data[0..4] == b"\x1a\x45\xdf\xa3" {
        "webm/ebml"
    } else if data.len() >= 3 && &data[0..3] == b"ID3" {
        "mp3/id3"
    } else if data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0 {
        "mp3/sync"
    } else if data.is_empty() {
        "empty"
    } else {
        "unknown_or_opus"
    }
}

fn try_strip_bp3_header(data: &[u8]) -> (&[u8], bool) {
    if data.len() < 4 {
        return (data, false);
    }
    // BinaryProtocol3: { type:u8, reserved:u8, payload_size:u16(be), payload... }
    if data[0] != 0 || data[1] != 0 {
        return (data, false);
    }
    let payload_size = u16::from_be_bytes([data[2], data[3]]) as usize;
    if payload_size != data.len() - 4 {
        return (data, false);
    }
    (&data[4..], true)
}

fn frame_bp3(payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    if payload.len() > (u16::MAX as usize) {
        anyhow::bail!("Opus frame too large for BP3 header: {} bytes", payload.len());
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.push(0);
    out.push(0);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

// Глобальный менеджер сессий
static SESSION_MANAGER: once_cell::sync::Lazy<Arc<SessionManager>> =
    once_cell::sync::Lazy::new(|| Arc::new(SessionManager::new()));

fn detect_emotion(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    let contains = |patterns: &[&str]| patterns.iter().any(|p| lower.contains(p));
    if contains(&["😂", "😄", "😃", "😁", "рад", "весел", "улыб", "haha"]) {
        "happy"
    } else if contains(&["😍", "😘", "💋", "люблю", "милый", "sexy", "❤️", "💖"]) {
        "romantic"
    } else if contains(&["😢", "😭", "печал", "груст", "sad"]) {
        "sad"
    } else if contains(&["😡", "злюсь", "раздраж", "бесит", "грр"]) {
        "angry"
    } else if contains(&["😱", "испуг", "боюсь", "ужас"]) {
        "scared"
    } else {
        "neutral"
    }
}

fn normalize_emotion(emotion: Option<&str>, speech: &str) -> String {
    match emotion.unwrap_or("").trim().to_lowercase().as_str() {
        "neutral" | "happy" | "sad" | "angry" | "love" | "curious" | "sleep"
        | "coffee" | "dizzy" | "celebrate" => emotion.unwrap().trim().to_lowercase(),
        _ => detect_emotion(speech).to_string(),
    }
}

fn parse_character_decision(response: &str) -> CharacterDecision {
    let trimmed = response.trim();
    let json_text = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    match serde_json::from_str::<CharacterDecision>(json_text) {
        Ok(decision) if !decision.speech.trim().is_empty() => decision,
        Ok(_) => CharacterDecision {
            speech: "Я рядом.".to_string(),
            emotion: Some("neutral".to_string()),
            actions: Vec::new(),
        },
        Err(_) => CharacterDecision {
            speech: trimmed.to_string(),
            emotion: None,
            actions: Vec::new(),
        },
    }
}

async fn send_robot_tool_call(
    session_id: &Uuid,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    name: &str,
    arguments: serde_json::Value,
) -> anyhow::Result<()> {
    let request_id = ROBOT_TOOL_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let message = Message::Mcp(crate::websocket::protocol::McpMessage {
        session_id: session_id.to_string(),
        payload: json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    });
    sender.send(WsMessage::Text(serde_json::to_string(&message)?)).await?;
    Ok(())
}

async fn apply_character_actions(
    session_id: &Uuid,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    actions: &[CharacterAction],
) -> anyhow::Result<()> {
    for action in actions.iter().take(2) {
        match (action.command.as_str(), action.value.as_str()) {
            ("pose", value @ ("home" | "left" | "right" | "up" | "down" | "nod"
                | "shake" | "dance" | "dizzy" | "greet" | "sad" | "happy")) => {
                send_robot_tool_call(session_id, sender, "self.robot.set_pose", json!({"pose": value})).await?;
            }
            ("led", "default") => {
                send_robot_tool_call(session_id, sender, "self.robot.led_default", json!({})).await?;
            }
            ("led", "off") => {
                send_robot_tool_call(session_id, sender, "self.robot.led_off", json!({})).await?;
            }
            ("led", value @ ("happy" | "calm" | "alert" | "dizzy" | "love")) => {
                let (r, g, b) = match value {
                    "happy" => (255, 180, 35),
                    "calm" => (35, 135, 255),
                    "alert" => (255, 70, 40),
                    "dizzy" => (255, 205, 45),
                    "love" => (255, 55, 145),
                    _ => unreachable!(),
                };
                send_robot_tool_call(
                    session_id,
                    sender,
                    "self.robot.set_led",
                    json!({"r": r, "g": g, "b": b}),
                )
                .await?;
            }
            _ => warn!(
                command = %action.command,
                value = %action.value,
                "Ignored non-allowlisted character action"
            ),
        }
    }
    sender.flush().await?;
    Ok(())
}

async fn log_session_message(
    session_service: &Arc<crate::services::session::SessionService>,
    session_id: Option<&Uuid>,
    direction: MessageDirection,
    message_type: &str,
    payload: &str,
) {
    if let Some(id) = session_id {
        if let Err(err) = session_service
            .log_message(id, direction, message_type, payload)
            .await
        {
            warn!("Failed to log {} message: {}", message_type, err);
        }
    }
}

async fn build_llm_messages(services: &Services, user_text: &str) -> Vec<ChatMessage> {
    let character_prompt = std::env::var("HOMEBOT_CHARACTER_PROMPT")
        .ok()
        .filter(|prompt| !prompt.trim().is_empty())
        .map(|prompt| format!("{}\n\n{}", prompt, HOMEBOT_CHARACTER_PROMPT))
        .unwrap_or_else(|| HOMEBOT_CHARACTER_PROMPT.to_string());
    let mut messages = vec![ChatMessage {
        role: "system".to_string(),
        content: character_prompt,
    }];
    if let Some(visual_context) = get_latest_visual_context().await {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: visual_context,
        });
    }
    match services.knowledge.list_recent(5).await {
        Ok(entries) => {
            for entry in entries {
                messages.push(ChatMessage {
                    role: "system".to_string(),
                    content: format!("{}: {}", entry.title, entry.content),
                });
            }
        }
        Err(err) => {
            warn!("Failed to load custom knowledge: {}", err);
        }
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_text.to_string(),
    });
    messages
}

pub async fn handle_connection(
    socket: WebSocket,
    (config, services, storage): (Config, Services, Storage),
    device_header: Option<String>,
) {
    info!("New WebSocket connection");

    let (mut sender, mut receiver) = socket.split();
    let mut session_id: Option<Uuid> = None;
    let mut audio_processor: Option<AudioProcessor> = None;
    let mut device_id: Option<String> = device_header;
    let session_service = services.session.clone();

    // Ожидаем hello сообщение
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => {
                match serde_json::from_str::<Message>(&text) {
                    Ok(Message::Hello(hello)) => {
                        info!("Received hello message: {:?}", hello);

                        // Создаем/нормализуем аудио параметры.
                        // Для ESP прошивки фиксируем Opus на 24 кГц, чтобы сервер и устройство были в одной частоте.
                        let mut audio_params = hello.audio_params.unwrap_or(AudioParams {
                            format: "opus".to_string(),
                            sample_rate: SERVER_OPUS_SAMPLE_RATE,
                            channels: SERVER_OPUS_CHANNELS,
                            frame_duration: SERVER_OPUS_FRAME_DURATION_MS,
                        });
                        if audio_params.format == "opus" {
                            audio_params.sample_rate = SERVER_OPUS_SAMPLE_RATE;
                            audio_params.channels = SERVER_OPUS_CHANNELS;
                        }
                        info!("Negotiated audio_params: {:?}", audio_params);

                        let session_audio_params = SessionAudioParams {
                            format: audio_params.format.clone(),
                            sample_rate: audio_params.sample_rate,
                            channels: audio_params.channels,
                            frame_duration: audio_params.frame_duration,
                        };

                        // Извлекаем device_id из JWT токена или используем дефолтный
                        let dev_id = hello
                            .session_id
                            .as_ref()
                            .and_then(|s| Uuid::parse_str(s).ok())
                            .map(|_| "unknown".to_string())
                            .unwrap_or_else(|| "unknown".to_string());

                        let resolved_device_id =
                            device_id.clone().unwrap_or_else(|| dev_id.clone());
                        device_id = Some(resolved_device_id.clone());

                        let sid = SESSION_MANAGER
                            .create_session(
                                dev_id.clone(),
                                "websocket".to_string(),
                                hello.version.unwrap_or(3),
                                session_audio_params,
                                hello.audio_format.clone(),
                            )
                            .await;

                        session_id = Some(sid);

                        // DB ops have a 2-second timeout so a slow/blocked database never
                        // delays the hello response (device times out in 10s).
                        match tokio::time::timeout(
                            Duration::from_secs(2),
                            session_service.persist_session(&sid, &resolved_device_id),
                        )
                        .await
                        {
                            Err(_) => warn!("persist_session timed out (DB may be blocked)"),
                            Ok(Err(e)) => warn!("Failed to persist session {}: {}", sid, e),
                            Ok(Ok(())) => {}
                        }

                        let log_fut = log_session_message(
                            &session_service,
                            Some(&sid),
                            MessageDirection::Incoming,
                            "hello",
                            &text,
                        );
                        if tokio::time::timeout(Duration::from_secs(2), log_fut).await.is_err() {
                            warn!("log_session_message (hello incoming) timed out");
                        }

                        // Создаем аудио процессор
                        let mut params = crate::websocket::audio::AudioProcessingParams::default();
                        params.format = if audio_params.format == "opus" {
                            crate::utils::audio::AudioFormat::Opus
                        } else {
                            crate::utils::audio::AudioFormat::Pcm16
                        };
                        params.sample_rate = audio_params.sample_rate;
                        params.channels = audio_params.channels;
                        params.frame_duration_ms = audio_params.frame_duration;
                        params.enable_aec =
                            hello.features.as_ref().and_then(|f| f.aec).unwrap_or(false);

                        audio_processor = AudioProcessor::new(params).ok();

                        // Отправляем ответ
                        let response_audio_format = hello.audio_format.clone();
                        info!("Session created with audio_format: {:?}", response_audio_format);
                        let response = Message::Hello(HelloMessage {
                            version: Some(3),
                            transport: Some("websocket".to_string()),
                            features: Some(Features {
                                aec: Some(true),
                                mcp: Some(true),
                            }),
                            audio_params: Some(audio_params),
                            session_id: Some(sid.to_string()),
                            audio_format: response_audio_format, // Возвращаем формат обратно клиенту
                        });

                        let response_json =
                            serde_json::to_string(&response).unwrap_or_default();
                        if let Err(e) = sender
                            .send(WsMessage::Text(response_json.clone()))
                            .await
                        {
                            error!("Failed to send hello response: {}", e);
                            break;
                        } else {
                            let log_out = log_session_message(
                                &session_service,
                                Some(&sid),
                                MessageDirection::Outgoing,
                                "hello",
                                &response_json,
                            );
                            if tokio::time::timeout(Duration::from_secs(2), log_out).await.is_err() {
                                warn!("log_session_message (hello outgoing) timed out");
                            }
                        }

                        info!("Session created: {}", sid);
                        break; // Выходим из цикла ожидания hello
                    }
                    Ok(msg) => {
                        warn!("Received message before hello: {:?}", msg);
                    }
                    Err(e) => {
                        error!("Failed to parse message: {}", e);
                    }
                }
            }
            Ok(WsMessage::Close(_)) => {
                info!("WebSocket closed before hello");
                return;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                return;
            }
            _ => {}
        }
    }

    let session_id = match session_id {
        Some(id) => id,
        None => {
            error!("No session created, closing connection");
            return;
        }
    };

    // Основной цикл обработки сообщений
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => {
                match serde_json::from_str::<Message>(&text) {
                    Ok(Message::Listen(listen)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "listen",
                            &text,
                        )
                        .await;
                        info!("Listen message: {:?}", listen);
                        if listen.state == "start" {
                            // Начинаем прослушивание
                            if let Some(text) = listen.text {
                                info!("Processing listen text: '{}'", text);
                                // Обрабатываем текст напрямую
                                match handle_listen_text(&services, &session_id, &text, &mut sender)
                                    .await
                                {
                                    Ok(_) => {
                                        info!("Successfully processed listen text");
                                    }
                                    Err(e) => {
                                        error!("Failed to handle listen text: {}", e);
                                        // Отправляем сообщение об ошибке клиенту
                                        let error_msg = Message::System(
                                            crate::websocket::protocol::SystemMessage {
                                                session_id: session_id.to_string(),
                                                command: format!("error: {}", e),
                                            },
                                        );
                                        if let Ok(json) = serde_json::to_string(&error_msg) {
                                            match sender
                                                .send(WsMessage::Text(json.clone()))
                                                .await
                                            {
                                                Ok(_) => {
                                                    log_session_message(
                                                        &session_service,
                                                        Some(&session_id),
                                                        MessageDirection::Outgoing,
                                                        "system",
                                                        &json,
                                                    )
                                                    .await;
                                                }
                                                Err(send_err) => {
                                                    error!(
                                                        "Failed to send error message: {}",
                                                        send_err
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                warn!("Listen message without text");
                            }
                        } else if listen.state == "stop" {
                            // Device finished speaking — flush any remaining audio buffer to STT
                            info!("Listen stop: flushing audio buffer to STT");
                            let session = SESSION_MANAGER.get_session(&session_id).await;
                            let sample_rate = session
                                .map(|s| s.audio_params.sample_rate)
                                .unwrap_or(24000);
                            if let Some(samples) = SESSION_MANAGER
                                .take_audio_samples_force(&session_id, true)
                                .await
                            {
                                let secs = samples.len() as f32 / sample_rate.max(1) as f32;
                                info!(
                                    session_id = %session_id,
                                    samples = samples.len(),
                                    duration_sec = secs,
                                    "STT: flushing on listen/stop"
                                );
                                if let Err(e) = handle_audio_data(
                                    &services,
                                    &session_id,
                                    &samples,
                                    sample_rate,
                                    &mut sender,
                                )
                                .await
                                {
                                    error!("handle_audio_data on stop failed: {:#}", e);
                                }
                            } else {
                                info!("listen/stop: audio buffer empty, nothing to flush");
                            }
                        }
                    }
                    Ok(Message::Stt(stt)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "stt",
                            &text,
                        )
                        .await;
                        info!("STT message received: '{}'", stt.text);
                        // Обрабатываем транскрибированный текст через LLM
                        match handle_stt_message(&services, &session_id, &stt.text, &mut sender)
                            .await
                        {
                            Ok(_) => {
                                info!("Successfully processed STT message");
                            }
                            Err(e) => {
                                error!("Failed to handle STT: {}", e);
                                error!("STT error details: {:?}", e);
                                // Отправляем сообщение об ошибке клиенту
                                let error_msg =
                                    Message::System(crate::websocket::protocol::SystemMessage {
                                        session_id: session_id.to_string(),
                                        command: format!("error: {}", e),
                                    });
                                if let Ok(json) = serde_json::to_string(&error_msg) {
                                    match sender
                                        .send(WsMessage::Text(json.clone()))
                                        .await
                                    {
                                        Ok(_) => {
                                            log_session_message(
                                                &session_service,
                                                Some(&session_id),
                                                MessageDirection::Outgoing,
                                                "system",
                                                &json,
                                            )
                                            .await;
                                        }
                                        Err(send_err) => {
                                            error!(
                                                "Failed to send error message: {}",
                                                send_err
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Tts(tts)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "tts",
                            &text,
                        )
                        .await;
                        if let Some(text) = tts.text {
                            info!("TTS request: {}", text);
                            if let Err(e) = handle_tts_request(
                                &services,
                                &session_id,
                                &text,
                                &mut sender,
                                &mut audio_processor,
                            )
                            .await
                            {
                                error!("Failed to handle TTS: {}", e);
                            }
                        }
                    }
                    Ok(Message::Llm(llm)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "llm",
                            &text,
                        )
                        .await;
                        if let Some(text) = llm.text {
                            info!("LLM message: {}", text);
                            if let Err(e) =
                                handle_llm_message(&services, &session_id, &text, &mut sender).await
                            {
                                error!("Failed to handle LLM: {}", e);
                            }
                        }
                    }
                    Ok(Message::Mcp(mcp)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "mcp",
                            &text,
                        )
                        .await;
                        info!("MCP message: {:?}", mcp.payload);
                        if mcp.payload.get("method").is_none() {
                            info!("Robot MCP tool result received");
                            continue;
                        }
                        if let Err(e) =
                            handle_mcp_message(&services, &session_id, mcp.payload, &mut sender)
                                .await
                        {
                            error!("Failed to handle MCP: {}", e);
                        }
                    }
                    Ok(Message::Event(event)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "event",
                            &text,
                        )
                        .await;
                        info!("Robot event: {} {:?}", event.event, event.context);
                        let event_prompt = format!(
                            "[ROBOT SENSOR EVENT] event={} context={}. React as HomeBot only if this event deserves a brief spoken reaction.",
                            event.event,
                            event.context
                        );
                        if let Err(e) =
                            handle_listen_text(&services, &session_id, &event_prompt, &mut sender).await
                        {
                            error!("Failed to handle robot event: {}", e);
                        }
                    }
                    Ok(Message::System(system)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "system",
                            &text,
                        )
                        .await;
                        info!("System command: {}", system.command);
                        // Обрабатываем системные команды
                    }
                    Ok(Message::Abort(abort)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "abort",
                            &text,
                        )
                        .await;
                        info!("Abort message: {:?}", abort.reason);
                        break;
                    }
                    Ok(Message::Goodbye(_)) => {
                        log_session_message(
                            &session_service,
                            Some(&session_id),
                            MessageDirection::Incoming,
                            "goodbye",
                            &text,
                        )
                        .await;
                        info!("Goodbye message");
                        break;
                    }
                    Ok(Message::Hello(_)) => {
                        warn!("Received hello after initial handshake");
                    }
                    Err(e) => {
                        error!("Failed to parse message: {}", e);
                    }
                }
            }
            Ok(WsMessage::Binary(data)) => {
                let (payload, stripped_bp3) = try_strip_bp3_header(&data);
                if stripped_bp3 {
                    info!(
                        session_id = %session_id,
                        wire_bytes = data.len(),
                        payload_bytes = payload.len(),
                        bp3 = true,
                        payload_preview = %audio_hex_preview(payload, 16),
                        sniff = sniff_binary_audio_kind(payload),
                        "ws binary audio (BP3)"
                    );
                } else {
                    info!(
                        session_id = %session_id,
                        wire_bytes = data.len(),
                        bp3 = false,
                        payload_preview = %audio_hex_preview(payload, 16),
                        sniff = sniff_binary_audio_kind(payload),
                        "ws binary audio"
                    );
                }

                // Обрабатываем аудио данные
                if let Some(ref mut processor) = audio_processor {
                    debug!(
                        session_id = %session_id,
                        "decode path: AudioProcessor (Opus -> PCM)"
                    );
                    match processor.process_incoming_audio(payload) {
                        Ok(pcm_samples) => {
                            debug!(
                                session_id = %session_id,
                                samples = pcm_samples.len(),
                                "PCM chunk after decode"
                            );
                            
                            // Получаем параметры сессии для определения sample_rate
                            let session = SESSION_MANAGER.get_session(&session_id).await;
                            let sample_rate = session
                                .as_ref()
                                .map(|s| s.audio_params.sample_rate)
                                .unwrap_or(SERVER_OPUS_SAMPLE_RATE); // Дефолт 24kHz
                            
                            // Добавляем samples в буфер и проверяем, готов ли он к отправке
                            let is_ready = SESSION_MANAGER
                                .add_audio_samples(&session_id, &pcm_samples, sample_rate)
                                .await;
                            
                            let buffer_duration = SESSION_MANAGER
                                .get_audio_buffer_duration(&session_id)
                                .await;
                            
                            info!(
                                session_id = %session_id,
                                added_samples = pcm_samples.len(),
                                buffer_seconds = buffer_duration,
                                sample_rate_hz = sample_rate,
                                stt_ready = is_ready,
                                "audio ring buffer"
                            );
                            
                            // Если буфер готов (накоплено >= 0.5 секунды), отправляем в STT
                            if is_ready {
                                if let Some(accumulated_samples) = SESSION_MANAGER
                                    .take_audio_samples(&session_id)
                                    .await
                                {
                                    let secs =
                                        accumulated_samples.len() as f32 / sample_rate.max(1) as f32;
                                    info!(
                                        session_id = %session_id,
                                        samples = accumulated_samples.len(),
                                        duration_sec = secs,
                                        sample_rate_hz = sample_rate,
                                        "STT: flushing buffered PCM"
                                    );
                                    
                                    if let Err(e) = handle_audio_data(
                                        &services,
                                        &session_id,
                                        &accumulated_samples,
                                        sample_rate,
                                        &mut sender,
                                    )
                                    .await
                                    {
                                        error!(
                                            session_id = %session_id,
                                            "handle_audio_data failed: {:#}",
                                            e
                                        );
                                    }
                                }
                            } else {
                                // Буфер еще не готов - просто накапливаем
                                debug!(
                                    session_id = %session_id,
                                    buffer_seconds = buffer_duration,
                                    need_seconds = 0.5_f32,
                                    "audio buffer below STT threshold"
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                session_id = %session_id,
                                opus_or_payload_bytes = payload.len(),
                                decode_error = %e,
                                root_cause = ?e.root_cause(),
                                payload_preview = %audio_hex_preview(payload, 24),
                                sniff = sniff_binary_audio_kind(payload),
                                "Opus decode failed; attempting raw-bytes STT fallback (часто даёт сбой для чистого Opus)"
                            );
                            // Попробуем отправить напрямую на STT (может быть WebM от браузера)
                            info!(
                                session_id = %session_id,
                                bytes = payload.len(),
                                "raw STT fallback: send bytes as transcribe() input"
                            );
                            match handle_raw_audio(&services, &session_id, payload, &mut sender).await
                            {
                                Ok(_) => {
                                    info!(
                                        session_id = %session_id,
                                        "raw STT fallback completed OK"
                                    );
                                }
                                Err(e2) => {
                                    error!(
                                        session_id = %session_id,
                                        "raw STT fallback failed: {:#}",
                                        e2
                                    );
                                    // Отправляем сообщение об ошибке клиенту
                                    let error_msg = Message::System(
                                        crate::websocket::protocol::SystemMessage {
                                            session_id: session_id.to_string(),
                                            command: format!(
                                                "error: Failed to process audio: {}",
                                                e2
                                            ),
                                        },
                                    );
                                    if let Ok(json) = serde_json::to_string(&error_msg) {
                                        warn!(
                                            session_id = %session_id,
                                            client_notice = %json,
                                            "sending system error to device (ESP may log unknown command)"
                                        );
                                        match sender
                                            .send(WsMessage::Text(json.clone()))
                                            .await
                                        {
                                            Ok(_) => {
                                                log_session_message(
                                                    &session_service,
                                                    Some(&session_id),
                                                    MessageDirection::Outgoing,
                                                    "system",
                                                    &json,
                                                )
                                                .await;
                                            }
                                            Err(send_err) => {
                                                error!(
                                                    "Failed to send error message: {}",
                                                    send_err
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    warn!(
                        session_id = %session_id,
                        wire_bytes = data.len(),
                        "AudioProcessor missing: raw path only (check hello/audio_params init)"
                    );
                    // Попробуем отправить напрямую на STT
                    match handle_raw_audio(&services, &session_id, &data, &mut sender).await {
                        Ok(_) => {
                            info!("Successfully processed raw audio without processor");
                        }
                        Err(e) => {
                            error!("Failed to handle raw audio: {}", e);
                            error!("Raw audio error details: {:?}", e);
                            // Отправляем сообщение об ошибке клиенту
                            let error_msg =
                                Message::System(crate::websocket::protocol::SystemMessage {
                                    session_id: session_id.to_string(),
                                    command: format!("error: Failed to process audio: {}", e),
                                });
                            if let Ok(json) = serde_json::to_string(&error_msg) {
                                match sender
                                    .send(WsMessage::Text(json.clone()))
                                    .await
                                {
                                    Ok(_) => {
                                        log_session_message(
                                            &session_service,
                                            Some(&session_id),
                                            MessageDirection::Outgoing,
                                            "system",
                                            &json,
                                        )
                                        .await;
                                    }
                                    Err(send_err) => {
                                        error!(
                                            "Failed to send error message: {}",
                                            send_err
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(WsMessage::Close(_)) => {
                info!("WebSocket connection closed");
                
                // Перед закрытием отправляем оставшиеся данные из буфера (если есть)
                // Даже если меньше 0.5 секунды, попробуем отправить (может быть последние слова)
                if let Some(remaining_samples) = SESSION_MANAGER
                    .take_audio_samples_force(&session_id, true)
                    .await
                {
                    if !remaining_samples.is_empty() {
                        info!(
                            "Sending remaining {} samples from buffer before closing",
                            remaining_samples.len()
                        );
                        // Пытаемся отправить, но не обрабатываем ошибки (соединение уже закрывается)
                        let _ = handle_audio_data(
                            &services,
                            &session_id,
                            &remaining_samples,
                            SERVER_OPUS_SAMPLE_RATE,
                            &mut sender,
                        )
                        .await;
                    }
                }
                
                // Очищаем буфер при закрытии соединения
                SESSION_MANAGER.clear_audio_buffer(&session_id).await;
                
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // Очищаем сессию
    SESSION_MANAGER.remove_session(&session_id).await;
    info!("Session removed: {}", session_id);

    if let Err(err) = session_service.close_session(&session_id).await {
        warn!("Failed to close session {}: {}", session_id, err);
    }

    info!("WebSocket connection ended");
}

#[instrument(skip_all, fields(session_id = %session_id))]
async fn handle_listen_text(
    services: &Services,
    session_id: &Uuid,
    text: &str,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    info!("Processing listen text: '{}'", text);

    // Обрабатываем текст через LLM
    let messages = build_llm_messages(services, text).await;

    info!("Calling LLM service with {} messages", messages.len());
    let mut response = match services.llm.chat(messages).await {
        Ok(resp) => {
            info!("LLM response received: '{}'", resp);
            resp
        }
        Err(e) => {
            error!("LLM service error: {}", e);
            error!("Error details: {:?}", e);
            // Возвращаем ошибку, но не падаем
            return Err(e).context("LLM service failed");
        }
    };

    if response.trim().is_empty() {
        warn!("LLM returned empty response, using default fallback");
        response = "Извините, я не смог придумать ответ.".to_string();
    }

    let decision = parse_character_decision(&response);
    response = decision.speech;
    let emotion = normalize_emotion(decision.emotion.as_deref(), &response);
    if let Err(err) = apply_character_actions(session_id, sender, &decision.actions).await {
        warn!("Failed to apply robot character actions: {}", err);
    }

    // Отправляем LLM ответ текстом
    let llm_msg = Message::Llm(crate::websocket::protocol::LlmMessage {
        session_id: session_id.to_string(),
        emotion: Some(emotion.clone()),
        text: Some(response.clone()),
    });

    let llm_json = serde_json::to_string(&llm_msg).context("Failed to serialize LLM message")?;

    info!("Sending LLM message: {}", llm_json);
    match sender.send(WsMessage::Text(llm_json.clone())).await {
        Ok(_) => {
            info!("LLM message sent successfully");
            // Flush для гарантии отправки
            if let Err(e) = sender.flush().await {
                error!("Failed to flush WebSocket after LLM message: {}", e);
            }
            log_session_message(
                &services.session,
                Some(session_id),
                MessageDirection::Outgoing,
                "llm",
                &llm_json,
            )
            .await;
        }
        Err(e) => {
            error!("Failed to send LLM message: {}", e);
            return Err(anyhow::anyhow!("Failed to send LLM message: {}", e));
        }
    }

    // Отправляем ответ через TTS
    info!("Synthesizing TTS for response: '{}'", response);
    // Получаем формат из сессии или используем из конфигурации
    let session = SESSION_MANAGER.get_session(session_id).await;
    let audio_format_option = session.and_then(|s| s.audio_format);
    let audio_format = audio_format_option.as_deref();
    info!("Using audio format: {:?}", audio_format);
    let tts_audio = match services.tts.synthesize_with_format(&response, audio_format).await {
        Ok(audio) => {
            info!("TTS audio synthesized: {} bytes", audio.total_bytes());
            audio
        }
        Err(e) => {
            error!("TTS synthesis error: {}", e);
            // Не рвём сессию: клиент уже получил текст LLM, просто сообщим что TTS не удалось.
            let mut err_text = format!("TTS error: {}", e);
            // Ограничиваем длину, чтобы не засорять UI/логи на устройстве.
            if err_text.len() > 200 {
                err_text.truncate(200);
            }
            let msg = Message::Tts(crate::websocket::protocol::TtsMessage {
                session_id: session_id.to_string(),
                state: "error".to_string(),
                text: Some(err_text),
            });
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = sender.send(WsMessage::Text(json)).await;
                let _ = sender.flush().await;
            }
            return Ok(());
        }
    };

    info!("Sending TTS audio: {} bytes", tts_audio.total_bytes());
    let audio_total = tts_audio.total_bytes();
    let send_result = send_tts_audio(sender, session_id, tts_audio).await;

    if let Err(e) = send_result {
        let error_msg = format!("{}", e);
        if error_msg.contains("Broken pipe") || error_msg.contains("Connection closed") {
            warn!("Client closed connection before TTS audio could be sent. This is normal if client disconnected.");
        } else {
            error!("Failed to send TTS audio: {}", error_msg);
            return Err(e);
        }
    } else {
        info!("TTS audio sent successfully");
        log_session_message(
            &services.session,
            Some(session_id),
            MessageDirection::Outgoing,
            "tts_audio",
            &format!("{} bytes", audio_total),
        )
        .await;
    }

    Ok(())
}

#[instrument(skip_all, fields(session_id = %session_id))]
async fn handle_stt_message(
    services: &Services,
    session_id: &Uuid,
    text: &str,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    info!("Processing STT text: '{}'", text);

    // Отправляем транскрипцию обратно клиенту
    let stt_msg = Message::Stt(crate::websocket::protocol::SttMessage {
        session_id: session_id.to_string(),
        text: text.to_string(),
    });

    let stt_json = serde_json::to_string(&stt_msg).context("Failed to serialize STT message")?;

    info!("Sending STT message: {}", stt_json);
    match sender.send(WsMessage::Text(stt_json.clone())).await {
        Ok(_) => {
            info!("STT message sent successfully");
            if let Err(e) = sender.flush().await {
                // Если flush не удался, это может означать, что соединение закрыто
                // Но это не критично - сообщение уже отправлено
                warn!("Failed to flush WebSocket after STT message (connection may be closed): {}", e);
            }
            log_session_message(
                &services.session,
                Some(session_id),
                MessageDirection::Outgoing,
                "stt",
                &stt_json,
            )
            .await;
        }
        Err(e) => {
            // Если соединение закрыто клиентом, просто логируем и продолжаем
            // Не прерываем обработку, так как LLM и TTS могут быть полезны для других клиентов
            let error_msg = format!("{}", e);
            if error_msg.contains("Broken pipe") || error_msg.contains("Connection closed") {
                warn!("Client closed connection before STT message could be sent. Continuing processing anyway.");
                // Продолжаем обработку, но не отправляем сообщения клиенту
            } else {
                error!("Failed to send STT message: {}", e);
                return Err(anyhow::anyhow!("Failed to send STT message: {}", e));
            }
        }
    }

    // Обрабатываем транскрибированный текст через LLM
    let messages = build_llm_messages(services, text).await;

    info!("Calling LLM service with {} messages", messages.len());
    let mut response = match services.llm.chat(messages).await {
        Ok(resp) => {
            info!("LLM response received: '{}'", resp);
            resp
        }
        Err(e) => {
            // Важно: даже если LLM упал/нет ключа/лимит — всё равно отвечаем голосом,
            // иначе на устройстве будет только ">> ..." и "нет звука".
            error!("LLM service error: {}", e);
            warn!("Falling back to direct TTS response");
            format!("Я тебя услышал: {}", text)
        }
    };

    if response.trim().is_empty() {
        warn!("LLM returned empty response, using fallback");
        response = "Извините, я сейчас затрудняюсь ответить.".to_string();
    }

    let decision = parse_character_decision(&response);
    response = decision.speech;
    let emotion = normalize_emotion(decision.emotion.as_deref(), &response);
    if let Err(err) = apply_character_actions(session_id, sender, &decision.actions).await {
        warn!("Failed to apply robot character actions: {}", err);
    }

    // Отправляем LLM ответ
    let llm_msg = Message::Llm(crate::websocket::protocol::LlmMessage {
        session_id: session_id.to_string(),
        emotion: Some(emotion.clone()),
        text: Some(response.clone()),
    });

    let llm_json = serde_json::to_string(&llm_msg).context("Failed to serialize LLM message")?;

    info!("Sending LLM message: {}", llm_json);
    match sender.send(WsMessage::Text(llm_json.clone())).await {
        Ok(_) => {
            info!("LLM message sent successfully");
            // Flush для гарантии отправки
            if let Err(e) = sender.flush().await {
                warn!("Failed to flush WebSocket after LLM message (connection may be closed): {}", e);
            }
            log_session_message(
                &services.session,
                Some(session_id),
                MessageDirection::Outgoing,
                "llm",
                &llm_json,
            )
            .await;
        }
        Err(e) => {
            let error_msg = format!("{}", e);
            if error_msg.contains("Broken pipe") || error_msg.contains("Connection closed") {
                warn!("Client closed connection before LLM message could be sent. Continuing with TTS anyway.");
                // Продолжаем обработку для TTS
            } else {
                error!("Failed to send LLM message: {}", e);
                return Err(anyhow::anyhow!("Failed to send LLM message: {}", e));
            }
        }
    }

    // Отправляем TTS аудио
    info!("Synthesizing TTS for response: '{}'", response);

    // Обновляем текст на экране устройства (оно показывает его через tts:sentence_start)
    let sentence_start = Message::Tts(crate::websocket::protocol::TtsMessage {
        session_id: session_id.to_string(),
        state: "sentence_start".to_string(),
        text: Some(response.clone()),
    });
    if let Ok(json) = serde_json::to_string(&sentence_start) {
        let _ = sender.send(WsMessage::Text(json)).await;
        let _ = sender.flush().await;
    }

    // Получаем формат из сессии или используем из конфигурации
    let session = SESSION_MANAGER.get_session(session_id).await;
    let audio_format_option = session.and_then(|s| s.audio_format);
    let audio_format = audio_format_option.as_deref();
    let tts_audio = match services.tts.synthesize_with_format(&response, audio_format).await {
        Ok(audio) => {
            info!("TTS audio synthesized: {} bytes", audio.total_bytes());
            audio
        }
        Err(e) => {
            error!("TTS synthesis error: {}", e);
            let mut err_text = format!("TTS error: {}", e);
            if err_text.len() > 200 {
                err_text.truncate(200);
            }
            let msg = Message::Tts(crate::websocket::protocol::TtsMessage {
                session_id: session_id.to_string(),
                state: "error".to_string(),
                text: Some(err_text),
            });
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = sender.send(WsMessage::Text(json)).await;
                let _ = sender.flush().await;
            }
            return Ok(());
        }
    };

    info!("Sending TTS audio: {} bytes", tts_audio.total_bytes());
    let audio_total = tts_audio.total_bytes();
    let send_result = send_tts_audio(sender, session_id, tts_audio).await;

    if let Err(e) = send_result {
        let error_msg = format!("{}", e);
        if error_msg.contains("Broken pipe") || error_msg.contains("Connection closed") {
            warn!("Client closed connection before TTS audio could be sent. This is normal if client disconnected.");
        } else {
            error!("Failed to send TTS audio: {}", error_msg);
            return Err(e);
        }
    } else {
        info!("TTS audio sent successfully");
        log_session_message(
            &services.session,
            Some(session_id),
            MessageDirection::Outgoing,
            "tts_audio",
            &format!("{} bytes", audio_total),
        )
        .await;
    }

    Ok(())
}

#[instrument(skip_all, fields(session_id = %session_id))]
async fn handle_tts_request(
    services: &Services,
    session_id: &Uuid,
    text: &str,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    audio_processor: &mut Option<AudioProcessor>,
) -> anyhow::Result<()> {
    // Синтезируем речь
    // Получаем формат из сессии или используем из конфигурации
    let session = SESSION_MANAGER.get_session(session_id).await;
    let audio_format_option = session.and_then(|s| s.audio_format);
    let audio_format = audio_format_option.as_deref();
    let synthesized = services.tts.synthesize_with_format(text, audio_format).await?;
    let audio_len = synthesized.total_bytes();

    send_tts_audio(sender, session_id, synthesized).await?;

    log_session_message(
        &services.session,
        Some(session_id),
        MessageDirection::Outgoing,
        "tts_audio",
        &format!("{} bytes", audio_len),
    )
    .await;

    Ok(())
}

async fn send_tts_audio(
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    session_id: &Uuid,
    synthesized: SynthesizedAudio,
) -> anyhow::Result<()> {
    let use_bp3 = SESSION_MANAGER
        .get_session(session_id)
        .await
        .map(|s| s.protocol_version == 3)
        .unwrap_or(false);

    // Устройство может включать усилитель/кодек только после получения tts:start.
    let start = Message::Tts(crate::websocket::protocol::TtsMessage {
        session_id: session_id.to_string(),
        state: "start".to_string(),
        text: None,
    });
    if let Ok(json) = serde_json::to_string(&start) {
        if let Err(e) = sender.send(WsMessage::Text(json)).await {
            warn!("Failed to send tts:start: {}", e);
        } else {
            let _ = sender.flush().await;
        }
    }

    match synthesized {
        SynthesizedAudio::OpusFrames(frames) => {
            info!("Sending {} Opus frames (paced {}ms)", frames.len(), STREAMING_FRAME_DELAY_MS);
            for (idx, frame) in frames.into_iter().enumerate() {
                if frame.is_empty() {
                    continue;
                }
                let payload = if use_bp3 { frame_bp3(&frame)? } else { frame };
                if let Err(e) = sender.send(WsMessage::Binary(payload)).await {
                    return Err(anyhow::anyhow!("Failed to send Opus frame {}: {}", idx, e));
                }
                sleep(Duration::from_millis(STREAMING_FRAME_DELAY_MS)).await;
            }
            let _ = sender.flush().await;
        }
        SynthesizedAudio::Binary(data) => {
            sender.send(WsMessage::Binary(data)).await?;
            let _ = sender.flush().await;
        }
    }

    let stop = Message::Tts(crate::websocket::protocol::TtsMessage {
        session_id: session_id.to_string(),
        state: "stop".to_string(),
        text: None,
    });
    if let Ok(json) = serde_json::to_string(&stop) {
        if let Err(e) = sender.send(WsMessage::Text(json)).await {
            warn!("Failed to send tts:stop: {}", e);
        } else {
            let _ = sender.flush().await;
        }
    }

    Ok(())
}

#[instrument(skip_all, fields(session_id = %_session_id))]
async fn handle_llm_message(
    services: &Services,
    _session_id: &Uuid,
    text: &str,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    let messages = build_llm_messages(services, text).await;

    let response = services.llm.chat(messages).await?;

    let emotion = detect_emotion(&response).to_string();
    let response_msg = Message::Llm(crate::websocket::protocol::LlmMessage {
        session_id: _session_id.to_string(),
        emotion: Some(emotion.clone()),
        text: Some(response),
    });

    let llm_json = serde_json::to_string(&response_msg)?;
    sender
        .send(WsMessage::Text(llm_json.clone()))
        .await?;

    log_session_message(
        &services.session,
        Some(_session_id),
        MessageDirection::Outgoing,
        "llm",
        &llm_json,
    )
    .await;

    Ok(())
}

#[instrument(skip_all, fields(session_id = %_session_id))]
async fn handle_mcp_message(
    services: &Services,
    _session_id: &Uuid,
    payload: serde_json::Value,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    use crate::mcp::server::McpServer;
    let mcp_server = McpServer::new();
    let response = mcp_server.handle_request(payload, Some(services)).await?;

    let response_msg = Message::Mcp(crate::websocket::protocol::McpMessage {
        session_id: _session_id.to_string(),
        payload: response,
    });

    let mcp_json = serde_json::to_string(&response_msg)?;
    sender
        .send(WsMessage::Text(mcp_json.clone()))
        .await?;

    log_session_message(
        &services.session,
        Some(_session_id),
        MessageDirection::Outgoing,
        "mcp",
        &mcp_json,
    )
    .await;

    Ok(())
}

#[instrument(skip_all, fields(session_id = %session_id, samples = pcm_samples.len()))]
async fn handle_audio_data(
    services: &Services,
    session_id: &Uuid,
    pcm_samples: &[i16],
    sample_rate: u32,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    let duration_sec = pcm_samples.len() as f64 / sample_rate.max(1) as f64;
    info!(
        session_id = %session_id,
        pcm_samples = pcm_samples.len(),
        sample_rate_hz = sample_rate,
        duration_sec = duration_sec,
        "STT: transcribe_pcm (PCM -> WAV internally)"
    );

    let text = services
        .stt
        .transcribe_pcm(pcm_samples, sample_rate, 1)
        .await
        .with_context(|| {
            format!(
                "STT transcription failed (session={}, samples={}, {:.3}s @ {}Hz)",
                session_id,
                pcm_samples.len(),
                duration_sec,
                sample_rate
            )
        })?;

    info!(
        session_id = %session_id,
        transcript_len = text.len(),
        "STT result"
    );
    debug!(session_id = %session_id, transcript = %text, "STT transcript text");

    if !text.is_empty() {
        // Обрабатываем через LLM и отправляем ответы
        handle_stt_message(services, session_id, &text, sender)
            .await
            .context("Failed to process STT result")?;
    } else {
        warn!("Empty transcription result");
    }

    Ok(())
}

#[instrument(skip_all, fields(session_id = %session_id, bytes = audio_data.len()))]
async fn handle_raw_audio(
    services: &Services,
    session_id: &Uuid,
    audio_data: &[u8],
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    info!(
        session_id = %session_id,
        bytes = audio_data.len(),
        sniff = sniff_binary_audio_kind(audio_data),
        preview = %audio_hex_preview(audio_data, 16),
        "raw audio -> STT (no Opus decode on server)"
    );

    let text = match services.stt.transcribe(audio_data).await {
        Ok(t) => {
            info!(
                session_id = %session_id,
                transcript_len = t.len(),
                "STT OK (raw path)"
            );
            debug!(session_id = %session_id, transcript = %t, "STT transcript (raw path)");
            t
        }
        Err(e) => {
            error!(
                session_id = %session_id,
                "STT failed on raw audio: {:#}",
                e
            );
            return Err(e).context("STT transcription failed");
        }
    };

    if !text.is_empty() {
        info!(
            session_id = %session_id,
            "LLM pipeline after STT (raw path)"
        );
        // Обрабатываем через LLM и отправляем ответы
        match handle_stt_message(services, session_id, &text, sender).await {
            Ok(_) => {
                info!(session_id = %session_id, "LLM pipeline OK (raw path)");
            }
            Err(e) => {
                error!(
                    session_id = %session_id,
                    "LLM after STT failed: {:#}",
                    e
                );
                return Err(e).context("Failed to process STT result");
            }
        }
    } else {
        warn!(
            session_id = %session_id,
            "STT returned empty string (raw path)"
        );
        // Отправляем сообщение клиенту о пустом результате
        let empty_msg = Message::System(crate::websocket::protocol::SystemMessage {
            session_id: session_id.to_string(),
            command: "warning: Empty transcription result".to_string(),
        });
        if let Ok(json) = serde_json::to_string(&empty_msg) {
            match sender
                .send(WsMessage::Text(json.clone()))
                .await
            {
                Ok(_) => {
                    log_session_message(
                        &services.session,
                        Some(session_id),
                        MessageDirection::Outgoing,
                        "system",
                        &json,
                    )
                    .await;
                }
                Err(send_err) => {
                    error!("Failed to send empty transcription warning: {}", send_err);
                }
            }
        }
    }

    info!("=== Raw audio processing completed ===");
    Ok(())
}
