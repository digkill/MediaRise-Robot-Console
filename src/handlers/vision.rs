use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Serialize;
use tracing::{error, info, warn};

use crate::server::AppState;

const MAX_VISION_FRAME_BYTES: usize = 256 * 1024;

#[derive(Debug, Serialize)]
pub struct VisionFrameResponse {
    pub accepted: bool,
}

/// POST /api/robot/vision/frame
/// Accepts a JPEG snapshot from the XIAO camera and analyzes it asynchronously.
pub async fn frame(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<VisionFrameResponse>), StatusCode> {
    if body.len() < 4
        || body.len() > MAX_VISION_FRAME_BYTES
        || !body.starts_with(&[0xff, 0xd8])
        || !body.ends_with(&[0xff, 0xd9])
    {
        warn!(frame_bytes = body.len(), "Rejected invalid robot vision JPEG");
        return Err(StatusCode::BAD_REQUEST);
    }

    let device_id = headers
        .get("x-device-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("homebot-xiao-camera")
        .to_string();
    let services = state.services.clone();
    let jpeg = body.to_vec();

    tokio::spawn(async move {
        match services.llm.describe_robot_view(&jpeg).await {
            Ok(description) if !description.trim().is_empty() => {
                info!(%device_id, %description, "Robot vision observation updated");
                crate::websocket::set_latest_visual_context(&device_id, &description).await;
            }
            Ok(_) => warn!(%device_id, "Vision model returned an empty observation"),
            Err(err) => error!(%device_id, "Vision analysis failed: {}", err),
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(VisionFrameResponse { accepted: true }),
    ))
}
