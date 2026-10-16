//! TCP transport: `[u16 LE length][payload]` frames and one connection type per server.

use std::io;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
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

/// A framed TCP connection. Reading is cancel safe, so `recv_payload` may sit in a `select!`.
pub struct Link {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
    buf: Vec<u8>,
    start: usize,
}

impl Link {
    pub async fn connect(addr: &str) -> Result<Self> {
        let sock = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
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

    pub async fn send_payload(&mut self, payload: &[u8]) -> Result<()> {
        let len = u16::try_from(payload.len())
            .map_err(|_| NetError::Protocol("frame exceeds 65535 bytes".into()))?;
        let mut frame = Vec::with_capacity(2 + payload.len());
        frame.extend(len.to_le_bytes());
        frame.extend(payload);
        self.wr.write_all(&frame).await?;
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
            let mut chunk = [0u8; 16 * 1024];
            let n = self.rd.read(&mut chunk).await?;
            if n == 0 {
                return Err(NetError::Closed);
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    pub async fn shutdown(&mut self) {
        let _ = self.wr.shutdown().await;
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
