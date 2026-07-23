-- Долговременная память робота на каждое устройство.
-- LLM сохраняет сюда факты через поле "remember" в своём JSON-ответе.

CREATE TABLE IF NOT EXISTS device_memory (
    id CHAR(36) PRIMARY KEY,
    device_id VARCHAR(128) NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_device_memory_device_id ON device_memory(device_id, created_at);
