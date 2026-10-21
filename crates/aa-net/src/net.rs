//! Transport: `[u16 LE length][payload]` frames and one connection type per server. TCP natively; in
//! the browser a WebSocket to the host page's `/net/<addr>` relay, which forwards the bytes to TCP.

use std::io;
use std::time::Duration;

use thiserror::Error;
#[cfg(not(target_arch = "wasm32"))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(not(target_arch = "wasm32"))]
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
#[cfg(not(target_arch = "wasm32"))]
use tokio::net::TcpStream;

use crate::wire::WireError;
use crate::{game, login, stream};

#[derive(Debug, Error)]
pub enum NetError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Wire(#[from] WireError),
    #[error("the server closed the connection")]
    Closed,
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    #[error("{0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, NetError>;

/// A framed connection. Reading is cancel safe, so `recv_payload` may sit in a `select!`.
pub struct Link {
    #[cfg(not(target_arch = "wasm32"))]
    rd: OwnedReadHalf,
    #[cfg(not(target_arch = "wasm32"))]
    wr: OwnedWriteHalf,
    #[cfg(target_arch = "wasm32")]
    ws: web::Socket,
    buf: Vec<u8>,
    start: usize,
}

impl Link {
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn connect(addr: &str) -> Result<Self> {
        let sock = crate::timer::timeout(Duration::from_secs(10), TcpStream::connect(addr))
            .await
            .map_err(|_| NetError::Timeout("the connection"))??;
        sock.set_nodelay(true)?;
        let (rd, wr) = sock.into_split();
        Ok(Self {
            rd,
            wr,
            buf: Vec::new(),
            start: 0,
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn connect(addr: &str) -> Result<Self> {
        let ws = crate::timer::timeout(Duration::from_secs(10), web::Socket::connect(addr))
            .await
            .map_err(|_| NetError::Timeout("the connection"))??;
        Ok(Self {
            ws,
            buf: Vec::new(),
            start: 0,
        })
    }

    pub async fn send_payload(&mut self, payload: &[u8]) -> Result<()> {
        let len = u16::try_from(payload.len())
            .map_err(|_| NetError::Protocol("frame exceeds 65535 bytes".into()))?;
        let mut frame = Vec::with_capacity(2 + payload.len());
        frame.extend(len.to_le_bytes());
        frame.extend(payload);
        #[cfg(not(target_arch = "wasm32"))]
        self.wr.write_all(&frame).await?;
        #[cfg(target_arch = "wasm32")]
        self.ws.send(&frame)?;
        Ok(())
    }

    /// Takes one complete frame out of the buffer.
    fn pop(&mut self) -> Option<Vec<u8>> {
        let avail = &self.buf[self.start..];
        if avail.len() < 2 {
            return None;
        }
        let len = usize::from(u16::from_le_bytes([avail[0], avail[1]]));
        if avail.len() < 2 + len {
            return None;
        }
        let frame = avail[2..2 + len].to_vec();
        self.start += 2 + len;
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > 64 * 1024 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        Some(frame)
    }

    /// The next frame payload; `Err(Closed)` when the peer closed the connection.
    pub async fn recv_payload(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(f) = self.pop() {
                return Ok(f);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut chunk = [0u8; 16 * 1024];
                let n = self.rd.read(&mut chunk).await?;
                if n == 0 {
                    return Err(NetError::Closed);
                }
                self.buf.extend_from_slice(&chunk[..n]);
            }
            #[cfg(target_arch = "wasm32")]
            {
                let chunk = self.ws.recv().await.ok_or(NetError::Closed)?;
                self.buf.extend_from_slice(&chunk);
            }
        }
    }

    pub async fn shutdown(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.wr.shutdown().await;
        #[cfg(target_arch = "wasm32")]
        self.ws.close();
    }
}

/// Browser WebSocket to the host page's `/net/<addr>` relay.
#[cfg(target_arch = "wasm32")]
mod web {
    use std::io;

    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
    use wasm_bindgen::prelude::*;
    use web_sys::{BinaryType, MessageEvent, WebSocket};

    use super::{NetError, Result};

    enum Ev {
        Open,
        Data(Vec<u8>),
        Closed,
    }

    pub struct Socket {
        ws: WebSocket,
        rx: UnboundedReceiver<Ev>,
        // Kept alive for as long as the socket can call them.
        _handlers: [Closure<dyn FnMut(JsValue)>; 4],
    }

    impl Socket {
        /// Resolves once the socket is open; a socket that fails or closes first is a connect error.
        pub async fn connect(addr: &str) -> Result<Self> {
            let loc = web_sys::window().ok_or_else(|| io::Error::other("no window"))?.location();
            let scheme = if loc.protocol().is_ok_and(|p| p == "https:") { "wss" } else { "ws" };
            let host = loc.host().map_err(js_err)?;
            let ws = WebSocket::new(&format!("{scheme}://{host}/net/{addr}")).map_err(js_err)?;
            ws.set_binary_type(BinaryType::Arraybuffer);

            let (tx, rx) = unbounded_channel();
            let on_open = Closure::<dyn FnMut(JsValue)>::new({
                let tx = tx.clone();
                move |_| {
                    let _ = tx.send(Ev::Open);
                }
            });
            let on_message = Closure::<dyn FnMut(JsValue)>::new({
                let tx = tx.clone();
                move |e: JsValue| {
                    let e: MessageEvent = e.unchecked_into();
                    let _ = tx.send(Ev::Data(js_sys::Uint8Array::new(&e.data()).to_vec()));
                }
            });
            let on_error = Closure::<dyn FnMut(JsValue)>::new({
                let tx = tx.clone();
                move |_| {
                    let _ = tx.send(Ev::Closed);
                }
            });
            let on_close = Closure::<dyn FnMut(JsValue)>::new(move |_| {
                let _ = tx.send(Ev::Closed);
            });
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            // Dropping `sock` (also when the caller's timeout cancels us) closes the socket.
            let mut sock = Self {
                ws,
                rx,
                _handlers: [on_open, on_message, on_error, on_close],
            };
            match sock.rx.recv().await {
                Some(Ev::Open) => Ok(sock),
                _ => Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("cannot open the relay connection to {addr}"),
                )
                .into()),
            }
        }

        pub fn send(&self, frame: &[u8]) -> Result<()> {
            self.ws.send_with_u8_array(frame).map_err(js_err)
        }

        /// The next chunk of bytes; `None` once the socket is closed. Cancel safe.
        pub async fn recv(&mut self) -> Option<Vec<u8>> {
            match self.rx.recv().await? {
                Ev::Data(d) => Some(d),
                _ => None,
            }
        }

        pub fn close(&self) {
            let _ = self.ws.close();
        }
    }

    impl Drop for Socket {
        fn drop(&mut self) {
            self.ws.set_onopen(None);
            self.ws.set_onmessage(None);
            self.ws.set_onerror(None);
            self.ws.set_onclose(None);
            let _ = self.ws.close();
        }
    }

    fn js_err(e: JsValue) -> NetError {
        NetError::Io(io::Error::other(format!("{e:?}")))
    }
}

/// Connection to the login server.
pub struct LoginConn {
    link: Link,
}

impl LoginConn {
    pub async fn connect(addr: &str) -> Result<Self> {
        Ok(Self {
            link: Link::connect(addr).await?,
        })
    }

    pub async fn send(&mut self, msg: &login::ClientMsg) -> Result<()> {
        tracing::debug!("login <- {}", msg.name());
        self.link.send_payload(&msg.encode()).await
    }

    /// Cancel safe.
    pub async fn recv(&mut self) -> Result<login::ServerMsg> {
        let payload = self.link.recv_payload().await?;
        let msg = login::ServerMsg::decode(&payload)?;
        tracing::debug!("login -> {}", msg.name());
        Ok(msg)
    }

    pub async fn shutdown(&mut self) {
        self.link.shutdown().await;
    }
}

/// Connection to the game server.
pub struct GameConn {
    link: Link,
    counter: u8,
}

impl GameConn {
    pub async fn connect(addr: &str) -> Result<Self> {
        Ok(Self {
            link: Link::connect(addr).await?,
            counter: 0,
        })
    }

    pub async fn send(&mut self, msg: &game::ClientMsg) -> Result<()> {
        tracing::trace!("game <- {}", msg.name());
        let payload = msg.encode(self.counter);
        if msg.level() == game::LEVEL_GAME {
            self.counter = self.counter.wrapping_add(1);
        }
        self.link.send_payload(&payload).await
    }

    /// Cancel safe. Packets that fail to decode are reported as errors; the caller can keep reading
    /// because frames are length delimited.
    pub async fn recv(&mut self) -> Result<game::ServerMsg> {
        let payload = self.link.recv_payload().await?;
        let msg = game::ServerMsg::decode(&payload).map_err(|e| {
            NetError::Wire(WireError::Invalid(format!(
                "{e} ({})",
                describe_game_frame(&payload)
            )))
        })?;
        tracing::trace!("game -> {}", msg.name());
        Ok(msg)
    }

    pub async fn shutdown(&mut self) {
        self.link.shutdown().await;
    }
}

/// `level/opcode name` of a game frame, for error messages.
fn describe_game_frame(frame: &[u8]) -> String {
    match frame {
        [_, game::LEVEL_GAME, _, _, lo, hi, ..] => {
            let op = u16::from_le_bytes([*lo, *hi]);
            format!("{} {op:#05x}, {} bytes", crate::opcodes::sc::name(op), frame.len())
        }
        [_, game::LEVEL_PROXY, lo, hi, ..] => {
            let op = u16::from_le_bytes([*lo, *hi]);
            format!("proxy {} {op:#05x}", crate::opcodes::pp::name(op))
        }
        _ => format!("{} bytes", frame.len()),
    }
}

/// Connection to the stream server.
pub struct StreamConn {
    link: Link,
}

impl StreamConn {
    pub async fn connect(addr: &str) -> Result<Self> {
        Ok(Self {
            link: Link::connect(addr).await?,
        })
    }

    pub async fn send(&mut self, msg: &stream::ClientMsg) -> Result<()> {
        tracing::debug!("stream <- {}", msg.name());
        self.link.send_payload(&msg.encode()).await
    }

    /// Cancel safe.
    pub async fn recv(&mut self) -> Result<stream::ServerMsg> {
        let payload = self.link.recv_payload().await?;
        Ok(stream::ServerMsg::decode(&payload)?)
    }

    pub async fn shutdown(&mut self) {
        self.link.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn frames_split_and_join_across_reads() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Two frames in one write, a third frame split in two.
            sock.write_all(&[3, 0, 1, 2, 3, 1, 0, 9, 4, 0, 7]).await.unwrap();
            sock.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            sock.write_all(&[8, 8, 8]).await.unwrap();
            // Echo what the client sent.
            let mut got = [0u8; 5];
            sock.read_exact(&mut got).await.unwrap();
            assert_eq!(got, [3, 0, 5, 6, 7]);
        });
        let mut link = Link::connect(&addr).await.unwrap();
        assert_eq!(link.recv_payload().await.unwrap(), [1, 2, 3]);
        assert_eq!(link.recv_payload().await.unwrap(), [9]);
        assert_eq!(link.recv_payload().await.unwrap(), [7, 8, 8, 8]);
        link.send_payload(&[5, 6, 7]).await.unwrap();
        assert!(matches!(link.recv_payload().await, Err(NetError::Closed)));
        server.await.unwrap();
    }
}
