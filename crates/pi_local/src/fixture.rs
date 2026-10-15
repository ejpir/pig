//! Bounded HTTP reader for synthetic native OAuth tests.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read},
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
/// Loopback is not UID-private on Android: bound even a hostile peer's partial request.
struct RequestReader<'a> {
    socket: &'a mut TcpStream,
    stop: &'a AtomicBool,
    deadline: Instant,
}
impl Read for RequestReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("fixture stopped"));
            }
            if Instant::now() >= self.deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "fixture request deadline",
                ));
            }
            match self.socket.read(bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                result => return result,
            }
        }
    }
}
pub(crate) fn read_http_request(
    socket: &mut TcpStream,
    stop: &AtomicBool,
) -> Result<(String, Value, Vec<u8>)> {
    read_http_request_limit(socket, stop, 1024 * 1024)
}
pub(crate) fn read_http_request_limit(
    socket: &mut TcpStream,
    stop: &AtomicBool,
    limit: usize,
) -> Result<(String, Value, Vec<u8>)> {
    socket.set_read_timeout(Some(Duration::from_millis(20)))?;
    let mut reader = BufReader::new(RequestReader {
        socket,
        stop,
        deadline: Instant::now() + Duration::from_secs(2),
    });
    let mut line = String::new();
    reader.by_ref().take(4097).read_line(&mut line)?;
    ensure!(
        line.len() <= 4096 && line.ends_with("\r\n"),
        "fixture request line too long or incomplete"
    );
    let parts: Vec<_> = line.split_whitespace().collect();
    ensure!(
        parts.len() == 3 && parts[0] == "POST" && parts[2] == "HTTP/1.1",
        "unexpected request line"
    );
    let path = parts[1].to_owned();
    let mut length = 0;
    let mut headers = json!({});
    let mut header_bytes = line.len();
    loop {
        line.clear();
        reader.by_ref().take(8193).read_line(&mut line)?;
        header_bytes += line.len();
        ensure!(header_bytes <= 16 * 1024, "fixture headers exceed 16 KiB");
        ensure!(
            line.len() <= 8192 && !line.is_empty(),
            "bad fixture request header"
        );
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').context("invalid header")?;
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse::<usize>()?;
        }
        ensure!(
            headers.get(name.to_ascii_lowercase()).is_none(),
            "duplicate fixture header"
        );
        headers[name.to_ascii_lowercase()] = json!(value.trim());
    }
    ensure!(
        headers.get("transfer-encoding").is_none() && length > 0 && length <= limit,
        "bad fixture request length"
    );
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok((path, headers, body))
}
