# Insomnia Collection для Xiaozhi Backend API

Эта коллекция содержит все необходимые запросы для тестирования Xiaozhi Backend Server.

## Установка

1. Откройте Insomnia
2. Нажмите `Create` → `Import/Export` → `Import Data` → `From File`
3. Выберите файл `Xiaozhi-API.json`

## Использование

### Настройка окружения

После импорта настройте переменные окружения в Insomnia:

1. Откройте `Manage Environments` (Ctrl+E / Cmd+E)
2. Убедитесь, что выбрана среда `Base Environment`
3. Настройте переменные:
   - `base_url` - адрес сервера (по умолчанию: `http://localhost:8080`)
   - `device_id` - ID устройства для тестирования
   - `client_id` - ID клиента
   - `serial_number` - серийный номер устройства
   - `firmware_version` - версия прошивки
   - `jwt_secret` - секретный ключ для JWT (из .env)
   - `hmac_key` - ключ для HMAC (из .env)

### Endpoints

#### 1. Health Check
- **GET** `/health`
- Проверка работоспособности сервера
- Ожидаемый ответ: `OK`

#### 2. OTA Endpoints

##### Check Version
- **POST** `/ota/`
- Проверка версии прошивки и получение информации о доступных обновлениях
- **Body:**
  ```json
  {
    "serial_number": "SN123456789",
    "firmware_version": "1.0.0",
    "client_id": "test-client-456"
  }
  ```
- **Response:**
  ```json
  {
    "version": "1.0.1",
    "url": "http://example.com/firmware.bin",
    "force": 0,
    "challenge": "random-challenge-string",
    "ws_url": "ws://localhost:8080/ws",
    "ws_token": "jwt-token-here"
  }
  ```

##### Activate Device
- **POST** `/ota/activate`
- Активация устройства
- **Body:**
  ```json
  {
    "serial_number": "SN123456789",
    "challenge": "challenge-from-check-version",
    "response": "hmac-sha256(challenge, hmac_key)"
  }
  ```
- **Примечание:** `response` должен быть вычислен как HMAC-SHA256 от `challenge` с использованием `hmac_key`

#### 3. Assets Endpoints

##### Download Assets
- **GET** `/assets/{version}`
- Загрузка ресурсов для указанной версии
- **URL параметры:**
  - `version` - версия ресурсов (например, `1.0.0`)
- **Response:** Бинарный файл или 404 если версия не найдена

#### 4. Upload Endpoints

##### Upload Screenshot
- **POST** `/upload/screenshot`
- Загрузка скриншота экрана устройства
- **Body:** `multipart/form-data`
  - `file` - файл изображения (PNG, JPEG)
- **Response:**
  ```json
  {
    "success": true,
    "url": "/storage/uploads/uuid.png"
  }
  ```

## WebSocket Endpoint (Голос/Аудио)

⚠️ **Важно:** Insomnia не поддерживает WebSocket напрямую!

**Endpoint:** `ws://localhost:8080/ws`

**Для тестирования WebSocket используйте:**

1. **HTML тест-клиент** (рекомендуется):
   - Откройте `websocket_test.html` в браузере
   - Подключитесь к WebSocket
   - Отправляйте hello, текст, получайте ответы

2. **wscat** (командная строка):
   ```bash
   npm install -g wscat
   wscat -c ws://localhost:8080/ws
   ```

3. **Postman** - имеет встроенную поддержку WebSocket

4. **Python скрипт** (см. `docs/ROBOT_WEBSOCKET.md`)

### Процесс работы с голосом:

1. **Подключение** → `ws://localhost:8080/ws`
2. **Hello** → Отправить JSON с hello сообщением
3. **Отправка голоса** → Отправить бинарные данные (Opus аудио)
4. **Получение транскрипции** → Получить JSON с текстом (stt)
5. **Получение ответа LLM** → Получить JSON с ответом (llm)
6. **Получение аудио ответа** → Получить бинарные данные (Opus)

**См. подробную инструкцию:** 
- `WEBSOCKET_GUIDE.md` - общий гайд по WebSocket
- `AUDIO_TESTING.md` - **как тестировать отправку аудио** ⭐

### Пример WebSocket теста с wscat

```bash
# Подключение
wscat -c ws://localhost:8080/ws

# Отправка hello сообщения
{"type":"hello","version":3,"transport":"websocket","features":{"aec":true,"mcp":false},"audio_params":{"format":"opus","sample_rate":48000,"channels":1,"frame_duration":20}}

# Ожидание ответа hello с session_id

# Отправка listen сообщения
{"type":"listen","session_id":"<session_id>","state":"start","text":"Привет, как дела?"}

# Ожидание ответов: stt, llm, и бинарных данных (аудио)
```

## Примеры тестовых сценариев

### Сценарий 1: Полный цикл OTA

1. **Health Check** - убедиться что сервер работает
2. **Check Version** - проверить версию прошивки
3. Вычислить HMAC для challenge
4. **Activate Device** - активировать устройство

### Сценарий 2: Загрузка ресурсов

1. **Check Version** - получить версию
2. **Download Assets** - загрузить ресурсы для версии

### Сценарий 3: Загрузка скриншота

1. Подготовить тестовое изображение
2. **Upload Screenshot** - загрузить скриншот
3. Проверить ответ с URL загруженного файла

## Вычисление HMAC для активации

Для тестирования активации устройства нужно вычислить HMAC-SHA256:

### Python
```python
import hmac
import hashlib

challenge = "challenge-from-server"
hmac_key = "FCDEfd3_fde3d3fcelcvmfdjk646cfe32"

response = hmac.new(
    hmac_key.encode(),
    challenge.encode(),
    hashlib.sha256
).hexdigest()

print(response)
```

### JavaScript/Node.js
```javascript
const crypto = require('crypto');

const challenge = 'challenge-from-server';
const hmacKey = 'FCDEfd3_fde3d3fcelcvmfdjk646cfe32';

const response = crypto
  .createHmac('sha256', hmacKey)
  .update(challenge)
  .digest('hex');

console.log(response);
```

### Online инструмент
Используйте https://www.freeformatter.com/hmac-generator.html
- Algorithm: SHA256
- Secret Key: ваш `hmac_key`
- Text to Hash: `challenge` из ответа Check Version

## Troubleshooting

### Ошибка 404
- Убедитесь, что сервер запущен
- Проверьте `base_url` в переменных окружения
- Проверьте правильность пути endpoint

### Ошибка 500
- Проверьте логи сервера
- Убедитесь, что база данных настроена и доступна
- Проверьте переменные окружения в `.env`

### WebSocket не подключается
- Убедитесь, что сервер запущен на правильном порту
- Проверьте, что используется `ws://` (не `http://`)
- Проверьте firewall настройки

## Дополнительные ресурсы

- [Документация WebSocket протокола](docs/ROBOT_WEBSOCKET.md)
- [Документация MCP протокола](docs/MCP.md)
- [Настройка MQTT](docs/MQTT_SETUP.md)

