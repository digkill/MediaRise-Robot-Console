//! OpenAI API proxy
//!
//! Endpoint: POST|GET /proxy/openai/*path
//!
//! Прозрачно пробрасывает запросы к настроенному OpenAI-совместимому API
//! (config.openai_llm.api_url), подставляя хранящийся API-ключ.
//! Поддерживает SSE-стриминг — тело ответа передаётся чанками без буферизации.
//!
//! Использование из Laravel (.env):
//!   OPENAI_API_URL=http://<host>:8080/proxy/openai
//!   OPENAI_API_KEY=   # ключ хранится на сервере, поле можно оставить пустым

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use futures::TryStreamExt;
use tracing::{debug, error};

use crate::server::AppState;

/// Проксирует запрос к OpenAI API.
///
/// `path` — всё после /proxy/openai/, например "chat/completions"
pub async fn proxy(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Result<Response<Body>, StatusCode> {
    let api_key = state
        .config
        .openai_llm
        .api_key
        .clone()
        .unwrap_or_default();

    let base = state.config.openai_llm.api_url.trim_end_matches('/');

    // Сохраняем query-string, если есть (?stream=true и т.п.)
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();

    let target_url = format!("{}/{}{}", base, path, query);
    debug!("OpenAI proxy → {} {}", req.method(), target_url);

    // Метод
    let method = match req.method().as_str() {
        "GET"     => reqwest::Method::GET,
        "POST"    => reqwest::Method::POST,
        "PUT"     => reqwest::Method::PUT,
        "DELETE"  => reqwest::Method::DELETE,
        "PATCH"   => reqwest::Method::PATCH,
        "HEAD"    => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        other => {
            error!("OpenAI proxy: unsupported method {}", other);
            return Err(StatusCode::METHOD_NOT_ALLOWED);
        }
    };

    // Заголовки входящего запроса
    let in_headers = req.headers().clone();

    // Читаем тело (лимит 8 МБ)
    let body_bytes = axum::body::to_bytes(req.into_body(), 8 * 1024 * 1024)
        .await
        .map_err(|e| {
            error!("OpenAI proxy: failed to read request body: {}", e);
            StatusCode::BAD_REQUEST
        })?;

    // Строим исходящий reqwest-запрос
    let client = reqwest::Client::new();
    let mut builder = client
        .request(method, &target_url)
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(180));

    // Пробрасываем Content-Type
    if let Some(ct) = in_headers.get("content-type") {
        if let Ok(s) = ct.to_str() {
            builder = builder.header("content-type", s);
        }
    }

    // Пробрасываем Accept (нужно для SSE: "text/event-stream")
    if let Some(accept) = in_headers.get("accept") {
        if let Ok(s) = accept.to_str() {
            builder = builder.header("accept", s);
        }
    }

    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }

    // Отправляем запрос апстриму
    let upstream = builder.send().await.map_err(|e| {
        error!("OpenAI proxy upstream error: {}", e);
        StatusCode::BAD_GATEWAY
    })?;

    let upstream_status = StatusCode::from_u16(upstream.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let upstream_headers = upstream.headers().clone();

    // Стримим ответ чанками, не буферизуя целиком
    let stream = upstream
        .bytes_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
    let body = Body::from_stream(stream);

    let mut response = Response::builder()
        .status(upstream_status)
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Пробрасываем заголовки ответа (кроме hop-by-hop)
    for (name, value) in upstream_headers.iter() {
        match name.as_str() {
            "transfer-encoding" | "connection" | "keep-alive" | "upgrade"
            | "proxy-connection" | "trailer" => continue,
            _ => {}
        }
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_ref()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            response.headers_mut().insert(n, v);
        }
    }

    Ok(response)
}
