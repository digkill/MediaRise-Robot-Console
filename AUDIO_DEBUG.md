# Отладка обработки аудио

## Что было добавлено:

1. **Подробное логирование** на каждом этапе обработки аудио
2. **Логирование ошибок** с деталями
3. **Отправка сообщений об ошибках** клиенту через WebSocket

## Как проверить:

### 1. Запустите сервер с логированием:

```bash
RUST_LOG=info cargo run
```

### 2. Отправьте аудио через HTML клиент

### 3. Проверьте логи сервера

При успешной обработке вы должны увидеть:

```
INFO Received binary audio data: 29363 bytes
INFO Audio processor not initialized, sending raw audio directly to STT
INFO === Starting raw audio processing ===
INFO Audio data size: 29363 bytes (may be WebM/Opus from browser)
INFO Sending audio to STT service...
INFO Transcribing audio: 29363 bytes, provider: whisper
INFO Sending audio to OpenAI Whisper API: 29363 bytes
INFO POST https://api.openai.com/v1/audio/transcriptions
INFO OpenAI STT API response status: 200
INFO ✅ Transcribed text: 'ваш текст'
INFO ✅ STT transcription successful: 'ваш текст'
INFO Processing STT result through LLM pipeline...
INFO Processing STT text: 'ваш текст'
INFO Sending STT message: {"type":"stt",...}
INFO STT message sent successfully
INFO Calling LLM service with 1 messages
INFO LLM response received: 'ответ'
INFO Sending LLM message: {"type":"llm",...}
INFO LLM message sent successfully
INFO ✅ Successfully processed STT result through LLM
INFO === Raw audio processing completed ===
```

## Возможные проблемы:

### Проблема 1: "STT API URL not configured" или "STT API key not configured"

**Решение:**
Проверьте `.env` файл:
```bash
grep STT .env
```

Должны быть:
```env
STT_PROVIDER=whisper
STT_API_URL=https://api.openai.com/v1
STT_API_KEY=your_openai_api_key
```

### Проблема 2: "STT API error: 401"

**Причина:** Неверный API ключ OpenAI

**Решение:**
- Проверьте правильность `STT_API_KEY`
- Убедитесь, что ключ имеет доступ к Whisper API

### Проблема 3: "Failed to process STT result"

**Причина:** Ошибка при обработке через LLM

**Решение:**
- Проверьте логи LLM (см. `DEBUG_LLM.md`)
- Проверьте `GROK_API_KEY` в `.env`

### Проблема 4: "Empty transcription result"

**Причина:** STT не смог распознать речь

**Решение:**
- Проверьте качество аудио
- Убедитесь, что в аудио есть речь
- Попробуйте записать заново

## Быстрая диагностика:

### 1. Проверьте конфигурацию:

```bash
# Проверьте STT настройки
grep -E "STT_PROVIDER|STT_API_URL|STT_API_KEY" .env

# Проверьте LLM настройки
grep -E "GROK_API_KEY|GROK_API_URL" .env
```

### 2. Проверьте логи при отправке:

```bash
# Запустите сервер и отправьте аудио
RUST_LOG=info cargo run 2>&1 | tee server.log

# Затем проверьте логи
grep -E "audio|STT|LLM|error|✅|❌" server.log
```

### 3. Проверьте консоль браузера:

В консоли браузера (F12 → Console) должны появиться:
- `📝 Транскрипция (STT): "..."`
- `🤖 Ответ LLM: "..."`

## Ожидаемый поток:

1. ✅ Аудио получено сервером
2. ✅ Аудио отправлено на STT (OpenAI Whisper)
3. ✅ Получена транскрипция
4. ✅ Транскрипция отправлена клиенту
5. ✅ Транскрипция обработана через LLM
6. ✅ LLM ответ отправлен клиенту
7. ✅ TTS аудио отправлено клиенту

Если какой-то шаг не выполняется - проверьте соответствующий раздел выше.

