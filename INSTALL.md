# Установка MediaRise Robot Console

Этот документ описывает процесс установки MediaRise Robot Console как системного сервиса (демона).

## Требования

- Rust и Cargo (установите с https://rustup.rs/)
- Linux с systemd или macOS
- Права root/sudo для установки

## Быстрая установка

```bash
# Клонируйте репозиторий или перейдите в директорию проекта
cd /path/to/mediarise-robot-console

# Запустите установочный скрипт с правами root
sudo ./install.sh
```

## Что делает скрипт установки

1. **Проверяет зависимости** (Rust, systemd/launchctl)
2. **Создает пользователя сервиса** (только Linux)
3. **Компилирует проект** в release режиме
4. **Создает директории**:
   - `/opt/mediarise-robot-console/` - основная директория
   - `/opt/mediarise-robot-console/bin/` - исполняемый файл
   - `/opt/mediarise-robot-console/storage/` - хранилище файлов
   - `/opt/mediarise-robot-console/logs/` - логи
5. **Устанавливает сервис**:
   - Linux: systemd service
   - macOS: LaunchDaemon

## Настройка

После установки настройте файл `.env`:

```bash
sudo nano /opt/mediarise-robot-console/.env
```

Обязательно настройте:
- `DATABASE_URL` - URL базы данных (MySQL, PostgreSQL или SQLite)
- `GROK_API_KEY` - API ключ для Grok LLM
- `STT_API_KEY` - API ключ для OpenAI Whisper (STT)
- `TTS_API_KEY` - API ключ для OpenAI TTS

## Управление сервисом

### Linux (systemd)

```bash
# Запустить сервис
sudo systemctl start mediarise-robot-console

# Остановить сервис
sudo systemctl stop mediarise-robot-console

# Перезапустить сервис
sudo systemctl restart mediarise-robot-console

# Проверить статус
sudo systemctl status mediarise-robot-console

# Просмотр логов
sudo journalctl -u mediarise-robot-console -f

# Включить автозапуск при загрузке системы
sudo systemctl enable mediarise-robot-console

# Отключить автозапуск
sudo systemctl disable mediarise-robot-console
```

### macOS (LaunchDaemon)

```bash
# Запустить сервис
sudo launchctl load /Library/LaunchDaemons/com.mediarise.robot-console.plist

# Остановить сервис
sudo launchctl unload /Library/LaunchDaemons/com.mediarise.robot-console.plist

# Проверить статус
launchctl list | grep com.mediarise.robot-console

# Просмотр логов
tail -f /opt/mediarise-robot-console/logs/stdout.log
tail -f /opt/mediarise-robot-console/logs/stderr.log
```

## Обновление

Для обновления сервиса:

```bash
# Остановите сервис
sudo systemctl stop mediarise-robot-console  # Linux
# или
sudo launchctl unload /Library/LaunchDaemons/com.mediarise.robot-console.plist  # macOS

# Обновите код (git pull, etc.)

# Пересоберите и переустановите
sudo ./install.sh

# Запустите сервис
sudo systemctl start mediarise-robot-console  # Linux
# или
sudo launchctl load /Library/LaunchDaemons/com.mediarise.robot-console.plist  # macOS
```

## Удаление

```bash
# Остановите и отключите сервис
sudo systemctl stop mediarise-robot-console
sudo systemctl disable mediarise-robot-console
sudo rm /etc/systemd/system/mediarise-robot-console.service
sudo systemctl daemon-reload

# Удалите файлы
sudo rm -rf /opt/mediarise-robot-console

# Удалите пользователя (Linux)
sudo userdel mediarise
```

## Устранение неполадок

### Сервис не запускается

1. Проверьте логи:
   ```bash
   # Linux
   sudo journalctl -u mediarise-robot-console -n 50
   
   # macOS
   tail -50 /opt/mediarise-robot-console/logs/stderr.log
   ```

2. Проверьте права доступа:
   ```bash
   ls -la /opt/mediarise-robot-console/
   ```

3. Проверьте конфигурацию `.env`:
   ```bash
   sudo cat /opt/mediarise-robot-console/.env
   ```

### Проблемы с базой данных

Убедитесь, что:
- База данных запущена
- `DATABASE_URL` правильно настроен
- Пользователь базы данных имеет необходимые права

### Проблемы с портами

По умолчанию сервер использует порт 8080. Если порт занят:
1. Измените `SERVER_PORT` в `.env`
2. Перезапустите сервис

## Ручная установка (без скрипта)

Если вы предпочитаете установить вручную:

1. Соберите проект:
   ```bash
   cargo build --release
   ```

2. Скопируйте файлы:
   ```bash
   sudo mkdir -p /opt/mediarise-robot-console/bin
   sudo cp target/release/mediarise-robot-console /opt/mediarise-robot-console/bin/
   sudo cp .env /opt/mediarise-robot-console/
   ```

3. Установите service файл:
   ```bash
   # Linux
   sudo cp systemd/mediarise-robot-console.service /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl enable mediarise-robot-console
   
   # macOS
   sudo cp macos/com.mediarise.robot-console.plist /Library/LaunchDaemons/
   sudo launchctl load /Library/LaunchDaemons/com.mediarise.robot-console.plist
   ```

## Безопасность

- Сервис запускается от отдельного пользователя `mediarise` (Linux)
- Файл `.env` имеет права 600 (только владелец может читать)
- Используются ограничения systemd/LaunchDaemon для безопасности

## Поддержка

При возникновении проблем:
1. Проверьте логи сервиса
2. Проверьте конфигурацию `.env`
3. Убедитесь, что все зависимости установлены

