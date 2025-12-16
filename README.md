# MediaRise Robot Console Backend Server

Backend сервер на Rust для управления устройствами Xiaozhi ESP32 и не только

## Архитектура

Сервер реализует все функции оригинального китайского backend:

- ✅ HTTP API для OTA обновлений
- ✅ WebSocket сервер для реального времени коммуникации
- ✅ MQTT поддержка (опционально)
- ✅ Обработка аудио (Opus encode/decode)
- ✅ Интеграция с Grok API для LLM
- ✅ STT/TTS интеграция
- ✅ MCP (Model Context Protocol) сервер
- ✅ Управление устройствами и сессиями
- ✅ Файловое хранилище

## Структура проекта

```
xiaozhi-backend/
├── src/
│   ├── main.rs                 # Точка входа
│   ├── config.rs               # Конфигурация
│   ├── server.rs                # HTTP/WebSocket сервер
│   ├── handlers/                # HTTP handlers
│   │   ├── ota.rs              # OTA endpoints
│   │   ├── assets.rs           # Assets endpoints
│   │   └── upload.rs           # Upload endpoints
│   ├── websocket/              # WebSocket обработка
│   │   ├── mod.rs
│   │   ├── session.rs          # Управление сессиями
│   │   ├── protocol.rs         # Протокол WebSocket
│   │   └── audio.rs            # Обработка аудио
│   ├── mqtt/                   # MQTT поддержка (опционально)
│   │   ├── mod.rs
│   │   └── handler.rs
│   ├── mcp/                    # MCP сервер
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   └── tools.rs
│   ├── services/               # Бизнес-логика
│   │   ├── device.rs           # Управление устройствами
│   │   ├── session.rs          # Управление сессиями
│   │   ├── audio.rs            # Обработка аудио
│   │   ├── stt.rs              # Speech-to-Text
│   │   ├── tts.rs              # Text-to-Speech
│   │   └── llm.rs              # LLM (Grok)
│   ├── storage/                # Хранилище
│   │   ├── mod.rs
│   │   ├── database.rs         # База данных
│   │   └── files.rs            # Файловое хранилище
│   └── utils/                  # Утилиты
│       ├── mod.rs
│       ├── crypto.rs           # Криптография
│       └── audio.rs            # Аудио утилиты
├── migrations/                 # SQL миграции
├── config/                     # Конфигурационные файлы
│   └── default.toml
├── .env.example               # Пример переменных окружения
└── Cargo.toml
```

## Установка и запуск

### Быстрая установка как системный сервис

Для установки сервера как демона (systemd на Linux или LaunchDaemon на macOS):

```bash
sudo ./install.sh
```

Подробная инструкция: [INSTALL.md](INSTALL.md)

### Ручная установка (для разработки)

#### Требования

- Rust 1.70+
- PostgreSQL, MySQL или SQLite (для базы данных)
- Grok API ключ
- OpenAI API ключи (для STT/TTS)

#### Настройка

1. Скопируйте `.env.example` в `.env` и заполните:

```bash
cp .env.example .env
```

2. Настройте переменные окружения:

```env
# Server
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
WEBSOCKET_PORT=8081

# Database
DATABASE_URL=postgresql://user:password@localhost/xiaozhi
# или для SQLite:
# DATABASE_URL=sqlite:xiaozhi.db

# Grok API
GROK_API_KEY=your_grok_api_key
GROK_API_URL=https://api.x.ai/v1
GROK_MODEL=grok-4
GROK_SYSTEM_PROMPT=You are Grok, a highly intelligent, helpful AI assistant.

# STT/TTS (настройте по необходимости)
STT_PROVIDER=whisper
STT_API_URL=https://api.openai.com/v1   # сервер сам добавит /audio/transcriptions
STT_API_KEY=your_key

TTS_PROVIDER=openai
TTS_API_URL=https://api.openai.com/v1   # сервер сам добавит /audio/speech
TTS_API_KEY=your_key

# Если STT_API_KEY не указан, будет использован TTS_API_KEY

# File storage
STORAGE_PATH=./storage
FIRMWARE_PATH=./storage/firmware
ASSETS_PATH=./storage/assets
UPLOADS_PATH=./storage/uploads

# Security
JWT_SECRET=your_jwt_secret_key
HMAC_KEY=your_hmac_key_for_device_activation

# MQTT (опционально)
MQTT_ENABLED=false
MQTT_BROKER=mqtt://localhost:1883
```

3. Запустите миграции базы данных:

```bash
sqlx migrate run
```

4. Запустите сервер:

```bash
cargo run --release
# Для детализированных логов всех WebSocket/STT/TTS событий:
# RUST_LOG=info cargo run
```

## API Endpoints

### OTA Endpoints

- `GET/POST /ota/` - Проверка версии и получение конфигурации
- `POST /ota/activate` - Активация устройства

### Assets Endpoints

- `GET /assets/{version}` - Загрузка ресурсов
- `POST /assets/upload` - Загрузка новых ресурсов (admin)

### Upload Endpoints

- `POST /upload/screenshot` - Загрузка скриншота экрана

### WebSocket

- `ws://localhost:8080/ws` - WebSocket endpoint для устройств

## Протоколы

### WebSocket Protocol

**Для разработчиков роботов:** См. [ROBOT_WEBSOCKET.md](docs/ROBOT_WEBSOCKET.md) - подробная инструкция как подключиться и отправлять аудио для получения транскрипции и ответа от LLM.

**Для разработчиков сервера:** См. документацию в `docs/websocket.md` для деталей протокола.

### MCP Protocol

См. документацию в `docs/mcp.md` для деталей MCP протокола.

## Тестирование

### Insomnia Collection

Для тестирования API доступна готовая коллекция Insomnia:

1. Импортируйте файл `insomnia/Xiaozhi-API.json` в Insomnia
2. Настройте переменные окружения (base_url, device_id, etc.)
3. См. подробную инструкцию в `insomnia/README.md`

### WebSocket тестирование

Для тестирования WebSocket используйте:
- HTML тест-клиент: откройте `insomnia/websocket_test.html` в браузере
- Python скрипт: см. примеры в `docs/ROBOT_WEBSOCKET.md`
- wscat: `wscat -c ws://localhost:8080/ws`

## Разработка

### Запуск в режиме разработки

```bash
cargo run
```

### Тесты

```bash
cargo test
```

### Линтинг

```bash
cargo clippy
```

### Форматирование

```bash
cargo fmt
```

## Лицензия

MIT
