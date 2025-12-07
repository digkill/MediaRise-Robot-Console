# WebSocket Endpoint для голоса и аудио

## Endpoint

```
ws://localhost:8080/ws
```

или с переменной окружения:
```
ws://{{ base_url }}/ws
```

## Процесс работы

### 1. Подключение и Hello

**Подключитесь к WebSocket** и отправьте hello сообщение:

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
  }
}
```

**Ответ сервера:**
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

**Сохраните `session_id`** для всех последующих сообщений.

### 2. Отправка голоса (аудио)

Отправьте **бинарные данные** (WebSocket Binary Message) с Opus аудио:

- Формат: Opus
- Частота: 48000 Hz
- Каналы: 1 (моно)
- Длительность фрейма: 20 мс

**Пример (ESP32):**
```cpp
uint8_t opus_frame[OPUS_FRAME_SIZE];
// Заполнить opus_frame данными от микрофона
webSocket.sendBIN(opus_frame, OPUS_FRAME_SIZE);
```

### 3. Получение транскрипции (текст)

Сервер отправит транскрипцию в формате JSON:

```json
{
  "type": "stt",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "text": "Привет, как дела?"
}
```

### 4. Получение ответа LLM (текст)

После обработки транскрипции через LLM, сервер отправит ответ:

```json
{
  "type": "llm",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "emotion": null,
  "text": "Привет! У меня всё отлично, спасибо!"
}
```

### 5. Получение аудио ответа (голос)

Сервер автоматически синтезирует речь и отправит **бинарные данные** (Opus аудио):

- Формат: Opus
- Частота: 48000 Hz
- Каналы: 1 (моно)

**Пример получения (ESP32):**
```cpp
void webSocketEvent(WStype_t type, uint8_t * payload, size_t length) {
    if (type == WStype_BIN) {
        // Декодировать Opus в PCM16
        int16_t pcm_samples[PCM_FRAME_SIZE];
        decode_opus_to_pcm(payload, length, pcm_samples);
        
        // Воспроизвести через динамик
        play_audio(pcm_samples, PCM_FRAME_SIZE);
    }
}
```

## Альтернатива: отправка текста напрямую

Если у вас уже есть транскрипция, можно отправить текст напрямую через `listen` сообщение:

```json
{
  "type": "listen",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "state": "start",
  "mode": "manual",
  "text": "Привет, как дела?"
}
```

Сервер обработает текст через LLM и отправит:
1. Ответ LLM (текст)
2. Аудио ответ (бинарные данные Opus)

## Полный поток

```
1. Подключение → ws://localhost:8080/ws
2. Отправка → {"type": "hello", ...}
3. Получение → {"type": "hello", "session_id": "..."}
4. Отправка → [Binary: Opus аудио фрейм 1]
5. Отправка → [Binary: Opus аудио фрейм 2]
6. Отправка → [Binary: Opus аудио фрейм N]
7. Получение → {"type": "stt", "text": "Привет"}
8. Получение → {"type": "llm", "text": "Привет! Как дела?"}
9. Получение → [Binary: Opus аудио ответ]
```

## Инструменты для тестирования

### 1. HTML тест-клиент

Откройте `websocket_test.html` в браузере:
- Подключение к WebSocket
- Отправка hello
- Отправка текста через Listen
- Получение ответов (текст и аудио)

### 2. wscat (командная строка)

```bash
npm install -g wscat
wscat -c ws://localhost:8080/ws
```

### 3. Python скрипт

См. примеры в `docs/ROBOT_WEBSOCKET.md`

### 4. Postman

Postman поддерживает WebSocket:
1. Создайте новый WebSocket request
2. URL: `ws://localhost:8080/ws`
3. Отправляйте JSON и бинарные данные

## Важные замечания

1. **Insomnia не поддерживает WebSocket** - используйте другие инструменты
2. **Аудио формат**: Рекомендуется Opus для экономии трафика
3. **Частота кадров**: Отправляйте аудио фреймы каждые 20 мс
4. **Сессия**: Сохраняйте `session_id` из ответа hello
5. **Переподключение**: При разрыве соединения переподключитесь и отправьте hello заново

## Примеры кода

См. подробные примеры в `docs/ROBOT_WEBSOCKET.md`:
- ESP32 (Arduino)
- Python
- JavaScript/Node.js

