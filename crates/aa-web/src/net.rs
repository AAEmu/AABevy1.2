//! `/net/<name>`: browsers cannot open TCP sockets, so the client opens a WebSocket here and its
//! binary messages are relayed to the configured game server (and back). Only the targets given
//! with `--net` are reachable, so this is not an open proxy.
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::App;

pub async fn bridge(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(&target) = app.net.get(&name) else {
        return (StatusCode::NOT_FOUND, "unknown net target").into_response();
    };
    let Ok(slot) = app.net_slots.clone().try_acquire_owned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "too many connections").into_response();
    };
    ws.max_message_size(1 << 20).on_upgrade(move |socket| async move {
        let _slot = slot;
        match tokio::net::TcpStream::connect(target).await {
            Ok(tcp) => {
                tracing::info!("net {name}: {peer} <-> {target}");
                relay(socket, tcp).await;
                tracing::info!("net {name}: {peer} closed");
            }
            Err(e) => tracing::warn!("net {name}: cannot reach {target}: {e}"),
        }
    })
}

async fn relay(socket: WebSocket, tcp: tokio::net::TcpStream) {
    let _ = tcp.set_nodelay(true);
    let (mut tcp_rx, mut tcp_tx) = tcp.into_split();
    let (mut ws_tx, mut ws_rx) = socket.split();
    let up = async {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Binary(b) => {
                    if tcp_tx.write_all(&b).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = tcp_tx.shutdown().await;
    };
    let down = async {
        let mut buf = vec![0u8; 64 << 10];
        loop {
            match tcp_rx.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_tx.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = ws_tx.close().await;
    };
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
}
