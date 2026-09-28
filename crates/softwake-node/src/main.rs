//! Softwake companion node (Remote Agent / ADR-0039).
//!
//! Slice 1 stub: `GET /health` and `GET /v1/outbox` (empty). Bind via
//! `SOFTWAKE_NODE_LISTEN` (default `127.0.0.1:8790`). Production should bind the
//! node's Tailscale IP / `MagicDNS` only — never a public WAN interface.

use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const DEFAULT_LISTEN: &str = "127.0.0.1:8790";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let listen = env::var("SOFTWAKE_NODE_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.to_owned());
    let listener = TcpListener::bind(&listen).unwrap_or_else(|error| {
        eprintln!("softwake-node: bind {listen} failed: {error}");
        std::process::exit(1);
    });
    let addr = listener.local_addr().expect("local addr");
    eprintln!("softwake-node: companion listening on {addr} (role=companion)");
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                if let Err(error) = handle_client(stream) {
                    eprintln!("softwake-node: connection error: {error}");
                }
            }
            Err(error) => eprintln!("softwake-node: accept: {error}"),
        }
    }
}

fn handle_client(mut stream: TcpStream) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0_u8; 4096];
    let n = stream.read(&mut buf)?;
    if n == 0 {
        return Ok(());
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = request_path(&req).unwrap_or("/");
    match path {
        "/health" | "/health/" => {
            let body = format!("{{\"ok\":true,\"role\":\"companion\",\"version\":\"{VERSION}\"}}");
            write_response(&mut stream, 200, "application/json", &body)
        }
        "/v1/outbox" | "/v1/outbox/" => {
            write_response(&mut stream, 200, "application/json", "{\"items\":[]}")
        }
        _ => write_response(&mut stream, 404, "text/plain", "not found\n"),
    }
}

fn request_path(req: &str) -> Option<&str> {
    let line = req.lines().next()?;
    let mut parts = line.split_whitespace();
    let _method = parts.next()?;
    let target = parts.next()?;
    Some(target.split('?').next().unwrap_or(target))
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn health_and_empty_outbox_on_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            handle_client(stream).expect("handle");
            let (stream, _) = listener.accept().expect("accept2");
            handle_client(stream).expect("handle2");
        });
        thread::sleep(Duration::from_millis(20));
        let health = http_get(addr, "/health");
        assert!(health.contains("200"), "{health}");
        assert!(health.contains("\"role\":\"companion\""), "{health}");
        let outbox = http_get(addr, "/v1/outbox");
        assert!(outbox.contains("200"), "{outbox}");
        assert!(outbox.contains("\"items\":[]"), "{outbox}");
        handle.join().expect("join");
    }

    fn http_get(addr: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(addr).expect("connect");
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).expect("write");
        let mut buf = String::new();
        stream.read_to_string(&mut buf).expect("read");
        buf
    }
}
