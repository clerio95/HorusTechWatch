//! TCP client for the Horustech concentrator.
//!
//! Lifecycle: one connection per query batch. No keep-alive. The connection
//! is closed automatically when [`Connection`] is dropped. The send path
//! takes [`Command`], not raw bytes — the allowlist is enforced by the type
//! system upstream of this module.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::protocol::frame::{self, FrameError, ResponseFrame, MAX_DATA_LEN};
use crate::protocol::Command;

/// Hard-coded socket timeout per CLAUDE.md.
pub const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound on total bytes the read loop will pull off the socket
/// before giving up. Generous over MAX_DATA_LEN to allow for the
/// `>!CCCC...KK` framing overhead plus any leading garbage we discard.
const READ_BUDGET: usize = MAX_DATA_LEN + 64;

#[derive(Debug)]
pub enum ClientError {
    Resolve(String),
    Connect(std::io::Error),
    Io(std::io::Error),
    Frame(FrameError),
    Timeout,
    UnexpectedEof,
    ReadBudgetExceeded,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Resolve(s) => write!(f, "could not resolve address: {}", s),
            ClientError::Connect(e) => write!(f, "connect failed: {}", e),
            ClientError::Io(e) => write!(f, "I/O error: {}", e),
            ClientError::Frame(e) => write!(f, "frame error: {}", e),
            ClientError::Timeout => write!(f, "operation timed out"),
            ClientError::UnexpectedEof => write!(f, "device closed the connection mid-frame"),
            ClientError::ReadBudgetExceeded => {
                write!(f, "discarded more than {} bytes without finding a valid frame", READ_BUDGET)
            }
        }
    }
}

impl std::error::Error for ClientError {}

impl From<FrameError> for ClientError {
    fn from(e: FrameError) -> Self {
        ClientError::Frame(e)
    }
}

pub struct Connection {
    stream: TcpStream,
}

impl Connection {
    /// Open a TCP connection with [`SOCKET_TIMEOUT`] on connect + read + write.
    /// Returns once the socket is fully established.
    pub fn connect(host: &str, port: u16) -> Result<Self, ClientError> {
        let addr_str = format!("{}:{}", host, port);
        let addr: SocketAddr = addr_str
            .to_socket_addrs()
            .map_err(|e| ClientError::Resolve(e.to_string()))?
            .next()
            .ok_or_else(|| ClientError::Resolve(format!("no address for {}", addr_str)))?;

        let stream = TcpStream::connect_timeout(&addr, SOCKET_TIMEOUT).map_err(ClientError::Connect)?;
        stream.set_read_timeout(Some(SOCKET_TIMEOUT)).map_err(ClientError::Io)?;
        stream.set_write_timeout(Some(SOCKET_TIMEOUT)).map_err(ClientError::Io)?;
        stream.set_nodelay(true).map_err(ClientError::Io)?;
        Ok(Connection { stream })
    }

    /// Send `cmd`'s query frame and read the response.
    ///
    /// SAFETY: `cmd: Command` ensures the send is restricted to the compile-time
    /// allowlist. There is no path here that accepts raw bytes.
    pub fn query(&mut self, cmd: Command) -> Result<ResponseFrame, ClientError> {
        let query_bytes = cmd.query_frame();
        self.stream.write_all(&query_bytes).map_err(ClientError::Io)?;
        self.stream.flush().map_err(ClientError::Io)?;
        read_response_frame(&mut self.stream, cmd.index())
    }
}

/// Pull bytes off the stream until we have a complete frame, then parse it.
/// Bounded by [`READ_BUDGET`] total bytes and [`SOCKET_TIMEOUT`] per read.
fn read_response_frame(stream: &mut TcpStream, expected_idx: u8) -> Result<ResponseFrame, ClientError> {
    let started = Instant::now();
    let mut buf: Vec<u8> = Vec::with_capacity(READ_BUDGET);
    let mut found_delim = false;
    let mut declared_total: Option<usize> = None;

    let mut tmp = [0u8; 64];

    loop {
        if started.elapsed() > SOCKET_TIMEOUT * 2 {
            return Err(ClientError::Timeout);
        }
        if buf.len() >= READ_BUDGET {
            return Err(ClientError::ReadBudgetExceeded);
        }

        let want = if let Some(total) = declared_total {
            total - buf.len()
        } else {
            // Enough to learn the framing header (>!CCCC = 6 bytes).
            6
        };
        let want = want.min(tmp.len());

        let n = match stream.read(&mut tmp[..want]) {
            Ok(0) => return Err(ClientError::UnexpectedEof),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                return Err(ClientError::Timeout);
            }
            Err(e) => return Err(ClientError::Io(e)),
        };
        let chunk = &tmp[..n];

        if !found_delim {
            if let Some(pos) = chunk.iter().position(|&b| b == b'>') {
                buf.push(b'>');
                buf.extend_from_slice(&chunk[pos + 1..]);
                found_delim = true;
            }
            // else: pre-frame garbage, discard
        } else {
            buf.extend_from_slice(chunk);
        }

        // Once we have the 6-byte header, decode the declared length.
        if declared_total.is_none() && buf.len() >= 6 {
            if buf[1] != b'!' {
                return Err(ClientError::Frame(FrameError::NotAResponse));
            }
            let cccc = std::str::from_utf8(&buf[2..6]).map_err(|_| ClientError::Frame(FrameError::BadLengthField))?;
            let data_len = usize::from_str_radix(cccc, 16).map_err(|_| ClientError::Frame(FrameError::BadLengthField))?;
            if data_len > MAX_DATA_LEN {
                return Err(ClientError::Frame(FrameError::LengthExceedsMax));
            }
            declared_total = Some(6 + data_len + 2);
        }

        if let Some(total) = declared_total {
            if buf.len() >= total {
                let frame = frame::parse(&buf[..total], expected_idx)?;
                return Ok(frame);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    /// Spin up a one-shot loopback "device" that sends `response_bytes` after
    /// reading the client's query. Returns the bound port.
    fn one_shot_device(response: &'static [u8]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut discard = [0u8; 256];
            let _ = sock.read(&mut discard);
            let _ = sock.write_all(response);
            let _ = sock.flush();
        });
        port
    }

    #[test]
    fn round_trip_device_info_response() {
        let live = b">!009E12B01.00 F08.03 22/07/19 0 12,84 2 0113 3-00010427 17/01/17 26/05/26 00:26:28:11:04:27 192.168.025.091;00/00/00 Fc  00000000;c900HDNN;000.000.000.000;00000;D;FB";
        let port = one_shot_device(live);
        let mut conn = Connection::connect("127.0.0.1", port).expect("connect");
        let frame = conn.query(Command::DeviceInfo).expect("query");
        assert_eq!(frame.command_index, 0x12);
        assert_eq!(frame.payload[0], b'B');
    }

    #[test]
    fn rejects_response_with_wrong_index() {
        // Device echoes cmd 0x01 (status) but we queried 0x12 (device_info).
        // Body "!000201" sums to 0x144 → checksum "44". Frame: >!00020144
        let port = one_shot_device(b">!00020144");
        let mut conn = Connection::connect("127.0.0.1", port).unwrap();
        let err = conn.query(Command::DeviceInfo).unwrap_err();
        match err {
            ClientError::Frame(FrameError::CommandIndexMismatch { expected: 0x12, actual: 0x01 }) => {}
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn discards_leading_garbage_before_delimiter() {
        let mut response = b"XYZGARBAGE".to_vec();
        response.extend_from_slice(b">!009E12B01.00 F08.03 22/07/19 0 12,84 2 0113 3-00010427 17/01/17 26/05/26 00:26:28:11:04:27 192.168.025.091;00/00/00 Fc  00000000;c900HDNN;000.000.000.000;00000;D;FB");
        let leaked: &'static [u8] = response.leak();
        let port = one_shot_device(leaked);
        let mut conn = Connection::connect("127.0.0.1", port).unwrap();
        let frame = conn.query(Command::DeviceInfo).expect("query");
        assert_eq!(frame.command_index, 0x12);
    }

    #[test]
    fn times_out_when_device_never_responds() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Accept but never write — let the client time out on read.
        thread::spawn(move || {
            let (_sock, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_secs(30));
        });
        let mut conn = Connection::connect("127.0.0.1", port).unwrap();
        let start = Instant::now();
        let err = conn.query(Command::Status).unwrap_err();
        assert!(matches!(err, ClientError::Timeout));
        // Must give up within roughly 2 * SOCKET_TIMEOUT, well under the 30s server sleep.
        assert!(start.elapsed() < Duration::from_secs(15));
    }
}
