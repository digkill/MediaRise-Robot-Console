# Архитектура Xiaozhi Backend

## Обзор

Backend сервер на Rust, который заменяет оригинальный китайский сервер и предоставляет все необходимые функции для работы с устройствами Xiaozhi ESP32.

## Основные компоненты

### 1. HTTP Server (Axum)

- **OTA Endpoints**: Проверка версии, активация устройств
- **Assets Endpoints**: Загрузка ресурсов
- **Upload Endpoints**: Загрузка скриншотов и других файлов

### 2. WebSocket Server

- Реальновременная коммуникация с устройствами
- Поддержка бинарных протоколов (версии 1, 2, 3)
- Обработка JSON сообщений
- Управление сессиями

### 3. Services Layer

#### Device Service
- Управление устройствами в базе данных
- Регистрация и активация
- Отслеживание версий прошивки

#### Session Service
- Управление WebSocket сессиями
- Отслеживание активности устройств

#### Audio Service
- Обработка аудио потоков
- Opus encode/decode
- Поддержка AEC

#### STT Service
- Speech-to-Text через внешние API
- Поддержка различных провайдеров (Whisper, OpenAI и т.д.)

#### TTS Service
- Text-to-Speech через внешние API
- Поддержка различных провайдеров

#### LLM Service
- Интеграция с Grok API
- Обработка чат-запросов
- Управление контекстом разговора

### 4. MCP Server

- Реализация Model Context Protocol
- JSON-RPC 2.0 обработка
- Управление инструментами устройств

### 5. Storage Layer

#### Database
- PostgreSQL или SQLite
- Хранение устройств, сессий, версий прошивки

#### File Storage
- Хранение прошивок
- Хранение ресурсов
- Хранение загруженных файлов

## Потоки данных

### 1. Подключение устройства

```
Device → WebSocket Connect → Server
Device → Hello Message → Server
Server → Hello Response → Device
Server → Session Created
```

### 2. Голосовое взаимодействие

```
Device → Audio Stream (Opus) → Server
Server → STT Service → Text
Server → LLM Service (Grok) → Response
Server → TTS Service → Audio (Opus)
Server → Audio Stream → Device
```

### 3. OTA обновление

```
Device → GET /ota/ → Server
Server → Check Version → Database
Server → Response (firmware info) → Device
Device → Download Firmware → Server
```

## Безопасность

- JWT токены для аутентификации
- HMAC для активации устройств
- TLS для WebSocket и HTTP
- Валидация всех входящих данных

## Масштабирование

- Stateless серверы (можно запускать несколько инстансов)
- База данных как единая точка истины
- Возможность использования Redis для сессий (в будущем)
- Load balancing через nginx или аналоги

## TODO

- [ ] Реализация всех TODO в коде
- [ ] Полная интеграция с Grok API
- [ ] STT/TTS интеграция
- [ ] MCP сервер полная реализация
- [ ] Тесты
- [ ] Документация API
- [ ] Docker контейнер
- [ ] CI/CD pipeline

