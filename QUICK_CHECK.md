# Быстрая проверка WebSocket

## 1. Запустите сервер с логированием:

```bash
RUST_LOG=info cargo run
```

## 2. Откройте HTML тест-клиент:

Откройте `insomnia/websocket_test.html` в браузере

## 3. Проверьте логи при отправке:

### При отправке Hello:
```
INFO New WebSocket connection
INFO Received hello message: ...
INFO Session created: ...
```

### При отправке текста через Listen:
```
INFO Processing listen text: 'ваш текст'
INFO Calling LLM service with 1 messages
INFO POST https://api.x.ai/v1/chat/completions
INFO Grok API response status: 200
INFO LLM response received: 'ответ'
INFO Sending LLM message: {"type":"llm",...}
INFO LLM message sent successfully
```

## 4. Если не видите логов:

1. Проверьте `.env` файл:
   ```bash
   grep GROK_API_KEY .env
   ```

2. Проверьте, что API ключ не пустой

3. Проверьте консоль браузера (F12) на наличие ошибок

4. Проверьте вкладку Network → WS в DevTools

## 5. Типичные ошибки:

- `"Grok API key is not configured"` → Проверьте `GROK_API_KEY` в `.env`
- `"Grok API error: 401"` → Неверный API ключ
- `"Failed to send LLM message"` → Проблема с WebSocket соединением
- `"Empty LLM response"` → LLM вернул пустой ответ

## 6. Если все логи есть, но сообщения не приходят:

1. Проверьте консоль браузера - там должны быть логи получения сообщений
2. Проверьте, что WebSocket соединение активно (не закрыто)
3. Попробуйте переподключиться

