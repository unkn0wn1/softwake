//! Minimal HTTP/1.1 request parse + response write.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Parsed request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Read one HTTP request (bounded).
pub fn read_request(stream: &mut TcpStream) -> std::io::Result<HttpRequest> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut buf = vec![0_u8; 64 * 1024];
    let mut filled = 0_usize;
    loop {
        if filled >= buf.len() {
            break;
        }
        match stream.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => {
                filled += n;
                if let Some(header_end) = find_header_end(&buf[..filled]) {
                    let header_bytes = &buf[..header_end];
                    let header_text = String::from_utf8_lossy(header_bytes);
                    let mut lines = header_text.lines();
                    let first = lines.next().unwrap_or("");
                    let mut parts = first.split_whitespace();
                    let method = parts.next().unwrap_or("GET").to_owned();
                    let target = parts.next().unwrap_or("/");
                    let path = target.split('?').next().unwrap_or(target).to_owned();
                    let mut headers = Vec::new();
                    let mut content_length = 0_usize;
                    for line in lines {
                        if let Some((k, v)) = line.split_once(':') {
                            let name = k.trim().to_owned();
                            let value = v.trim().to_owned();
                            if name.eq_ignore_ascii_case("content-length") {
                                content_length = value.parse().unwrap_or(0);
                            }
                            headers.push((name, value));
                        }
                    }
                    let mut body = String::new();
                    let body_start = header_end;
                    let have = filled.saturating_sub(body_start);
                    if content_length > 0 {
                        let mut body_bytes = Vec::with_capacity(content_length);
                        if have > 0 {
                            let take = have.min(content_length);
                            body_bytes.extend_from_slice(&buf[body_start..body_start + take]);
                        }
                        while body_bytes.len() < content_length {
                            let mut chunk = [0_u8; 4096];
                            let n = stream.read(&mut chunk)?;
                            if n == 0 {
                                break;
                            }
                            let need = content_length - body_bytes.len();
                            body_bytes.extend_from_slice(&chunk[..n.min(need)]);
                        }
                        body = String::from_utf8_lossy(&body_bytes).into_owned();
                    }
                    return Ok(HttpRequest {
                        method,
                        path,
                        headers,
                        body,
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
            Err(error) => return Err(error),
        }
    }
    // Fallback: treat whatever we have as headers-only GET
    let text = String::from_utf8_lossy(&buf[..filled]);
    let line = text.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_owned();
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or(target).to_owned();
    Ok(HttpRequest {
        method,
        path,
        headers: Vec::new(),
        body: String::new(),
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Write a simple response.
pub fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
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
