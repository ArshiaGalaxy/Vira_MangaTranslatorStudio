//! Health checks: poll localhost HTTP endpoints over a raw TCP socket
//! (no HTTP client crate needed; everything is loopback http).

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::time::{Duration, Instant};

/// Single GET attempt; returns true for any status < 500 (404/302 are healthy
/// for our purposes — Django/Next responded).
fn probe(port: u16, path: &str) -> Option<bool> {
    let mut stream = TcpStream::connect_timeout(
        &std::net::SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_millis(2500),
    )
    .ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(2500)))
        .ok()?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nUser-Agent: ViraLauncher/1.0\r\n\r\n",
        path, port
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let status: u16 = head
        .strip_prefix("HTTP/1.")
        .and_then(|s| s.split_once(' '))
        .and_then(|(_, rest)| rest.split_whitespace().next().and_then(|s| s.parse().ok()))
        .unwrap_or(999);
    Some(status < 500)
}

/// Poll until healthy, or until the process exits / timeout elapses.
/// `is_alive` lets the caller bail out early when the child process died.
pub fn wait_for(
    port: u16,
    path: &str,
    label: &str,
    timeout_secs: u64,
    mut is_alive: impl FnMut() -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut last = String::from("never connected");
    while Instant::now() < deadline {
        if !is_alive() {
            return Err(format!(
                "{label} process exited before becoming healthy (check the launcher log)"
            ));
        }
        match probe(port, path) {
            Some(true) => return Ok(()),
            Some(false) => last = "server returned HTTP >= 500".into(),
            None => last = "connection refused".into(),
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err(format!("{label} not ready after {timeout_secs}s ({last})"))
}
