# Hello сообщение - Формат и примеры

## Стандартный формат Hello сообщения

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

## Минимальный формат (все поля опциональны)

```json
{
  "type": "hello"
}
```

Сервер автоматически установит значения по умолчанию.

## Описание полей

### Обязательные поля:
- `type` - всегда `"hello"`

### Опциональные поля:

#### `version` (число)
- Версия протокола
- Рекомендуется: `3`
- По умолчанию: `3`

#### `transport` (строка)
- Тип транспорта
- Значение: `"websocket"`
- По умолчанию: `"websocket"`

#### `features` (объект)
- Поддерживаемые функции
- `aec` (boolean) - Acoustic Echo Cancellation
- `mcp` (boolean) - Model Context Protocol поддержка
- По умолчанию: `{"aec": false, "mcp": false}`

#### `audio_params` (объект)
- Параметры аудио
- `format` (строка) - формат: `"opus"` или `"pcm16"`
- `sample_rate` (число) - частота дискретизации: `48000`
- `channels` (число) - количество каналов: `1` (моно) или `2` (стерео)
- `frame_duration` (число) - длительность фрейма в миллисекундах: `20`
- По умолчанию: Opus, 48kHz, моно, 20ms

#### `session_id` (строка или null)
- ID существующей сессии (для переподключения)
- При первом подключении: `null`
- При переподключении: UUID сессии

## Примеры для разных случаев

### 1. Первое подключение (рекомендуется)

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

### 2. Минимальный вариант

```json
{
  "type": "hello"
}
```

### 3. С PCM16 аудио

```json
{
  "type": "hello",
  "version": 3,
  "audio_params": {
    "format": "pcm16",
    "sample_rate": 48000,
    "channels": 1,
    "frame_duration": 20
  }
}
```

### 4. С поддержкой MCP

```json
{
  "type": "hello",
  "version": 3,
  "features": {
    "aec": true,
    "mcp": true
  },
  "audio_params": {
    "format": "opus",
    "sample_rate": 48000,
    "channels": 1,
    "frame_duration": 20
  }
}
```

### 5. Переподключение к существующей сессии

```json
{
  "type": "hello",
  "version": 3,
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "audio_params": {
    "format": "opus",
    "sample_rate": 48000,
    "channels": 1,
    "frame_duration": 20
  }
}
```

## Ответ сервера

После отправки Hello, сервер отправит ответ:

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

**Важно:** Сохраните `session_id` из ответа для всех последующих сообщений!

## Как отправить в HTML тест-клиенте

1. Откройте `websocket_test.html`
2. Подключитесь к WebSocket
3. Выберите тип сообщения: "Hello (обязательно первым)"
4. В текстовом поле уже есть готовый JSON (можно использовать как есть)
5. Нажмите "Отправить Hello" или "Отправить JSON"

## Как отправить через wscat

```bash
wscat -c ws://localhost:8080/ws
```

Затем введите:
```json
{"type":"hello","version":3,"transport":"websocket","features":{"aec":true,"mcp":false},"audio_params":{"format":"opus","sample_rate":48000,"channels":1,"frame_duration":20}}
```

## Как отправить через Python

```python
import asyncio
import websockets
import json

async def send_hello():
    uri = "ws://localhost:8080/ws"
    
    async with websockets.connect(uri) as websocket:
        hello = {
            "type": "hello",
            "version": 3,
            "transport": "websocket",
            "features": {
                "aec": True,
                "mcp": False
            },
            "audio_params": {
                "format": "opus",
                "sample_rate": 48000,
                "channels": 1,
                "frame_duration": 20
            }
        }
        
        await websocket.send(json.dumps(hello))
        
        # Получить ответ
        response = await websocket.recv()
        data = json.loads(response)
        print(f"Session ID: {data.get('session_id')}")

asyncio.run(send_hello())
```

## Ошибки и решения

### Ошибка: "Received message before hello"
- **Причина:** Отправлено другое сообщение до Hello
- **Решение:** Всегда отправляйте Hello первым

### Ошибка: "No session created"
- **Причина:** Hello не был отправлен или не получен ответ
- **Решение:** Убедитесь, что получили ответ с session_id

### Ошибка: "Invalid JSON"
- **Причина:** Неправильный формат JSON
- **Решение:** Проверьте синтаксис JSON (запятые, кавычки)

## Рекомендации

1. **Всегда отправляйте Hello первым** после подключения
2. **Используйте полный формат** для явного указания параметров
3. **Сохраняйте session_id** из ответа для последующих сообщений
4. **При переподключении** используйте сохраненный session_id

