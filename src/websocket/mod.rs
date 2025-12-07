//! WebSocket обработка

pub mod audio;
pub mod protocol;
pub mod session;

use anyhow::Context;
use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::services::Services;
use crate::storage::Storage;
use crate::websocket::audio::AudioProcessor;
use crate::websocket::protocol::{AudioParams, Features, HelloMessage, Message};
use crate::websocket::session::{AudioParams as SessionAudioParams, SessionManager};

// Глобальный менеджер сессий
static SESSION_MANAGER: once_cell::sync::Lazy<Arc<SessionManager>> =
    once_cell::sync::Lazy::new(|| Arc::new(SessionManager::new()));

pub async fn handle_connection(
    socket: WebSocket,
    (config, services, storage): (Config, Services, Storage),
) {
    info!("New WebSocket connection");

    let (mut sender, mut receiver) = socket.split();
    let mut session_id: Option<Uuid> = None;
    let mut audio_processor: Option<AudioProcessor> = None;
    let mut device_id: Option<String> = None;

    // Ожидаем hello сообщение
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => {
                match serde_json::from_str::<Message>(&text) {
                    Ok(Message::Hello(hello)) => {
                        info!("Received hello message: {:?}", hello);

                        // Создаем сессию
                        let audio_params = hello.audio_params.unwrap_or(AudioParams {
                            format: "opus".to_string(),
                            sample_rate: 48000,
                            channels: 1,
                            frame_duration: 20,
                        });

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

                        device_id = Some(dev_id.clone());

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

                        if let Err(e) = sender
                            .send(WsMessage::Text(
                                serde_json::to_string(&response).unwrap_or_default(),
                            ))
                            .await
                        {
                            error!("Failed to send hello response: {}", e);
                            break;
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
                                            let _ = sender.send(WsMessage::Text(json)).await;
                                        }
                                    }
                                }
                            } else {
                                warn!("Listen message without text");
                            }
                        }
                    }
                    Ok(Message::Stt(stt)) => {
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
                                    if let Err(send_err) = sender.send(WsMessage::Text(json)).await
                                    {
                                        error!("Failed to send error message: {}", send_err);
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Tts(tts)) => {
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
                        info!("MCP message: {:?}", mcp.payload);
                        if let Err(e) =
                            handle_mcp_message(&services, &session_id, mcp.payload, &mut sender)
                                .await
                        {
                            error!("Failed to handle MCP: {}", e);
                        }
                    }
                    Ok(Message::System(system)) => {
                        info!("System command: {}", system.command);
                        // Обрабатываем системные команды
                    }
                    Ok(Message::Abort(abort)) => {
                        info!("Abort message: {:?}", abort.reason);
                        break;
                    }
                    Ok(Message::Goodbye(_)) => {
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
                            // Отправляем на STT
                            if let Err(e) =
                                handle_audio_data(&services, &session_id, &pcm_samples, &mut sender)
                                    .await
                            {
                                error!("Failed to handle audio: {}", e);
                                error!("Audio handling error details: {:?}", e);
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
                                        if let Err(send_err) =
                                            sender.send(WsMessage::Text(json)).await
                                        {
                                            error!("Failed to send error message: {}", send_err);
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
                                if let Err(send_err) = sender.send(WsMessage::Text(json)).await {
                                    error!("Failed to send error message: {}", send_err);
                                }
                            }
                        }
                    }
                }
            }
            Ok(WsMessage::Close(_)) => {
                info!("WebSocket connection closed");
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
    let messages = vec![crate::services::llm::ChatMessage {
        role: "user".to_string(),
        content: text.to_string(),
    }];

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
    let llm_msg = Message::Llm(crate::websocket::protocol::LlmMessage {
        session_id: session_id.to_string(),
        emotion: None,
        text: Some(response.clone()),
    });

    let llm_json = serde_json::to_string(&llm_msg).context("Failed to serialize LLM message")?;

    info!("Sending LLM message: {}", llm_json);
    match sender.send(WsMessage::Text(llm_json)).await {
        Ok(_) => {
            info!("LLM message sent successfully");
            // Flush для гарантии отправки
            if let Err(e) = sender.flush().await {
                error!("Failed to flush WebSocket after LLM message: {}", e);
            }
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
            info!("TTS audio synthesized: {} bytes", audio.len());
            audio
        }
        Err(e) => {
            error!("TTS synthesis error: {}", e);
            // Не падаем, просто не отправляем аудио
            return Err(e).context("TTS synthesis failed");
        }
    };

    info!("Sending TTS audio: {} bytes", tts_audio.len());
    match sender.send(WsMessage::Binary(tts_audio)).await {
        Ok(_) => {
            info!("TTS audio sent successfully");
            if let Err(e) = sender.flush().await {
                error!("Failed to flush WebSocket after TTS audio: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to send TTS audio: {}", e);
            return Err(anyhow::anyhow!("Failed to send TTS audio: {}", e));
        }
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
    match sender.send(WsMessage::Text(stt_json)).await {
        Ok(_) => {
            info!("STT message sent successfully");
            if let Err(e) = sender.flush().await {
                error!("Failed to flush WebSocket after STT message: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to send STT message: {}", e);
            return Err(anyhow::anyhow!("Failed to send STT message: {}", e));
        }
    }

    // Обрабатываем транскрибированный текст через LLM
    let messages = vec![crate::services::llm::ChatMessage {
        role: "user".to_string(),
        content: text.to_string(),
    }];

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
    let llm_msg = Message::Llm(crate::websocket::protocol::LlmMessage {
        session_id: session_id.to_string(),
        emotion: None,
        text: Some(response.clone()),
    });

    let llm_json = serde_json::to_string(&llm_msg).context("Failed to serialize LLM message")?;

    info!("Sending LLM message: {}", llm_json);
    match sender.send(WsMessage::Text(llm_json)).await {
        Ok(_) => {
            info!("LLM message sent successfully");
            // Flush для гарантии отправки
            if let Err(e) = sender.flush().await {
                error!("Failed to flush WebSocket after LLM message: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to send LLM message: {}", e);
            return Err(anyhow::anyhow!("Failed to send LLM message: {}", e));
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
            info!("TTS audio synthesized: {} bytes", audio.len());
            audio
        }
        Err(e) => {
            error!("TTS synthesis error: {}", e);
            return Err(e).context("TTS synthesis failed");
        }
    };

    info!("Sending TTS audio: {} bytes", tts_audio.len());
    match sender.send(WsMessage::Binary(tts_audio)).await {
        Ok(_) => {
            info!("TTS audio sent successfully");
            if let Err(e) = sender.flush().await {
                error!("Failed to flush WebSocket after TTS audio: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to send TTS audio: {}", e);
            return Err(anyhow::anyhow!("Failed to send TTS audio: {}", e));
        }
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
    let opus_audio = services.tts.synthesize_with_format(text, audio_format).await?;

    // Отправляем аудио
    sender.send(WsMessage::Binary(opus_audio)).await?;

    Ok(())
}

#[instrument(skip_all, fields(session_id = %_session_id))]
async fn handle_llm_message(
    services: &Services,
    _session_id: &Uuid,
    text: &str,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    let messages = vec![crate::services::llm::ChatMessage {
        role: "user".to_string(),
        content: text.to_string(),
    }];

    let response = services.llm.chat(messages).await?;

    let response_msg = Message::Llm(crate::websocket::protocol::LlmMessage {
        session_id: _session_id.to_string(),
        emotion: None,
        text: Some(response),
    });

    sender
        .send(WsMessage::Text(serde_json::to_string(&response_msg)?))
        .await?;

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

    sender
        .send(WsMessage::Text(serde_json::to_string(&response_msg)?))
        .await?;

    Ok(())
}

#[instrument(skip_all, fields(session_id = %session_id, samples = pcm_samples.len()))]
async fn handle_audio_data(
    services: &Services,
    session_id: &Uuid,
    pcm_samples: &[i16],
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
) -> anyhow::Result<()> {
    info!("Handling audio data: {} PCM samples", pcm_samples.len());

    // Конвертируем PCM в байты для STT
    let pcm_bytes = crate::utils::audio::utils::pcm_samples_to_bytes(pcm_samples);

    info!("Sending {} bytes to STT", pcm_bytes.len());
    // Отправляем на STT
    let text = services
        .stt
        .transcribe(&pcm_bytes)
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
            if let Err(send_err) = sender.send(WsMessage::Text(json)).await {
                error!("Failed to send empty transcription warning: {}", send_err);
            }
        }
    }

    info!("=== Raw audio processing completed ===");
    Ok(())
}
