//! Assets endpoints

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use sqlx::Row;
use std::path::PathBuf;
use tracing::{error, info};

use crate::server::AppState;

/// GET /assets/:version
/// Загрузка ресурсов для устройства
pub async fn download(
    axum::extract::State(state): axum::extract::State<AppState>,
    Path(version): Path<String>,
) -> Result<Response, StatusCode> {
    info!("Assets download request for version: {}", version);

    // Получаем URL ресурсов из базы данных
    let assets_url = match &*state.storage.database {
        crate::storage::database::Database::Sqlite(pool) => {
            let row = sqlx::query("SELECT url FROM assets_versions WHERE version = ?")
                .bind(version)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

            if let Some(row) = row {
                Some(row.get::<String, _>("url"))
            } else {
                None
            }
        }
        crate::storage::database::Database::Postgres(pool) => {
            let row = sqlx::query("SELECT url FROM assets_versions WHERE version = $1")
                .bind(version)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

            if let Some(row) = row {
                Some(row.get::<String, _>("url"))
            } else {
                None
            }
        }
        crate::storage::database::Database::Mysql(pool) => {
            let row = sqlx::query("SELECT url FROM assets_versions WHERE version = ?")
                .bind(version)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();

            if let Some(row) = row {
                Some(row.get::<String, _>("url"))
            } else {
                None
            }
        }
    };

    if let Some(url) = assets_url {
        // Если URL - это путь к файлу, читаем его
        if url.starts_with("http://") || url.starts_with("https://") {
            // Внешний URL - редирект
            return Err(StatusCode::NOT_IMPLEMENTED); // TODO: Implement redirect
        }

        let file_path = PathBuf::from(&url);
        if file_path.exists() {
            match tokio::fs::read(&file_path).await {
                Ok(data) => {
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "Content-Type",
                        HeaderValue::from_static("application/octet-stream"),
                    );
                    headers.insert(
                        "Content-Disposition",
                        HeaderValue::from_str(&format!(
                            "attachment; filename=\"{}\"",
                            file_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("assets")
                        ))
                        .unwrap_or(HeaderValue::from_static("attachment")),
                    );

                    return Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/octet-stream")
                        .body(Body::from(data))
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
                }
                Err(e) => {
                    error!("Failed to read assets file: {}", e);
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
        }
    }

    Err(StatusCode::NOT_FOUND)
}
