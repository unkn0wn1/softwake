//! Daemon and UI protocol.
//!
//! A client sends [`ClientMessage`] values and the daemon replies with
//! [`ServerMessage`] values. Each value is one line of JSON. See [`framing`]
//! for the line rules and [`socket`] for the path and the version handshake.
//!
//! On Linux the transport is a Unix domain socket ([ADR 0003] and
//! [ADR 0019](../../docs/ADR-0019-multiplatform-releases.md)). On Windows the
//! same framing rides TCP localhost with a port file at the resolved path.
//!
//! `set_config` is intentionally absent until the daemon can validate a
//! configuration document.

mod framing;
mod socket;
mod types;

pub use framing::{MAX_MESSAGE_BYTES, TransportError, read_message, write_message};
pub use socket::{
    CallError, Client, DEFAULT_CLIENT_TIMEOUT, HandshakeError, IpcStream, Listener, PendingHello,
    ServerConnection, ServerReader, ServerWriter, SocketError, connect_stream, resolve_socket_path,
    resolve_socket_path_from,
};
pub use types::{
    ClientMessage, Command, Event, IpcError, PROTOCOL_VERSION, PendingTool, ResponseBody,
    ServerMessage, SoulReport, Status, VoiceState,
};
