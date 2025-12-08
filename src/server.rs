//! HTTP и WebSocket сервер
//!
//! Этот модуль отвечает за:
//! 1. Создание HTTP роутера (маршрутизатора запросов)
//! 2. Настройку маршрутов (endpoints) для API
//! 3. Запуск HTTP сервера
//! 4. Обработку WebSocket соединений

// Импортируем нужные типы из библиотеки axum (веб-фреймворк для Rust)
use axum::{
    extract::ws::WebSocketUpgrade,  // Для обновления HTTP соединения до WebSocket
    response::Response,              // Тип для HTTP ответов
    routing::{get, post},            // Функции для создания маршрутов (GET, POST)
    Router,                          // Роутер - объект, который направляет запросы к нужным обработчикам
};
// Импортируем middleware (промежуточное ПО) для HTTP
use tower_http::{cors::CorsLayer, trace::TraceLayer};
// CORS - Cross-Origin Resource Sharing, позволяет браузерам делать запросы с других доменов
// TraceLayer - логирует все HTTP запросы

use tracing::info;  // Функция для информационного логирования

use axum::extract::State as AxumState;  // Для передачи состояния (state) в обработчики

// Импортируем наши модули
use crate::config::Config;      // Конфигурация
use crate::handlers;             // HTTP обработчики (функции, которые обрабатывают запросы)
use crate::services::Services;   // Бизнес-логика (сервисы)
use crate::storage::Storage;     // Хранилище данных
use crate::websocket;            // WebSocket обработка

/// Запускает HTTP и WebSocket сервер
/// 
/// Эта функция:
/// 1. Создает роутер с маршрутами
/// 2. Биндит (привязывает) сервер к указанному адресу и порту
/// 3. Начинает слушать входящие HTTP запросы и WebSocket соединения
/// 
/// Параметры:
/// - config: конфигурация сервера (адрес, порт)
/// - services: сервисы с бизнес-логикой
/// - storage: хранилище данных
/// 
/// Функция работает бесконечно, пока сервер не остановят (Ctrl+C)
pub async fn start(config: Config, services: Services, storage: Storage) -> anyhow::Result<()> {
    // Создаем роутер - объект, который знает, какой обработчик вызывать для какого URL
    let app = create_router(config.clone(), services, storage);

    // Формируем адрес для прослушивания в формате "IP:PORT"
    // Например: "0.0.0.0:8080" или "127.0.0.1:8080"
    let addr = format!("{}:{}", config.server.host, config.server.port);
    
    // Выводим информацию о том, где запущен сервер
    info!("Server listening on http://{}", addr);
    info!(
        "WebSocket endpoint: ws://{}:{}/ws",
        config.server.host, config.server.port
    );

    // Создаем TCP listener - объект, который слушает входящие TCP соединения
    // TCP - это протокол, на котором работает HTTP
    // bind() привязывает listener к указанному адресу и порту
    // await ждет, пока операция завершится
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    
    // Запускаем сервер
    // serve() принимает listener и роутер, начинает обрабатывать входящие запросы
    // Эта функция работает бесконечно, пока сервер не остановят
    axum::serve(listener, app).await?;

    // Если сервер остановился, возвращаем успешный результат
    Ok(())
}

/// Создает HTTP роутер с маршрутами
/// 
/// Роутер - это объект, который знает, какую функцию вызывать для какого URL.
/// Например, когда приходит запрос на "/health", роутер вызывает функцию,
/// которая возвращает "OK".
/// 
/// Параметры:
/// - config: конфигурация
/// - services: сервисы с бизнес-логикой
/// - storage: хранилище данных
/// 
/// Возвращает Router - настроенный роутер со всеми маршрутами
fn create_router(config: Config, services: Services, storage: Storage) -> Router {
    // Создаем состояние приложения (AppState)
    // Это объект, который будет доступен во всех обработчиках
    // В Rust это называется "state" - способ передать данные в обработчики
    let state = AppState {
        config,      // Конфигурация
        services,    // Сервисы (LLM, STT, TTS и т.д.)
        storage,     // Хранилище (база данных, файлы)
    };

    // Создаем новый роутер и добавляем маршруты
    Router::new()
        // ============================================
        // OTA (Over-The-Air) endpoints - обновления прошивки
        // ============================================
        // GET /ota/ - проверка версии прошивки (можно получить через GET или POST)
        .route("/ota/", get(handlers::ota::check_version))
        .route("/ota/", post(handlers::ota::check_version))
        // POST /ota/activate - активация устройства (подтверждение, что устройство готово к обновлению)
        .route("/ota/activate", post(handlers::ota::activate))
        
        // ============================================
        // Assets endpoints - загрузка ресурсов
        // ============================================
        // GET /assets/:version - скачать ассеты (ресурсы) для указанной версии
        // :version - это параметр пути, например /assets/v1.0.0
        .route("/assets/:version", get(handlers::assets::download))
        
        // ============================================
        // Upload endpoints - загрузка файлов
        // ============================================
        // POST /upload/screenshot - загрузить скриншот с устройства
        .route("/upload/screenshot", post(handlers::upload::screenshot))
        
        // ============================================
        // WebSocket endpoint - для голосового взаимодействия
        // ============================================
        // GET /ws - установить WebSocket соединение
        // WebSocket позволяет двустороннюю связь в реальном времени
        // Используется для отправки аудио и получения ответов
        .route("/ws", get(websocket_handler))
        
        // ============================================
        // Health check endpoint - проверка работоспособности
        // ============================================
        // GET /health - простой endpoint для проверки, что сервер работает
        // Обычно используется мониторингом и балансировщиками нагрузки
        // || async { "OK" } - это замыкание (анонимная функция), которое возвращает "OK"
        .route("/health", get(|| async { "OK" }))
        
        // ============================================
        // Middleware (промежуточное ПО)
        // ============================================
        // CORS - разрешает запросы с других доменов (нужно для веб-приложений)
        // permissive() - разрешает все запросы (в продакшене лучше настроить конкретные домены)
        .layer(CorsLayer::permissive())
        // TraceLayer - логирует все HTTP запросы (URL, метод, статус, время выполнения)
        .layer(TraceLayer::new_for_http())
        // with_state() - передает состояние (state) во все обработчики
        // Теперь каждый обработчик может получить доступ к config, services, storage
        .with_state(state)
}

/// Состояние приложения (Application State)
/// 
/// Этот объект хранит все данные, которые нужны обработчикам:
/// - config: конфигурация (настройки)
/// - services: сервисы с бизнес-логикой
/// - storage: хранилище данных
/// 
/// Clone означает, что можно создавать копии этого объекта
/// (на самом деле копируются только ссылки, не сами данные)
#[derive(Clone)]
pub struct AppState {
    /// Конфигурация приложения
    pub config: Config,
    /// Сервисы (LLM, STT, TTS, Device и т.д.)
    pub services: Services,
    /// Хранилище данных (база данных, файлы)
    pub storage: Storage,
}

/// Обработчик WebSocket соединений
/// 
/// Эта функция вызывается, когда клиент пытается установить WebSocket соединение.
/// 
/// Параметры:
/// - ws: объект WebSocketUpgrade - содержит информацию о запросе на обновление до WebSocket
/// - state: состояние приложения (config, services, storage)
/// 
/// Процесс:
/// 1. Клиент отправляет HTTP запрос с заголовками для обновления до WebSocket
/// 2. Эта функция проверяет запрос и решает, разрешить ли обновление
/// 3. Если разрешено, вызывается on_upgrade с функцией-обработчиком
/// 4. Функция-обработчик получает WebSocket соединение и начинает обрабатывать сообщения
async fn websocket_handler(
    ws: WebSocketUpgrade,  // Объект для обновления HTTP до WebSocket
    AxumState(state): AxumState<AppState>,  // Извлекаем состояние из запроса
) -> Response {
    // on_upgrade вызывается, когда HTTP соединение успешно обновлено до WebSocket
    // socket - это установленное WebSocket соединение
    // Теперь можно отправлять и получать сообщения в реальном времени
    ws.on_upgrade(|socket| {
        // Вызываем функцию handle_connection из модуля websocket
        // Она будет обрабатывать все сообщения от клиента
        // Передаем socket и состояние (config, services, storage)
        websocket::handle_connection(socket, (state.config, state.services, state.storage))
    })
}
