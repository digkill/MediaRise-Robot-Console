# Настройка MQTT брокера

MQTT (Message Queuing Telemetry Transport) - это легковесный протокол обмена сообщениями, идеально подходящий для IoT устройств.

## Установка MQTT брокера

### Вариант 1: Mosquitto (рекомендуется)

#### macOS (Homebrew)
```bash
brew install mosquitto
brew services start mosquitto
```

#### Linux (Ubuntu/Debian)
```bash
sudo apt-get update
sudo apt-get install mosquitto mosquitto-clients
sudo systemctl start mosquitto
sudo systemctl enable mosquitto
```

#### Linux (CentOS/RHEL)
```bash
sudo yum install epel-release
sudo yum install mosquitto
sudo systemctl start mosquitto
sudo systemctl enable mosquitto
```

#### Docker
```bash
docker run -it -p 1883:1883 -p 9001:9001 eclipse-mosquitto
```

### Вариант 2: EMQX (для продакшена)

```bash
# Docker
docker run -d --name emqx -p 1883:1883 -p 8083:8083 -p 8084:8084 -p 8883:8883 -p 18083:18083 emqx/emqx:latest
```

### Вариант 3: HiveMQ (коммерческий, есть бесплатная версия)

Скачайте с [hivemq.com](https://www.hivemq.com/downloads/)

## Проверка работы брокера

### Проверка, что брокер запущен
```bash
# macOS/Linux
ps aux | grep mosquitto

# Или проверка порта
netstat -an | grep 1883
# или
lsof -i :1883
```

### Тестирование подключения

#### Подписка на топик (в одном терминале)
```bash
mosquitto_sub -h localhost -t "xiaozhi/+/command" -v
```

#### Публикация сообщения (в другом терминале)
```bash
mosquitto_pub -h localhost -t "xiaozhi/device/test/command" -m "Hello from MQTT"
```

## Настройка в проекте

### 1. Включите MQTT feature при сборке

```bash
cargo build --features mqtt
# или
cargo run --features mqtt
```

### 2. Настройте переменные окружения в `.env`

```env
# Включить MQTT
MQTT_ENABLED=true

# Адрес брокера (можно указать с портом или без)
MQTT_BROKER=localhost:1883
# или просто
MQTT_BROKER=localhost

# Идентификатор клиента
MQTT_CLIENT_ID=mediarise-robot-console
```

### 3. Формат URL брокера

Поддерживаются следующие форматы:
- `localhost:1883` (рекомендуется)
- `localhost` (использует порт 1883 по умолчанию)
- `mqtt://localhost:1883`
- `192.168.1.100:1883` (удаленный брокер)

## Структура топиков

Проект использует следующую структуру топиков:

```
xiaozhi/
  ├── device/
  │   ├── {device_id}/
  │   │   ├── command      # Команды для устройства
  │   │   └── status       # Статус устройства
  └── broadcast            # Широковещательные сообщения
```

### Примеры топиков:
- `xiaozhi/device/ESP32-001/command` - команды для устройства ESP32-001
- `xiaozhi/device/ESP32-001/status` - статус устройства ESP32-001
- `xiaozhi/broadcast` - сообщения для всех устройств

## Использование в коде

### Публикация сообщения

```rust
use crate::mqtt::MqttHandler;

// Получить handler из сервиса
let mqtt_handler = mqtt_service.get_handler();

// Публикация команды устройству
mqtt_handler
    .publish_to_device("ESP32-001", "command", b"restart")
    .await?;

// Публикация в произвольный топик
mqtt_handler
    .publish("xiaozhi/broadcast", b"Hello all devices", QoS::AtLeastOnce)
    .await?;
```

## Безопасность

### Настройка аутентификации в Mosquitto

1. Создайте файл паролей:
```bash
mosquitto_passwd -c /etc/mosquitto/passwd username
```

2. Настройте `/etc/mosquitto/mosquitto.conf`:
```
allow_anonymous false
password_file /etc/mosquitto/passwd
```

3. Перезапустите брокер:
```bash
sudo systemctl restart mosquitto
```

4. Обновите `.env`:
```env
MQTT_BROKER=mqtt://username:password@localhost:1883
```

## Мониторинг

### Просмотр подключенных клиентов
```bash
mosquitto_sub -h localhost -t '$SYS/#' -v
```

### Веб-интерфейс (для EMQX)
Откройте в браузере: `http://localhost:18083`
- Логин: `admin`
- Пароль: `public`

## Troubleshooting

### Брокер не запускается
```bash
# Проверьте логи
sudo journalctl -u mosquitto -f
# или
mosquitto -v
```

### Не могу подключиться
1. Проверьте, что брокер запущен
2. Проверьте firewall: `sudo ufw allow 1883`
3. Проверьте, что порт не занят: `lsof -i :1883`

### Ошибка подключения в приложении
1. Убедитесь, что `MQTT_ENABLED=true` в `.env`
2. Проверьте, что собрали с feature: `cargo build --features mqtt`
3. Проверьте формат `MQTT_BROKER` в `.env`

