# Тестирование отправки аудио

## Способы тестирования отправки аудио

### 1. HTML тест-клиент (Рекомендуется) ⭐

**Файл:** `websocket_test.html`

**Как использовать:**

1. Откройте `websocket_test.html` в браузере (Chrome, Firefox, Edge)
2. Нажмите "Подключиться" к WebSocket
3. Нажмите "Отправить Hello" для получения session_id
4. Нажмите "🎤 Начать запись" - браузер запросит разрешение на доступ к микрофону
5. Говорите в микрофон - вы увидите визуализацию уровня звука
6. Нажмите "⏹ Остановить запись"
7. Нажмите "📤 Отправить записанное аудио"
8. Получите ответы:
   - Транскрипция (STT) - текст
   - Ответ LLM - текст
   - Аудио ответ - автоматически воспроизводится

**Особенности:**
- ✅ Запись с микрофона в реальном времени
- ✅ Визуализация уровня звука
- ✅ Автоматическое воспроизведение ответа
- ✅ Поддержка Opus кодека
- ⚠️ Требует HTTPS или localhost для доступа к микрофону

### 2. Python скрипт с записью аудио

Создайте файл `test_audio_send.py`:

```python
import asyncio
import websockets
import json
import pyaudio
import wave
import io

# Параметры аудио
CHUNK = 1024
FORMAT = pyaudio.paInt16
CHANNELS = 1
RATE = 48000
RECORD_SECONDS = 5

async def test_audio_send():
    uri = "ws://localhost:8080/ws"
    
    async with websockets.connect(uri) as websocket:
        # 1. Отправить hello
        hello = {
            "type": "hello",
            "version": 3,
            "transport": "websocket",
            "features": {"aec": True, "mcp": False},
            "audio_params": {
                "format": "opus",
                "sample_rate": 48000,
                "channels": 1,
                "frame_duration": 20
            }
        }
        await websocket.send(json.dumps(hello))
        
        # 2. Получить ответ hello
        response = await websocket.recv()
        hello_response = json.loads(response)
        session_id = hello_response.get("session_id")
        print(f"Session ID: {session_id}")
        
        # 3. Записать аудио с микрофона
        print("Запись аудио...")
        audio = pyaudio.PyAudio()
        stream = audio.open(
            format=FORMAT,
            channels=CHANNELS,
            rate=RATE,
            input=True,
            frames_per_buffer=CHUNK
        )
        
        frames = []
        for _ in range(0, int(RATE / CHUNK * RECORD_SECONDS)):
            data = stream.read(CHUNK)
            frames.append(data)
        
        stream.stop_stream()
        stream.close()
        audio.terminate()
        
        # 4. Отправить аудио (PCM16)
        audio_data = b''.join(frames)
        print(f"Отправка аудио: {len(audio_data)} bytes")
        await websocket.send(audio_data)
        
        # 5. Получить ответы
        while True:
            try:
                message = await asyncio.wait_for(websocket.recv(), timeout=5.0)
                
                if isinstance(message, bytes):
                    print(f"Получено аудио: {len(message)} bytes")
                    # Сохранить или воспроизвести
                    with open("response_audio.opus", "wb") as f:
                        f.write(message)
                    print("Аудио сохранено в response_audio.opus")
                else:
                    data = json.loads(message)
                    msg_type = data.get("type")
                    
                    if msg_type == "stt":
                        print(f"Транскрипция: {data['text']}")
                    elif msg_type == "llm":
                        print(f"Ответ LLM: {data['text']}")
            except asyncio.TimeoutError:
                break

if __name__ == "__main__":
    asyncio.run(test_audio_send())
```

**Установка зависимостей:**
```bash
pip install websockets pyaudio
```

### 3. Использование готового аудио файла

Если у вас есть готовый Opus файл:

```python
import asyncio
import websockets
import json

async def send_audio_file():
    uri = "ws://localhost:8080/ws"
    
    async with websockets.connect(uri) as websocket:
        # Hello
        hello = {
            "type": "hello",
            "version": 3,
            "transport": "websocket",
            "features": {"aec": True, "mcp": False},
            "audio_params": {
                "format": "opus",
                "sample_rate": 48000,
                "channels": 1,
                "frame_duration": 20
            }
        }
        await websocket.send(json.dumps(hello))
        
        response = await websocket.recv()
        hello_response = json.loads(response)
        session_id = hello_response.get("session_id")
        print(f"Session ID: {session_id}")
        
        # Читаем и отправляем аудио файл
        with open("test_audio.opus", "rb") as f:
            audio_data = f.read()
        
        print(f"Отправка аудио файла: {len(audio_data)} bytes")
        await websocket.send(audio_data)
        
        # Получаем ответы
        while True:
            try:
                message = await asyncio.wait_for(websocket.recv(), timeout=5.0)
                
                if isinstance(message, bytes):
                    print(f"Получено аудио ответ: {len(message)} bytes")
                    with open("response.opus", "wb") as f:
                        f.write(message)
                else:
                    data = json.loads(message)
                    if data.get("type") == "stt":
                        print(f"Транскрипция: {data['text']}")
                    elif data.get("type") == "llm":
                        print(f"Ответ: {data['text']}")
            except asyncio.TimeoutError:
                break

asyncio.run(send_audio_file())
```

### 4. Использование curl для тестирования (только текст)

Для тестирования без аудио можно использовать отправку текста:

```bash
# Сначала нужно установить wscat или использовать другой инструмент
# WebSocket не поддерживается напрямую в curl
```

### 5. Postman WebSocket

1. Создайте новый WebSocket request в Postman
2. URL: `ws://localhost:8080/ws`
3. Отправьте hello JSON
4. Отправьте бинарные данные (аудио файл)
5. Получите ответы

## Формат аудио

### Требования:
- **Формат:** Opus (рекомендуется) или PCM16
- **Частота дискретизации:** 48000 Hz
- **Каналы:** 1 (моно)
- **Длительность фрейма:** 20 мс

### Конвертация аудио в Opus

**Используя ffmpeg:**
```bash
# Конвертировать WAV в Opus
ffmpeg -i input.wav -ar 48000 -ac 1 -c:a libopus -frame_duration 20 output.opus

# Конвертировать MP3 в Opus
ffmpeg -i input.mp3 -ar 48000 -ac 1 -c:a libopus -frame_duration 20 output.opus
```

**Используя Python (opuslib):**
```python
import opuslib

# Декодировать PCM в Opus
encoder = opuslib.Encoder(48000, 1, opuslib.APPLICATION_VOIP)
pcm_data = ...  # PCM16 данные
opus_data = encoder.encode(pcm_data, 960)  # 960 samples = 20ms at 48kHz
```

## Отладка

### Проблемы с доступом к микрофону

**В браузере:**
- Убедитесь, что используете HTTPS или localhost
- Проверьте настройки браузера для разрешений микрофона
- Chrome: `chrome://settings/content/microphone`

### Проблемы с воспроизведением

**Opus аудио не воспроизводится:**
- Браузер может не поддерживать Opus напрямую
- Используйте декодер Opus (opus.js, opus-decoder)
- Или конвертируйте в WAV перед воспроизведением

### Проверка отправки данных

**В HTML клиенте:**
- Откройте DevTools (F12)
- Вкладка Network → WS
- Проверьте отправленные и полученные сообщения

## Примеры тестовых сценариев

### Сценарий 1: Быстрый тест
1. Откройте `websocket_test.html`
2. Подключитесь и отправьте Hello
3. Нажмите "Отправить текст (Listen)"
4. Введите: "Привет, как дела?"
5. Получите ответы

### Сценарий 2: Полный тест с микрофоном
1. Откройте `websocket_test.html`
2. Подключитесь и отправьте Hello
3. Начните запись
4. Скажите: "Привет, как дела?"
5. Остановите запись
6. Отправьте аудио
7. Получите транскрипцию, ответ LLM и аудио ответ

### Сценарий 3: Тест с файлом
1. Подготовьте Opus файл (5-10 секунд)
2. Используйте Python скрипт для отправки
3. Проверьте получение всех ответов

## Рекомендации

1. **Для разработки:** Используйте HTML тест-клиент - самый простой способ
2. **Для автоматизации:** Используйте Python скрипты
3. **Для отладки:** Используйте Postman WebSocket с логированием
4. **Для продакшена:** Используйте ESP32 код из `docs/ROBOT_WEBSOCKET.md`

