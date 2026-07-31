use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use tokio::net::TcpStream;
use tokio_tungstenite::client_async;
use tracing::{error, info, warn};

use crate::http::error::ApiError;
use crate::instance::manager::Status;
use crate::state::SharedState;

pub async fn ws_handler(
    State(state): State<SharedState>,
    Path(instance_id): Path<i64>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let port = match state.instances.lookup(instance_id).await {
        Some((port, Status::Ready | Status::Running)) => port,
        Some(_) => return ApiError::InstanceNotReady.into_response(),
        None => return ApiError::InstanceNotFound.into_response(),
    };

    ws.on_upgrade(move |socket| bridge(socket, instance_id, port))
}

async fn bridge(client_ws: WebSocket, instance_id: i64, game_port: u16) {
    let tcp = match TcpStream::connect(("127.0.0.1", game_port)).await {
        Ok(t) => t,
        Err(e) => {
            warn!(instance_id, error=%e, "ws_proxy: connect to game failed");
            return;
        }
    };

    let req = match axum::http::Request::builder()
        .method("GET")
        .header("Host", format!("127.0.0.1:{}", game_port))
        .header("Upgrade", "websocket")
        .header("Connection", "Upgrade")
        .header("Sec-WebSocket-Key", ws_key())
        .header("Sec-WebSocket-Version", "13")
        .body(())
    {
        Ok(r) => r,
        Err(e) => {
            error!(instance_id, error=%e, "ws_proxy: build upgrade req failed");
            return;
        }
    };

    let (game_ws, _resp) = match client_async(req, tcp).await {
        Ok(v) => v,
        Err(e) => {
            warn!(instance_id, error=%e, "ws_proxy: game handshake failed");
            return;
        }
    };

    info!(instance_id, "ws_proxy: bridge established");

    let (mut ct, mut cr) = client_ws.split();
    let (mut gt, mut gr) = game_ws.split();

    let c2g = tokio::spawn(async move {
        while let Some(Ok(msg)) = cr.next().await {
            if gt.send(axum_to_tung(msg)).await.is_err() {
                break;
            }
        }
    });

    let g2c = tokio::spawn(async move {
        while let Some(Ok(msg)) = gr.next().await {
            if ct.send(tung_to_axum(msg)).await.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(c2g, g2c);
}

fn axum_to_tung(m: axum::extract::ws::Message) -> tokio_tungstenite::tungstenite::Message {
    use axum::extract::ws::Message as A;
    use tokio_tungstenite::tungstenite::Message as T;
    match m {
        A::Text(s) => T::Text(s),
        A::Binary(b) => T::Binary(b),
        A::Ping(b) => T::Ping(b),
        A::Pong(b) => T::Pong(b),
        A::Close(_) => T::Close(None),
    }
}

fn tung_to_axum(m: tokio_tungstenite::tungstenite::Message) -> axum::extract::ws::Message {
    use axum::extract::ws::Message as A;
    use tokio_tungstenite::tungstenite::Message as T;
    match m {
        T::Text(s) => A::Text(s),
        T::Binary(b) => A::Binary(b),
        T::Ping(b) => A::Ping(b),
        T::Pong(b) => A::Pong(b),
        T::Close(_) => A::Close(None),
        T::Frame(_) => unreachable!(),
    }
}

fn ws_key() -> String {
    let bytes: [u8; 16] = rand::thread_rng().gen();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}