# Исправление проблемы с STT_API_KEY

## Проблема:
```
ERROR: STT API key not configured
```

## Причина:
Ключ `STT_API_KEY` есть в `.env` файле, но не загружается при старте сервера.

## Решение:

### 1. Проверьте формат `.env` файла

Убедитесь, что в `.env` файле нет пробелов вокруг `=`:

**Правильно:**
```env
STT_API_KEY=sk-proj-...
```

**Неправильно:**
```env
STT_API_KEY = sk-proj-...
STT_API_KEY= sk-proj-...
STT_API_KEY =sk-proj-...
```

### 2. Проверьте, что ключ не пустой

```bash
grep STT_API_KEY .env
```

Должно быть что-то вроде:
```
STT_API_KEY=sk-proj-...
```

### 3. Перезапустите сервер

После исправления `.env` файла перезапустите сервер:

```bash
RUST_LOG=info cargo run
```

### 4. Проверьте логи при старте

Вы должны увидеть:
```
INFO Loaded .env file
INFO Configuration loaded: STT provider=whisper, url=Some("https://api.openai.com/v1"), key_present=true
```

Если `key_present=false`, значит ключ все еще не загружается.

### 5. Альтернативный способ - установить переменную окружения

Если `.env` файл не работает, можно установить переменную напрямую:

```bash
export STT_API_KEY=sk-proj-...
cargo run
```

Или в одной команде:
```bash
STT_API_KEY=sk-proj-... cargo run
```

### 6. Проверка загрузки

После запуска сервера проверьте логи:
```bash
RUST_LOG=info cargo run 2>&1 | grep -i "stt\|config"
```

Должно быть:
```
INFO Loaded .env file
INFO Configuration loaded: STT provider=whisper, url=Some("https://api.openai.com/v1"), key_present=true
```

## Если проблема сохраняется:

1. Проверьте, что `.env` файл находится в корне проекта (там же, где `Cargo.toml`)
2. Проверьте права доступа к файлу: `ls -la .env`
3. Попробуйте пересоздать `.env` файл из `.env.example`
4. Убедитесь, что нет скрытых символов в ключе (копируйте ключ заново)

