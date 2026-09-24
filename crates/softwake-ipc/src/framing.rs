//! Newline-delimited JSON.
//!
//! One message is one UTF-8 JSON value followed by `\n`. The writer flushes
//! after every message so the peer is not left waiting on a block buffer.
//! A length prefix would also frame the stream; lines stay readable in a log
//! and are enough for these small messages.
//!
//! [`std::io::BufRead::read_until`] leaves its buffer unspecified when the
//! read fails. A retry after `Interrupted` could repeat or drop bytes, so
//! this module fills and consumes the buffer itself.

use std::io::{self, BufRead, ErrorKind, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Largest accepted line, including the trailing newline.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// A frame that is not a single JSON value, or a socket failure while moving one.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The peer closed the connection before the next message.
    #[error("daemon disconnected")]
    Disconnected,

    /// The socket timeout elapsed before a full message arrived.
    #[error("timed out waiting for the daemon")]
    TimedOut,

    /// The bytes were not one JSON message on one line.
    #[error("protocol: {message}")]
    Protocol {
        /// What was wrong with the frame.
        message: String,
    },

    /// JSON encoding or decoding failed.
    #[error("protocol: {0}")]
    Json(#[from] serde_json::Error),

    /// The underlying read or write failed.
    #[error("io: {0}")]
    Io(io::Error),
}

impl TransportError {
    /// Frame or protocol failure with a display message.
    #[must_use]
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol {
            message: message.into(),
        }
    }

    /// Whether the peer is gone or the next read should be retried later.
    #[must_use]
    pub const fn is_disconnect(&self) -> bool {
        matches!(self, Self::Disconnected)
    }
}

impl From<io::Error> for TransportError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            ErrorKind::TimedOut | ErrorKind::WouldBlock => Self::TimedOut,
            ErrorKind::UnexpectedEof
            | ErrorKind::ConnectionReset
            | ErrorKind::BrokenPipe
            | ErrorKind::ConnectionAborted => Self::Disconnected,
            _ => Self::Io(error),
        }
    }
}

/// Write one JSON message and flush it.
///
/// # Errors
///
/// Returns [`TransportError::Json`] when `message` cannot be encoded,
/// [`TransportError::Protocol`] when the encoding contains a raw line break,
/// and [`TransportError::Io`] or [`TransportError::Disconnected`] when the
/// write fails.
pub fn write_message(
    writer: &mut impl Write,
    message: &impl Serialize,
) -> Result<(), TransportError> {
    let mut payload = serde_json::to_vec(message)?;
    if payload.contains(&b'\n') || payload.contains(&b'\r') {
        return Err(TransportError::protocol(
            "encoded message contains a line break",
        ));
    }
    payload.push(b'\n');
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// Read one JSON message.
///
/// # Errors
///
/// Returns [`TransportError::Disconnected`] at end of file,
/// [`TransportError::TimedOut`] when the socket timeout fires between
/// messages, and [`TransportError::Protocol`] or [`TransportError::Json`]
/// when the line is not one JSON value. A line longer than
/// [`MAX_MESSAGE_BYTES`] is a protocol error.
pub fn read_message<T, R>(reader: &mut R) -> Result<T, TransportError>
where
    T: DeserializeOwned,
    R: BufRead,
{
    read_message_limited(reader, MAX_MESSAGE_BYTES)
}

pub(crate) fn read_message_limited<T, R>(reader: &mut R, max: usize) -> Result<T, TransportError>
where
    T: DeserializeOwned,
    R: BufRead,
{
    let line = read_line_limited(reader, max)?;
    Ok(serde_json::from_slice(&line)?)
}

fn read_line_limited(reader: &mut impl BufRead, max: usize) -> Result<Vec<u8>, TransportError> {
    let mut buf = Vec::new();
    loop {
        let chunk = match reader.fill_buf() {
            Ok(bytes) => bytes.to_vec(),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_while_reading(error, !buf.is_empty())),
        };
        if chunk.is_empty() {
            if buf.is_empty() {
                return Err(TransportError::Disconnected);
            }
            return Err(TransportError::protocol("message missing newline"));
        }
        if let Some(index) = chunk.iter().position(|byte| *byte == b'\n') {
            let line_len = buf.len().saturating_add(index).saturating_add(1);
            reader.consume(index.saturating_add(1));
            if line_len > max {
                return Err(exceeds(max));
            }
            buf.extend_from_slice(&chunk[..=index]);
            break;
        }
        let next_len = buf.len().saturating_add(chunk.len());
        reader.consume(chunk.len());
        if next_len > max {
            return Err(exceeds(max));
        }
        buf.extend_from_slice(&chunk);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    if buf.is_empty() {
        return Err(TransportError::protocol("empty message"));
    }
    Ok(buf)
}

fn io_while_reading(error: io::Error, mid_message: bool) -> TransportError {
    if mid_message && matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) {
        return TransportError::protocol("timed out mid-message");
    }
    TransportError::from(error)
}

fn exceeds(max: usize) -> TransportError {
    TransportError::protocol(format!("message exceeds {max} bytes"))
}

#[cfg(test)]
mod tests {
    use std::io::{self, BufRead, Cursor, ErrorKind, Read};

    use super::{
        MAX_MESSAGE_BYTES, TransportError, read_line_limited, read_message_limited, write_message,
    };
    use crate::{ClientMessage, Command, Event, ResponseBody, ServerMessage, Status, VoiceState};

    #[test]
    fn round_trip_covers_commands_responses_and_state_changes() {
        let messages = [
            line(&ClientMessage::Request {
                id: 1,
                command: Command::GetStatus,
            }),
            line(&ClientMessage::Request {
                id: 2,
                command: Command::Hibernate,
            }),
            line(&ServerMessage::Response {
                id: 1,
                body: ResponseBody::ok(Status {
                    state: VoiceState::Hibernate,
                    capture_running: false,
                    soul_reload_pending: false,
                    soul: None,
                    message: None,
                    detail: Some("has\na newline".to_owned()),
                    pending_tool: None,
                    last_tool: None,
                }),
            }),
            line(&ServerMessage::Event {
                body: Event::StateChanged {
                    state: VoiceState::Hibernate,
                    previous: VoiceState::Sleep,
                    capture_running: false,
                    detail: Some("sleep -> hibernate".to_owned()),
                },
            }),
        ];
        let mut buffer = Vec::new();
        for message in &messages {
            buffer.extend_from_slice(message);
        }
        let mut cursor = Cursor::new(buffer);
        let first: ClientMessage =
            read_message_limited(&mut cursor, MAX_MESSAGE_BYTES).expect("command");
        assert!(matches!(
            first,
            ClientMessage::Request {
                command: Command::GetStatus,
                ..
            }
        ));
        let second: ClientMessage =
            read_message_limited(&mut cursor, MAX_MESSAGE_BYTES).expect("hibernate");
        assert!(matches!(
            second,
            ClientMessage::Request {
                command: Command::Hibernate,
                ..
            }
        ));
        let response: ServerMessage =
            read_message_limited(&mut cursor, MAX_MESSAGE_BYTES).expect("response");
        match response {
            ServerMessage::Response { id, body } => {
                assert_eq!(id, 1);
                let status = body.status().expect("status");
                assert_eq!(status.detail.as_deref(), Some("has\na newline"));
                assert!(!status.capture_running);
            }
            other => panic!("expected response, got {other:?}"),
        }
        let event: ServerMessage =
            read_message_limited(&mut cursor, MAX_MESSAGE_BYTES).expect("event");
        assert!(matches!(
            event,
            ServerMessage::Event {
                body: Event::StateChanged {
                    state: VoiceState::Hibernate,
                    previous: VoiceState::Sleep,
                    capture_running: false,
                    ..
                }
            }
        ));
    }

    #[test]
    fn empty_truncated_and_oversize_lines_are_protocol_errors() {
        let mut empty = Cursor::new(b"\n".as_slice());
        assert!(
            read_line_limited(&mut empty, 32)
                .expect_err("empty")
                .to_string()
                .contains("empty")
        );

        let mut crlf = Cursor::new(b"{\"a\":1}\r\n".as_slice());
        let line = read_line_limited(&mut crlf, 32).expect("crlf");
        assert_eq!(line, b"{\"a\":1}");

        let mut truncated = Cursor::new(b"{\"a\":1}".as_slice());
        assert!(
            read_line_limited(&mut truncated, 32)
                .expect_err("eof")
                .to_string()
                .contains("newline")
        );

        let mut huge = vec![b'a'; 40];
        huge.push(b'\n');
        let mut over = Cursor::new(huge);
        let error = read_line_limited(&mut over, 16).expect_err("oversize");
        assert!(error.to_string().contains("exceeds 16"));
    }

    #[test]
    fn end_of_file_and_timeout_are_distinct() {
        let mut eof = Cursor::new(b"".as_slice());
        assert!(matches!(
            read_line_limited(&mut eof, 32),
            Err(TransportError::Disconnected)
        ));

        let mut timed = TimeoutRead;
        assert!(matches!(
            read_line_limited(&mut timed, 32),
            Err(TransportError::TimedOut)
        ));
    }

    fn line(message: &impl serde::Serialize) -> Vec<u8> {
        let mut buffer = Vec::new();
        write_message(&mut buffer, message).expect("write");
        assert_eq!(buffer.last(), Some(&b'\n'));
        buffer
    }

    struct TimeoutRead;

    impl Read for TimeoutRead {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(ErrorKind::TimedOut, "timeout"))
        }
    }

    impl BufRead for TimeoutRead {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Err(io::Error::new(ErrorKind::TimedOut, "timeout"))
        }

        fn consume(&mut self, _amount: usize) {}
    }
}
