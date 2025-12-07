//! HTTP и WebSocket сервер

use axum::{
    extract::ws::WebSocketUpgrade,
    response::Response,
    routing::{get, post},
    Router,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

use axum::extract::State as AxumState;

use crate::config::Config;
use crate::handlers;
use crate::services::Services;
use crate::storage::Storage;
use crate::websocket;

pub async fn start(config: Config, services: Services, storage: Storage) -> anyhow::Result<()> {
    let app = create_router(config.clone(), services, storage);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    info!("Server listening on http://{}", addr);
    info!(
        "WebSocket endpoint: ws://{}:{}/ws",
        config.server.host, config.server.port
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn create_router(config: Config, services: Services, storage: Storage) -> Router {
    let state = AppState {
        config,
        services,
        storage,
    };

    Router::new()
        // OTA endpoints
        .route("/ota/", get(handlers::ota::check_version))
        .route("/ota/", post(handlers::ota::check_version))
        .route("/ota/activate", post(handlers::ota::activate))
        // Assets endpoints
        .route("/assets/:version", get(handlers::assets::download))
        // Upload endpoints
        .route("/upload/screenshot", post(handlers::upload::screenshot))
        // WebSocket
        .route("/ws", get(websocket_handler))
        // Health check
        .route("/health", get(|| async { "OK" }))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub services: Services,
    pub storage: Storage,
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<AppState>,
) -> Response {
    ws.on_upgrade(|socket| {
        websocket::handle_connection(socket, (state.config, state.services, state.storage))
    })
}
