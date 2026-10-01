//! The desktop channel: a local socket that the desktop extension in pi connects to
//! (`PI_DESKTOP_CHANNEL`). It carries the commands routed to the extension; replies
//! have the shape of pi's RPC responses and join the same event stream.
//!
//! pi recreates extensions whenever it replaces its session (new, resume, fork,
//! reload), so the extension connects again each time and first says `hello`. New
//! requests go to the newest connection. Replies are accepted from any connection,
//! because a request sent before a reload is answered on the connection that received
//! it. Requests sent before the first connection wait for it; their deadlines still run.

use std::{
    collections::VecDeque,
    ffi::OsString,
    io::{BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result};
use async_channel::Sender;
use serde_json::Value;

use crate::protocol::read_record;

/// The extension's address: a socket path, or `tcp:<port>` on 127.0.0.1.
pub const ADDRESS_ENV: &str = "PI_DESKTOP_CHANNEL";
/// With a port: the token the extension's `hello` must carry.
pub const TOKEN_ENV: &str = "PI_DESKTOP_CHANNEL_TOKEN";
/// How long a new connection may take to say `hello`.
const HELLO_LIMIT: Duration = Duration::from_secs(10);
/// Requests waiting for a connection; their deadlines fail them long before this fills.
const MAX_QUEUED: usize = 256;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Transport {
    /// A socket in a private directory: only this user can reach it.
    #[cfg(unix)]
    Unix,
    /// A loopback port, which any local program can reach, so `hello` carries a
    /// random token. Windows has no Unix sockets in Rust's standard library.
    #[cfg_attr(unix, allow(dead_code))]
    Tcp,
}

#[cfg(unix)]
pub(crate) const TRANSPORT: Transport = Transport::Unix;
#[cfg(not(unix))]
pub(crate) const TRANSPORT: Transport = Transport::Tcp;

enum Listener {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixListener),
    Tcp(TcpListener),
}

enum Stream {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    Tcp(TcpStream),
}

impl Listener {
    fn accept(&self) -> std::io::Result<Stream> {
        match self {
            #[cfg(unix)]
            Self::Unix(listener) => listener.accept().map(|(stream, _)| Stream::Unix(stream)),
            Self::Tcp(listener) => listener.accept().map(|(stream, _)| Stream::Tcp(stream)),
        }
    }
}

impl Stream {
    fn try_clone(&self) -> std::io::Result<Self> {
        Ok(match self {
            #[cfg(unix)]
            Self::Unix(stream) => Self::Unix(stream.try_clone()?),
            Self::Tcp(stream) => Self::Tcp(stream.try_clone()?),
        })
    }

    /// Accepted sockets inherit the listener's nonblocking mode on some systems.
    fn configure(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => {
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(timeout)
            }
            Self::Tcp(stream) => {
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(timeout)
            }
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(bytes),
            Self::Tcp(stream) => stream.write(bytes),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

enum Message {
    Request(Vec<u8>),
    Connected(Stream),
}

/// One pi process's channel. Dropping it stops accepting and removes a socket file.
pub(crate) struct Channel {
    env: Vec<(OsString, OsString)>,
    writes: Sender<Message>,
    _socket_dir: Option<tempfile::TempDir>,
}

impl Channel {
    /// Listens until `stopped`; `deliver` receives each record a connection sends
    /// after its `hello`, and returns `false` once nobody listens any more.
    pub(crate) fn open(
        transport: Transport,
        stopped: Arc<AtomicBool>,
        deliver: impl Fn(Value) -> bool + Send + Sync + 'static,
    ) -> Result<Self> {
        let (env, socket_dir, listener, token) = match transport {
            #[cfg(unix)]
            Transport::Unix => {
                let dir = tempfile::Builder::new().prefix("pi-desktop-").tempdir()?;
                let socket = dir.path().join("channel");
                let listener = std::os::unix::net::UnixListener::bind(&socket)
                    .with_context(|| format!("listening on {}", socket.display()))?;
                listener.set_nonblocking(true)?;
                let env = vec![(ADDRESS_ENV.into(), socket.into_os_string())];
                (env, Some(dir), Listener::Unix(listener), None)
            }
            Transport::Tcp => {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
                listener.set_nonblocking(true)?;
                let port = listener.local_addr()?.port();
                let token = format!("{:032x}", rand::random::<u128>());
                let env = vec![
                    (ADDRESS_ENV.into(), format!("tcp:{port}").into()),
                    (TOKEN_ENV.into(), token.clone().into()),
                ];
                (env, None, Listener::Tcp(listener), Some(token))
            }
        };
        let (writes, requests) = async_channel::bounded::<Message>(64);
        thread::spawn(move || {
            let mut current: Option<Stream> = None;
            let mut queued = VecDeque::<Vec<u8>>::new();
            while let Ok(message) = requests.recv_blocking() {
                match message {
                    Message::Connected(stream) => current = Some(stream),
                    Message::Request(bytes) => {
                        if queued.len() >= MAX_QUEUED {
                            queued.pop_front();
                        }
                        queued.push_back(bytes);
                    }
                }
                while let (Some(stream), Some(bytes)) = (current.as_mut(), queued.front()) {
                    if stream
                        .write_all(bytes)
                        .and_then(|_| stream.flush())
                        .is_err()
                    {
                        // The extension went away; the next connection gets the rest.
                        current = None;
                        break;
                    }
                    queued.pop_front();
                }
            }
        });
        let deliver = Arc::new(deliver);
        let connected = writes.clone();
        thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok(stream) => {
                        let deliver = deliver.clone();
                        let connected = connected.clone();
                        let token = token.clone();
                        thread::spawn(move || serve(stream, token, connected, &*deliver));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            env,
            writes,
            _socket_dir: socket_dir,
        })
    }

    /// Add to pi's environment: [`ADDRESS_ENV`], and [`TOKEN_ENV`] with a port.
    pub(crate) fn env(&self) -> &[(OsString, OsString)] {
        &self.env
    }

    /// Queues one JSON line for the extension.
    pub(crate) fn send(&self, bytes: Vec<u8>) -> Result<()> {
        self.writes
            .try_send(Message::Request(bytes))
            .context("Extension write queue full or closed")
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.writes.close();
    }
}

/// One connection: `hello` first, then records until the extension goes away.
fn serve(
    stream: Stream,
    token: Option<String>,
    connected: Sender<Message>,
    deliver: &(dyn Fn(Value) -> bool + Send + Sync),
) {
    let hello = (|| -> Result<Stream> {
        stream.configure(Some(HELLO_LIMIT))?;
        let writer = stream.try_clone()?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let hello = read_record(&mut reader)?.context("closed before hello")?;
        anyhow::ensure!(hello["type"] == "hello", "the first record must be hello");
        if let Some(token) = &token {
            anyhow::ensure!(
                hello["token"].as_str() == Some(token),
                "missing or wrong token"
            );
        }
        stream.configure(None)?;
        Ok(writer)
    })();
    let Ok(writer) = hello else {
        return;
    };
    if connected.send_blocking(Message::Connected(writer)).is_err() {
        return;
    }
    let mut reader = BufReader::new(stream);
    while let Ok(Some(record)) = read_record(&mut reader) {
        if !deliver(record) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead as _, BufReader},
        sync::Mutex,
        time::Instant,
    };

    fn connect(channel: &Channel) -> Stream {
        let address = channel.env()[0].1.to_string_lossy().into_owned();
        match address.strip_prefix("tcp:") {
            Some(port) => {
                Stream::Tcp(TcpStream::connect(("127.0.0.1", port.parse().unwrap())).unwrap())
            }
            #[cfg(unix)]
            None => Stream::Unix(std::os::unix::net::UnixStream::connect(address).unwrap()),
            #[cfg(not(unix))]
            None => unreachable!(),
        }
    }

    fn line(stream: &mut Stream) -> Value {
        let mut text = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut text)
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn wait_for(records: &Mutex<Vec<Value>>, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let records = records.lock().unwrap();
            if records.len() >= count {
                return records.clone();
            }
            drop(records);
            assert!(Instant::now() < deadline, "records did not arrive");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn open(transport: Transport) -> (Channel, Arc<Mutex<Vec<Value>>>, Arc<AtomicBool>) {
        let records = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let sink = records.clone();
        let channel = Channel::open(transport, stopped.clone(), move |record| {
            sink.lock().unwrap().push(record);
            true
        })
        .unwrap();
        (channel, records, stopped)
    }

    fn hello(channel: &Channel) -> Value {
        match channel.env().get(1) {
            Some((_, token)) => {
                serde_json::json!({"type": "hello", "token": token.to_string_lossy()})
            }
            None => serde_json::json!({"type": "hello"}),
        }
    }

    fn requests_wait_for_hello_and_follow_the_newest_connection(transport: Transport) {
        let (channel, records, stopped) = open(transport);
        channel
            .send(b"{\"id\":\"1\",\"type\":\"get_settings\"}\n".to_vec())
            .unwrap();
        let mut first = connect(&channel);
        first
            .write_all(format!("{}\n", hello(&channel)).as_bytes())
            .unwrap();
        assert_eq!(
            line(&mut first)["id"],
            "1",
            "the waiting request arrives after hello"
        );

        // A reload: the old connection still answers what it received.
        let mut second = connect(&channel);
        second
            .write_all(format!("{}\n", hello(&channel)).as_bytes())
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        channel
            .send(b"{\"id\":\"2\",\"type\":\"get_settings\"}\n".to_vec())
            .unwrap();
        assert_eq!(line(&mut second)["id"], "2");
        first
            .write_all(b"{\"type\":\"response\",\"id\":\"1\",\"command\":\"get_settings\",\"success\":true}\n")
            .unwrap();
        second
            .write_all(b"{\"type\":\"response\",\"id\":\"2\",\"command\":\"get_settings\",\"success\":true}\n")
            .unwrap();
        let records = wait_for(&records, 2);
        let mut ids: Vec<_> = records
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        assert_eq!(ids, ["1", "2"]);
        assert!(
            records.iter().all(|r| r["type"] == "response"),
            "hello is not delivered"
        );
        stopped.store(true, Ordering::Release);
    }

    #[test]
    #[cfg(unix)]
    fn unix_requests_wait_for_hello_and_follow_the_newest_connection() {
        requests_wait_for_hello_and_follow_the_newest_connection(Transport::Unix);
    }

    #[test]
    fn tcp_requests_wait_for_hello_and_follow_the_newest_connection() {
        requests_wait_for_hello_and_follow_the_newest_connection(Transport::Tcp);
    }

    #[test]
    fn a_tcp_connection_without_the_token_gets_nothing() {
        let (channel, records, stopped) = open(Transport::Tcp);
        let mut stranger = connect(&channel);
        stranger
            .write_all(b"{\"type\":\"hello\",\"token\":\"guess\"}\n")
            .unwrap();
        stranger
            .write_all(b"{\"type\":\"response\",\"id\":\"1\",\"command\":\"x\",\"success\":true}\n")
            .unwrap();
        channel
            .send(b"{\"id\":\"1\",\"type\":\"get_settings\"}\n".to_vec())
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(records.lock().unwrap().is_empty());
        let mut buffer = [0; 1];
        stranger
            .configure(Some(Duration::from_millis(200)))
            .unwrap();
        assert!(
            matches!(stranger.read(&mut buffer), Ok(0) | Err(_)),
            "the request is not sent to a stranger"
        );
        stopped.store(true, Ordering::Release);
    }
}
