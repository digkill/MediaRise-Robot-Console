#!/bin/bash
# Скрипт для тестирования WebSocket соединения

echo "=== Тест WebSocket соединения ==="
echo ""

# Проверяем, что сервер запущен
if ! curl -s http://localhost:8080/health > /dev/null; then
    echo "❌ Сервер не запущен на http://localhost:8080"
    echo "Запустите сервер: cargo run"
    exit 1
fi

echo "✅ Сервер запущен"
echo ""

# Проверяем WebSocket (простая проверка через wscat если установлен)
if command -v wscat &> /dev/null; then
    echo "Тестирование WebSocket через wscat..."
    echo "Отправьте hello сообщение:"
    echo '{"type":"hello","version":3,"transport":"websocket","features":{"aec":true,"mcp":false},"audio_params":{"format":"opus","sample_rate":48000,"channels":1,"frame_duration":20}}'
    echo ""
    wscat -c ws://localhost:8080/ws
else
    echo "⚠️ wscat не установлен. Установите: npm install -g wscat"
    echo ""
    echo "Или используйте HTML тест-клиент: insomnia/websocket_test.html"
fi

