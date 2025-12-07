//! Upload endpoints

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::Json,
};
use serde::Serialize;
use std::path::PathBuf;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub success: bool,
    pub url: Option<String>,
}

/// POST /upload/screenshot
/// Загрузка скриншота экрана устройства
pub async fn screenshot(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, StatusCode> {
    info!("Screenshot upload request");

    let mut device_id: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;

    // Парсим multipart form data
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error!("Failed to read multipart field: {}", e);
        StatusCode::BAD_REQUEST
    })? {
        let name = field.name().unwrap_or("");

        match name {
            "device_id" => {
                device_id = field.text().await.ok();
            }
            "file" | "screenshot" => {
                file_name = field.file_name().map(|s| s.to_string());
                if let Ok(data) = field.bytes().await {
                    file_data = Some(data.to_vec());
                }
            }
            _ => {
                warn!("Unknown multipart field: {}", name);
            }
        }
    }

    let device_id = device_id.ok_or_else(|| {
        error!("Missing device_id in upload request");
        StatusCode::BAD_REQUEST
    })?;

    let file_data = file_data.ok_or_else(|| {
        error!("Missing file data in upload request");
        StatusCode::BAD_REQUEST
    })?;

    // Генерируем уникальное имя файла
    let file_id = Uuid::new_v4();
    let extension = if let Some(ref name) = file_name {
        PathBuf::from(name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "png".to_string())
    } else {
        "png".to_string()
    };

    let file_name = format!("{}.{}", file_id, extension);
    let file_path = state.config.storage.uploads_path.join(&file_name);

    // Сохраняем файл
    if let Err(e) = tokio::fs::create_dir_all(&state.config.storage.uploads_path).await {
        error!("Failed to create uploads directory: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Err(e) = tokio::fs::write(&file_path, &file_data).await {
        error!("Failed to save screenshot: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Сохраняем информацию о загрузке в базу данных
    let upload_id = Uuid::new_v4().to_string();
    let file_path_str = file_path.to_string_lossy().to_string();
    match &*state.storage.database {
        crate::storage::database::Database::Sqlite(pool) => {
            let _ = sqlx::query(
                "INSERT INTO uploads (id, device_id, file_path, file_type) VALUES (?, ?, ?, ?)",
            )
            .bind(&upload_id)
            .bind(&device_id)
            .bind(&file_path_str)
            .bind("screenshot")
            .execute(pool)
            .await;
        }
        crate::storage::database::Database::Postgres(pool) => {
            let _ = sqlx::query(
                "INSERT INTO uploads (id, device_id, file_path, file_type) VALUES ($1, $2, $3, $4)",
            )
            .bind(&upload_id)
            .bind(&device_id)
            .bind(&file_path_str)
            .bind("screenshot")
            .execute(pool)
            .await;
        }
        crate::storage::database::Database::Mysql(pool) => {
            let _ = sqlx::query(
                "INSERT INTO uploads (id, device_id, file_path, file_type) VALUES (?, ?, ?, ?)",
            )
            .bind(&upload_id)
            .bind(&device_id)
            .bind(&file_path_str)
            .bind("screenshot")
            .execute(pool)
            .await;
        }
    }

    let url = format!("/uploads/{}", file_name);

    info!(
        "Screenshot uploaded: {} ({} bytes)",
        file_name,
        file_data.len()
    );

    Ok(Json(UploadResponse {
        success: true,
        url: Some(url),
    }))
}
