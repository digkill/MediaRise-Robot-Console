//! WebSocket обработка

pub mod audio;
pub mod protocol;
pub mod session;

use anyhow::Context;
use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio::time::{sleep, Duration};
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
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
    let mut messages = Vec::new();
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

                        if let Err(err) =
                            session_service.persist_session(&sid, &resolved_device_id).await
                        {
                            warn!("Failed to persist session {}: {}", sid, err);
                        }

                        log_session_message(
                            &session_service,
                            Some(&sid),
                            MessageDirection::Incoming,
                            "hello",
                            &text,
                        )
                        .await;

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
                            log_session_message(
                                &session_service,
                                Some(&sid),
                                MessageDirection::Outgoing,
                                "hello",
                                &response_json,
                            )
                            .await;
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
                        if let Err(e) =
                            handle_mcp_message(&services, &session_id, mcp.payload, &mut sender)
                                .await
                        {
                            error!("Failed to handle MCP: {}", e);
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
                info!("Received binary audio data: {} bytes", data.len());

                // Обрабатываем аудио данные
                if let Some(ref mut processor) = audio_processor {
                    info!("Audio processor available, trying to decode audio...");
                    match processor.process_incoming_audio(&data) {
                        Ok(pcm_samples) => {
                            info!("Decoded audio to PCM: {} samples", pcm_samples.len());
                            
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
                                "Audio buffer: added {} samples, total: {:.2} seconds, ready: {}",
                                pcm_samples.len(),
                                buffer_duration,
                                is_ready
                            );
                            
                            // Если буфер готов (накоплено >= 0.5 секунды), отправляем в STT
                            if is_ready {
                                if let Some(accumulated_samples) = SESSION_MANAGER
                                    .take_audio_samples(&session_id)
                                    .await
                                {
                                    info!(
                                        "Buffer ready! Sending {} samples ({:.2} seconds) to STT",
                                        accumulated_samples.len(),
                                        accumulated_samples.len() as f32 / sample_rate as f32
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
                                        error!("Failed to handle audio: {}", e);
                                        error!("Audio handling error details: {:?}", e);
                                    }
                                }
                            } else {
                                // Буфер еще не готов - просто накапливаем
                                info!(
                                    "Buffer not ready yet: {:.2} seconds (need >= 0.5 seconds)",
                                    buffer_duration
                                );
                            }
                        }
                        Err(e) => {
                            warn!("Failed to process audio through processor: {}", e);
                            warn!("Error details: {:?}", e);
                            // Попробуем отправить напрямую на STT (может быть WebM от браузера)
                            info!(
                                "Trying to send raw audio to STT (may be WebM format from browser)"
                            );
                            match handle_raw_audio(&services, &session_id, &data, &mut sender).await
                            {
                                Ok(_) => {
                                    info!("Successfully processed raw audio");
                                }
                                Err(e2) => {
                                    error!("Failed to handle raw audio: {}", e2);
                                    error!("Raw audio error details: {:?}", e2);
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
                    info!("Audio processor not initialized, sending raw audio directly to STT");
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

    // Отправляем LLM ответ текстом
    let emotion = detect_emotion(&response).to_string();
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
            // Не падаем, просто не отправляем аудио
            return Err(e).context("TTS synthesis failed");
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
            error!("LLM service error: {}", e);
            error!("Error details: {:?}", e);
            return Err(e).context("LLM service failed");
        }
    };

    if response.trim().is_empty() {
        warn!("LLM returned empty response, using fallback");
        response = "Извините, я сейчас затрудняюсь ответить.".to_string();
    }

    // Отправляем LLM ответ
    let emotion = detect_emotion(&response).to_string();
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
            return Err(e).context("TTS synthesis failed");
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
                if let Err(e) = sender.send(WsMessage::Binary(frame)).await {
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
    info!("Handling audio data: {} PCM samples", pcm_samples.len());

    info!(
        "Sending PCM to STT: samples={}, sample_rate={}Hz",
        pcm_samples.len(),
        sample_rate
    );
    let text = services
        .stt
        .transcribe_pcm(pcm_samples, sample_rate, 1)
        .await
        .context("STT transcription failed")?;

    info!("STT transcription result: '{}'", text);

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
    info!("=== Starting raw audio processing ===");
    info!(
        "Audio data size: {} bytes (may be WebM/Opus from browser)",
        audio_data.len()
    );

    // Отправляем сырые данные на STT (OpenAI Whisper поддерживает различные форматы)
    info!("Sending audio to STT service...");
    let text = match services.stt.transcribe(audio_data).await {
        Ok(t) => {
            info!("✅ STT transcription successful: '{}'", t);
            t
        }
        Err(e) => {
            error!("❌ STT transcription failed: {}", e);
            error!("STT error details: {:?}", e);
            return Err(e).context("STT transcription failed");
        }
    };

    if !text.is_empty() {
        info!("Processing STT result through LLM pipeline...");
        // Обрабатываем через LLM и отправляем ответы
        match handle_stt_message(services, session_id, &text, sender).await {
            Ok(_) => {
                info!("✅ Successfully processed STT result through LLM");
            }
            Err(e) => {
                error!("❌ Failed to process STT result: {}", e);
                error!("LLM processing error details: {:?}", e);
                return Err(e).context("Failed to process STT result");
            }
        }
    } else {
        warn!("⚠️ Empty transcription result from STT");
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
