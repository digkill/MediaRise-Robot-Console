#!/usr/bin/env python3
"""
Python клиент для тестирования WebSocket соединения с MediaRise Robot Console.

Этот скрипт:
1. Подключается к WebSocket серверу
2. Отправляет Hello сообщение
3. Записывает аудио с микрофона (5 секунд)
4. Кодирует аудио в Opus и отправляет на сервер
5. Получает транскрипцию (STT), ответ LLM и аудио ответ (TTS)
6. Декодирует и воспроизводит аудио ответ

Требования:
- pip install websockets sounddevice opuslib
- brew install opus (на macOS)
"""

import asyncio
import ctypes
import ctypes.util
import json
import time
import websockets

try:
    import sounddevice as sd
except ImportError:
    raise RuntimeError(
        "sounddevice not installed. Install with: pip install sounddevice"
    )

try:
    from opuslib import Encoder, Decoder
except Exception as exc:
    opus_path = ctypes.util.find_library("opus")
    hint = (
        "Opus library not found. Install system libopus and Python bindings:\n"
        "  brew install opus\n"
        "  pip install --force-reinstall opuslib\n"
        "If still failing, export DYLD_LIBRARY_PATH to your libopus (e.g. /opt/homebrew/lib)."
    )
    raise RuntimeError(f"{hint}\nDetected opus path: {opus_path}") from exc


async def robot_client():
    """
    Главная функция клиента.
    
    Подключается к WebSocket серверу и обрабатывает голосовое взаимодействие.
    """
    # URL WebSocket сервера
    uri = "ws://localhost:8080/ws"
    
    # Подключаемся к WebSocket серверу
    # async with - это контекстный менеджер, который автоматически закроет соединение
    async with websockets.connect(uri) as websocket:
        # ============================================
        # Настройка аудио параметров
        # ============================================
        sample_rate = 48000  # Частота дискретизации (48 kHz - стандарт для Opus)
        channels = 1         # Моно (1 канал)
        frame_size = sample_rate // 50  # 20 мс кадр => 960 сэмплов при 48 kHz
        record_seconds = 5   # Записываем 5 секунд аудио
        
        # Создаем Opus энкодер для кодирования PCM в Opus
        # application="audio" - для голосового аудио (не музыки)
        encoder = Encoder(sample_rate, channels, application="audio")
        
        # Создаем Opus декодер для декодирования Opus в PCM
        # (но если сервер отправляет MP3, декодер не понадобится)
        decoder = Decoder(sample_rate, channels)
        
        # Создаем поток для вывода звука (воспроизведение)
        # RawOutputStream - сырой поток без дополнительной обработки
        output_stream = sd.RawOutputStream(
            samplerate=sample_rate,
            channels=channels,
            dtype="int16",  # 16-битные целые числа (стандарт для PCM)
            blocksize=frame_size,  # Размер блока для буферизации
        )
        output_stream.start()  # Запускаем поток вывода
        
        # ============================================
        # ШАГ 1: Отправка Hello сообщения
        # ============================================
        # Hello сообщение - обязательное первое сообщение при подключении
        # Оно устанавливает параметры соединения и создает сессию
        hello = {
            "type": "hello",  # Тип сообщения
            "version": 3,     # Версия протокола
            "transport": "websocket",  # Тип транспорта
            "features": {
                "aec": True,   # Acoustic Echo Cancellation (подавление эха)
                "mcp": False   # Model Context Protocol (пока не используем)
            },
            "audio_params": {
                "format": "opus",      # Формат входящего аудио (от клиента к серверу)
                "sample_rate": 48000,  # Частота дискретизации
                "channels": 1,         # Количество каналов (моно)
                "frame_duration": 20   # Длительность кадра в миллисекундах
            },
            # ВАЖНО: используем "audio_format" (с подчеркиванием), а не "audioFormat"
            # Это формат аудио для ответов от сервера (TTS)
            "audio_format": "mp3",  # Можно выбрать "opus" или "mp3"
        }
        
        # Отправляем Hello сообщение как JSON строку
        await websocket.send(json.dumps(hello))
        print(f"✅ Sent Hello message with audio_format: {hello['audio_format']}")
        
        # ============================================
        # ШАГ 2: Получение ответа Hello
        # ============================================
        # Сервер должен ответить Hello сообщением с session_id
        response = await websocket.recv()
        hello_response = json.loads(response)
        
        # Проверяем, что это действительно Hello ответ
        if hello_response.get("type") != "hello":
            print(f"⚠️ Unexpected response type: {hello_response.get('type')}")
        
        session_id = hello_response.get("session_id")
        print(f"✅ Session ID: {session_id}")
        
        # Проверяем, какой формат аудио будет использовать сервер
        # Это формат, который мы запросили в Hello сообщении
        server_audio_format = hello_response.get("audio_format", hello.get("audio_format", "opus"))
        print(f"📦 Server will send audio in format: {server_audio_format}")
        
        # ============================================
        # ШАГ 3: Запись и отправка аудио
        # ============================================
        # Создаем событие для остановки (сигнал завершения)
        stop = asyncio.Event()
        
        async def send_audio():
            """
            Асинхронная функция для записи аудио с микрофона и отправки на сервер.
            
            Процесс:
            1. Открываем поток ввода с микрофона
            2. Читаем PCM сэмплы блоками по 20 мс
            3. Кодируем PCM в Opus
            4. Отправляем Opus кадры на сервер через WebSocket
            """
            last_overflow_log = 0.0  # Время последнего предупреждения о переполнении
            frames: list[bytes] = []  # Список для накопления PCM кадров
            
            # Открываем поток ввода с микрофона
            with sd.RawInputStream(
                samplerate=sample_rate,
                channels=channels,
                dtype="int16",
                blocksize=frame_size,
            ) as mic:
                start_ts = time.monotonic()  # Время начала записи
                
                # Записываем аудио в течение record_seconds секунд
                while not stop.is_set() and (time.monotonic() - start_ts) < record_seconds:
                    try:
                        # Читаем один кадр PCM аудио с микрофона
                        # pcm_bytes - это numpy массив с аудио данными
                        # overflowed - флаг, указывающий на переполнение буфера
                        pcm_bytes, overflowed = mic.read(frame_size)
                        
                        # Если произошло переполнение, выводим предупреждение (не чаще раза в 5 секунд)
                        if overflowed and time.time() - last_overflow_log > 5.0:
                            print("⚠️ Audio input overflowed - some audio may be lost")
                            last_overflow_log = time.time()
                        
                        # Преобразуем numpy массив в bytes
                        # mic.read возвращает cffi buffer, нужно привести к bytes
                        pcm_raw = bytes(pcm_bytes)
                        frames.append(pcm_raw)
                        
                        # Отдаем управление циклу событий (yield)
                        # Это важно для асинхронности - позволяет другим задачам выполняться
                        await asyncio.sleep(0)
                        
                    except websockets.ConnectionClosed:
                        # Соединение закрыто - останавливаем запись
                        print("❌ WebSocket connection closed during recording")
                        stop.set()
                    except Exception as exc:
                        print(f"❌ Send audio error: {exc}")
                        stop.set()
                
                # После записи всех кадров - кодируем и отправляем их
                if frames:
                    total_ms = len(frames) * 20  # Каждый кадр = 20 мс
                    print(f"📤 Sending {len(frames)} frames (~{total_ms} ms of audio)")
                
                # Кодируем каждый PCM кадр в Opus и отправляем
                for pcm_raw in frames:
                    try:
                        # Создаем C строковый буфер из bytes
                        # Opus энкодер требует C буфер, а не Python bytes
                        pcm_buf = ctypes.create_string_buffer(pcm_raw, len(pcm_raw))
                        
                        # Кодируем PCM в Opus
                        # frame_size - размер кадра в сэмплах (960 для 20 мс при 48 kHz)
                        opus_frame = encoder.encode(pcm_buf, frame_size=frame_size)
                        
                        # Отправляем Opus кадр как бинарные данные через WebSocket
                        await websocket.send(opus_frame)
                        
                    except websockets.ConnectionClosed:
                        print("❌ WebSocket connection closed during sending")
                        stop.set()
                        break
                    except Exception as exc:
                        print(f"❌ Send audio error: {exc}")
                        stop.set()
                        break
                
                print("✅ Finished sending audio")
        
        async def receive_messages():
            """
            Асинхронная функция для приема сообщений от сервера.
            
            Обрабатывает:
            - JSON сообщения (STT транскрипция, LLM ответы)
            - Бинарные данные (аудио ответы от TTS)
            """
            try:
                while not stop.is_set():
                    # Ждем сообщение от сервера
                    message = await websocket.recv()
                    
                    if isinstance(message, bytes):
                        # ============================================
                        # БИНАРНЫЕ ДАННЫЕ - АУДИО ОТВЕТ ОТ TTS
                        # ============================================
                        print(f"🎵 Received audio: {len(message)} bytes")
                        
                        # Сначала используем формат из Hello ответа
                        # Это самый надежный способ, так как мы сами запросили этот формат
                        is_mp3 = (server_audio_format.lower() == "mp3")
                        is_opus = (server_audio_format.lower() == "opus")
                        
                        # Дополнительно проверяем по magic bytes для подтверждения
                        # (на случай, если сервер отправил не тот формат, что мы запросили)
                        detected_format = None
                        if len(message) >= 3:
                            # MP3 файлы начинаются с:
                            # - ID3 тег: "ID3" (первые 3 байта)
                            # - MP3 frame sync: 0xFF 0xFB или 0xFF 0xF3 (первые 2 байта)
                            if message[:3] == b"ID3":
                                detected_format = "mp3"
                                print("🔍 Detected MP3 format (ID3 tag)")
                            elif len(message) >= 2 and message[0] == 0xFF and (message[1] & 0xE0) == 0xE0:
                                # MP3 frame sync: 0xFF и следующие 3 бита = 111
                                detected_format = "mp3"
                                print("🔍 Detected MP3 format (frame sync)")
                            elif len(message) >= 4:
                                # Opus файлы могут начинаться с OggS (если это Ogg Opus)
                                # или просто с Opus пакетов (TOC байт)
                                # Проверяем на Ogg контейнер
                                if message[:4] == b"OggS":
                                    detected_format = "opus"
                                    print("🔍 Detected Opus format (Ogg container)")
                        
                        # Если определенный формат не совпадает с запрошенным - предупреждаем
                        if detected_format and detected_format != server_audio_format.lower():
                            print(f"⚠️ Received {detected_format} audio, but decoder is set for {server_audio_format}")
                            print(f"💡 Tip: Set audio_format to '{detected_format}' in Hello message to use {detected_format} decoder")
                            # Используем определенный формат вместо запрошенного
                            is_mp3 = (detected_format == "mp3")
                            is_opus = (detected_format == "opus")
                        
                        # Обрабатываем в зависимости от определенного формата
                        if is_mp3:
                            # MP3 формат - сохраняем в файл
                            # Для воспроизведения MP3 в Python нужна дополнительная библиотека
                            # (например, pydub + ffmpeg), поэтому просто сохраняем
                            filename = "response.mp3"
                            with open(filename, "wb") as f:
                                f.write(message)
                            print(f"💾 Saved MP3 audio to {filename}")
                            print("💡 To play MP3, use: afplay response.mp3 (macOS) or mpv response.mp3 (Linux)")
                            
                        elif is_opus:
                            # Opus формат - декодируем и воспроизводим
                            try:
                                # Декодируем Opus в PCM
                                pcm = decoder.decode(message, frame_size=frame_size)
                                
                                # Воспроизводим декодированное аудио
                                output_stream.write(pcm)
                                print("🔊 Playing decoded Opus audio")
                                
                            except Exception as e:
                                print(f"❌ Error decoding Opus: {e}")
                                print(f"   Audio length: {len(message)} bytes")
                                print(f"   First 10 bytes: {message[:10].hex()}")
                                
                                # Возможно, это не Opus, а другой формат
                                # Сохраняем в файл для анализа
                                with open("response_unknown.bin", "wb") as f:
                                    f.write(message)
                                print("💾 Saved unknown audio format to response_unknown.bin")
                        else:
                            # Не удалось определить формат - используем формат из Hello
                            print(f"⚠️ Could not determine audio format by magic bytes, using requested format: {server_audio_format}")
                            
                            if server_audio_format.lower() == "opus":
                                # Пробуем декодировать как Opus
                                try:
                                    pcm = decoder.decode(message, frame_size=frame_size)
                                    output_stream.write(pcm)
                                    print("✅ Successfully decoded as Opus")
                                except Exception as e:
                                    print(f"❌ Error decoding as Opus: {e}")
                                    # Сохраняем для анализа
                                    filename = "response_opus_error.bin"
                                    with open(filename, "wb") as f:
                                        f.write(message)
                                    print(f"💾 Saved to {filename} for analysis")
                            else:
                                # Сохраняем как MP3 (или другой формат)
                                filename = f"response.{server_audio_format.lower()}"
                                with open(filename, "wb") as f:
                                    f.write(message)
                                print(f"💾 Saved as {filename}")
                                print(f"💡 To play, use: afplay {filename} (macOS) or mpv {filename} (Linux)")
                    else:
                        # ============================================
                        # JSON СООБЩЕНИЯ - ТЕКСТОВЫЕ ОТВЕТЫ
                        # ============================================
                        data = json.loads(message)
                        msg_type = data.get("type")
                        
                        if msg_type == "stt":
                            # STT (Speech-to-Text) - транскрипция речи
                            text = data.get("text", "")
                            print(f"📝 Transcription (STT): {text}")
                            
                        elif msg_type == "llm":
                            # LLM ответ - текст от языковой модели
                            text = data.get("text", "")
                            print(f"🤖 LLM Response: {text}")
                            
                        elif msg_type == "hello":
                            # Повторный Hello (может быть, если сервер переподключился)
                            print(f"🔄 Received Hello again: {data}")
                            
                        elif msg_type == "system":
                            # Системное сообщение (ошибки, уведомления)
                            command = data.get("command", "")
                            print(f"⚙️ System message: {command}")
                            
                        else:
                            # Неизвестный тип сообщения
                            print(f"❓ Unknown message type: {msg_type}")
                            print(f"   Full message: {data}")
                            
            except websockets.ConnectionClosed:
                print("❌ WebSocket connection closed during receive")
                stop.set()
            except Exception as exc:
                print(f"❌ Receive error: {exc}")
                stop.set()
        
        # ============================================
        # Запуск параллельных задач
        # ============================================
        # Создаем две задачи, которые выполняются параллельно:
        # 1. send_task - запись и отправка аудио
        # 2. recv_task - прием сообщений от сервера
        send_task = asyncio.create_task(send_audio())
        recv_task = asyncio.create_task(receive_messages())
        
        # Ждем завершения отправки аудио
        await send_task
        
        # Ждем ответов от сервера (но не дольше 300 секунд)
        try:
            await asyncio.wait_for(recv_task, timeout=300.0)
        except asyncio.TimeoutError:
            print("⏱️ No response within 300s, closing connection")
        finally:
            # Останавливаем все задачи
            stop.set()
            
            # Закрываем соединение
            await websocket.close()
            
            # Ждем завершения всех задач (с обработкой исключений)
            await asyncio.gather(recv_task, return_exceptions=True)
            
            # Останавливаем поток вывода звука
            output_stream.stop()
            output_stream.close()
            
            print("✅ Connection closed")


# Запускаем клиент
if __name__ == "__main__":
    print("🚀 Starting MediaRise Robot Console WebSocket Client...")
    print("📡 Connecting to ws://localhost:8080/ws")
    print("🎤 Will record 5 seconds of audio from microphone")
    print("=" * 60)
    
    try:
        asyncio.run(robot_client())
    except KeyboardInterrupt:
        print("\n⚠️ Interrupted by user")
    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()

