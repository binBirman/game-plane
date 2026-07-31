use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "static/"]
struct Asset;

pub async fn fallback(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/').to_string();

    if path.starts_with("api/") {
        return api_not_found();
    }

    if let Some(file) = Asset::get(&path) {
        return serve_asset(&path, file);
    }

    if let Some(file) = Asset::get("index.html") {
        return serve_asset("index.html", file);
    }

    (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "not found")
        .into_response()
}

fn serve_asset(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    let mime = mime_of(path);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime)],
        file.data,
    )
        .into_response()
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn api_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":{"code":"NOT_FOUND","message":"api endpoint not found"}}"#,
    )
        .into_response()
}