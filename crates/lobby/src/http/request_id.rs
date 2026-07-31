use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use rand::RngCore;
use tracing::Instrument;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

pub async fn middleware(req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(generate_id);

    let method = req.method().clone();
    let uri = req.uri().clone();

    let span = tracing::info_span!(
        "http_request",
        request_id = %id,
        method = %method,
        path = %uri.path(),
    );

    async move {
        tracing::info!(target: "lobby::http", "request received");

        let mut response = next.run(req).await;
        let status = response.status();

        let value = HeaderValue::from_str(&id).unwrap_or_else(|_| HeaderValue::from_static("invalid"));
        response.headers_mut().insert(REQUEST_ID_HEADER.clone(), value);

        tracing::info!(target: "lobby::http", status = %status.as_u16(), "request finished");
        response
    }
    .instrument(span)
    .await
}

fn generate_id() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{:016x}", u64::from_be_bytes(bytes))
}