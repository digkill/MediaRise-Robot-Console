# WebSocket протокол для робота

## Подключение

Робот должен подключаться к WebSocket endpoint:

```
ws://<SERVER_HOST>:<SERVER_PORT>/ws
```

Где:
- `SERVER_HOST` - адрес сервера (по умолчанию `0.0.0.0` или `localhost`)
- `SERVER_PORT` - порт сервера (по умолчанию `8080`)

Пример:
```
ws://localhost:8080/ws
```

## Протокол обмена

### 1. Подключение и Hello сообщение

После подключения к WebSocket, робот должен отправить `hello` сообщение:

```json
{
  "type": "hello",
  "version": 3,
  "transport": "websocket",
  "features": {
    "aec": true,
    "mcp": false
  },
  "audio_params": {
    "format": "opus",
    "sample_rate": 48000,
    "channels": 1,
    "frame_duration": 20
  },
  "session_id": null
}
```

**Параметры:**
- `version` - версия протокола (рекомендуется 3)
- `transport` - тип транспорта ("websocket")
- `features.aec` - поддержка Acoustic Echo Cancellation
- `features.mcp` - поддержка MCP протокола
- `audio_params.format` - формат аудио ("opus" или "pcm16")
- `audio_params.sample_rate` - частота дискретизации (48000)
- `audio_params.channels` - количество каналов (1 для моно)
- `frame_duration` - длительность фрейма в миллисекундах (20)

**Ответ сервера:**

Сервер отправит обратно `hello` сообщение с подтверждением и `session_id`:

```json
{
  "type": "hello",
  "version": 3,
  "transport": "websocket",
  "features": {
    "aec": true,
    "mcp": true
  },
  "audio_params": {
    "format": "opus",
    "sample_rate": 48000,
    "channels": 1,
    "frame_duration": 20
  },
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

### 2. Отправка аудио

После получения `hello` ответа, робот может отправлять аудио данные в **бинарном формате** (WebSocket Binary Message).

**Формат аудио:**
- Формат: Opus (рекомендуется) или PCM16
- Частота дискретизации: 48000 Hz
- Каналы: 1 (моно)
- Длительность фрейма: 20 мс

**Пример отправки Opus аудио:**

```cpp
// ESP32 пример
WebSocketsClient webSocket;
uint8_t opus_frame[OPUS_FRAME_SIZE]; // Opus закодированный фрейм

// Заполнить opus_frame данными от микрофона
// ...

// Отправить бинарные данные
webSocket.sendBIN(opus_frame, OPUS_FRAME_SIZE);
```

### 3. Получение транскрипции

После обработки аудио, сервер отправит транскрипцию в формате `stt` сообщения:

```json
{
  "type": "stt",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "text": "Привет, как дела?"
}
```

### 4. Получение ответа от LLM

После обработки транскрипции через LLM, сервер отправит ответ:

```json
{
  "type": "llm",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "emotion": null,
  "text": "Привет! У меня всё отлично, спасибо!"
}
```

### 5. Получение аудио ответа (TTS)

После генерации ответа LLM, сервер автоматически синтезирует речь и отправит **бинарные данные** (Opus аудио) через WebSocket Binary Message.

Робот должен декодировать Opus аудио и воспроизвести через динамик.

**Пример получения и декодирования:**

```cpp
// ESP32 пример
void webSocketEvent(WStype_t type, uint8_t * payload, size_t length) {
    switch(type) {
        case WStype_BIN:
            // Получены бинарные данные (Opus аудио)
            uint8_t* opus_audio = payload;
            size_t opus_size = length;
            
            // Декодировать Opus в PCM16
            int16_t pcm_samples[PCM_FRAME_SIZE];
            decode_opus_to_pcm(opus_audio, opus_size, pcm_samples);
            
            // Воспроизвести через динамик
            play_audio(pcm_samples, PCM_FRAME_SIZE);
            break;
            
        case WStype_TEXT:
            // Получено текстовое сообщение (JSON)
            handle_json_message((char*)payload);
            break;
    }
}
```

## Альтернативный способ: отправка текста напрямую

Если робот уже имеет транскрипцию (например, локальную), можно отправить текст напрямую:

### Вариант 1: Через `listen` сообщение

```json
{
  "type": "listen",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "state": "start",
  "mode": "manual",
  "text": "Привет, как дела?"
}
```

Сервер обработает текст через LLM и отправит аудио ответ.

### Вариант 2: Через `stt` сообщение

```json
{
  "type": "stt",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "text": "Привет, как дела?"
}
```

Сервер обработает через LLM и отправит текстовый ответ + аудио.

## Полный пример потока

```
1. Робот → Сервер: WebSocket подключение
2. Робот → Сервер: {"type": "hello", ...}
3. Сервер → Робот: {"type": "hello", "session_id": "..."}
4. Робот → Сервер: [Binary: Opus аудио фрейм 1]
5. Робот → Сервер: [Binary: Opus аудио фрейм 2]
6. Робот → Сервер: [Binary: Opus аудио фрейм N]
7. Сервер → Робот: {"type": "stt", "text": "Привет"}
8. Сервер → Робот: {"type": "llm", "text": "Привет! Как дела?"}
9. Сервер → Робот: [Binary: Opus аудио ответ]
```

## Завершение сессии

Для корректного завершения сессии отправьте:

```json
{
  "type": "goodbye",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

Или просто закройте WebSocket соединение.

## Обработка ошибок

Если произошла ошибка, сервер может отправить:

```json
{
  "type": "system",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "command": "error"
}
```

Или закрыть соединение.

## Примеры кода

### ESP32 (Arduino)

```cpp
#include <WebSocketsClient.h>
#include <ArduinoJson.h>

WebSocketsClient webSocket;

void setup() {
    webSocket.begin("192.168.1.100", 8080, "/ws");
    webSocket.onEvent(webSocketEvent);
    webSocket.setReconnectInterval(5000);
}

void loop() {
    webSocket.loop();
    
    // Отправка аудио каждые 20 мс
    if (millis() - lastAudioTime >= 20) {
        send_audio_frame();
        lastAudioTime = millis();
    }
}

void webSocketEvent(WStype_t type, uint8_t * payload, size_t length) {
    switch(type) {
        case WStype_CONNECTED:
            send_hello();
            break;
            
        case WStype_TEXT:
            handle_json_message((char*)payload);
            break;
            
        case WStype_BIN:
            handle_audio_response(payload, length);
            break;
            
        case WStype_DISCONNECTED:
            Serial.println("Disconnected");
            break;
    }
}

void send_hello() {
    StaticJsonDocument<512> doc;
    doc["type"] = "hello";
    doc["version"] = 3;
    doc["transport"] = "websocket";
    
    JsonObject features = doc.createNestedObject("features");
    features["aec"] = true;
    features["mcp"] = false;
    
    JsonObject audioParams = doc.createNestedObject("audio_params");
    audioParams["format"] = "opus";
    audioParams["sample_rate"] = 48000;
    audioParams["channels"] = 1;
    audioParams["frame_duration"] = 20;
    
    String json;
    serializeJson(doc, json);
    webSocket.sendTXT(json);
}

void send_audio_frame() {
    uint8_t opus_frame[OPUS_FRAME_SIZE];
    // Заполнить opus_frame данными от микрофона
    // ...
    webSocket.sendBIN(opus_frame, OPUS_FRAME_SIZE);
}

void handle_json_message(char* payload) {
    StaticJsonDocument<512> doc;
    deserializeJson(doc, payload);
    
    String type = doc["type"];
    
    if (type == "stt") {
        String text = doc["text"];
        Serial.print("Transcription: ");
        Serial.println(text);
    } else if (type == "llm") {
        String text = doc["text"];
        Serial.print("LLM Response: ");
        Serial.println(text);
    }
}

void handle_audio_response(uint8_t* payload, size_t length) {
    // Декодировать Opus и воспроизвести
    int16_t pcm[PCM_FRAME_SIZE];
    decode_opus_to_pcm(payload, length, pcm);
    play_audio(pcm, PCM_FRAME_SIZE);
}
```

### Python

```python
import asyncio
import websockets
import json
import audioop

async def robot_client():
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
        
        # 3. Отправка аудио и получение ответов
        async def send_audio():
            while True:
                # Получить аудио от микрофона
                opus_frame = get_audio_frame()  # Ваша функция
                await websocket.send(opus_frame)
                await asyncio.sleep(0.02)  # 20 мс
        
        async def receive_messages():
            while True:
                message = await websocket.recv()
                
                if isinstance(message, bytes):
                    # Бинарные данные - аудио ответ
                    play_audio(message)
                else:
                    # JSON сообщение
                    data = json.loads(message)
                    msg_type = data.get("type")
                    
                    if msg_type == "stt":
                        print(f"Transcription: {data['text']}")
                    elif msg_type == "llm":
                        print(f"LLM Response: {data['text']}")
        
        # Запустить обе задачи параллельно
        await asyncio.gather(
            send_audio(),
            receive_messages()
        )

asyncio.run(robot_client())
```

## Важные замечания

1. **Аудио формат**: Рекомендуется использовать Opus для экономии трафика
2. **Частота кадров**: Отправляйте аудио фреймы каждые 20 мс
3. **Сессия**: Сохраняйте `session_id` из ответа hello для всех последующих сообщений
4. **Переподключение**: При разрыве соединения переподключитесь и отправьте hello заново
5. **Таймауты**: Установите таймауты для WebSocket соединения (рекомендуется 60 секунд)

