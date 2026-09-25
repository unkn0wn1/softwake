//! IPC listener and client.
//!
//! Linux uses a Unix domain socket ([ADR 0003](../../../docs/ADR-0003-ipc-transport.md)).
//! Windows uses TCP on `127.0.0.1` and stores the bound `host:port` in a port
//! file at the resolved path ([ADR 0019](../../../docs/ADR-0019-multiplatform-releases.md)).
//! Framing and the hello handshake are identical on both.

#[cfg(unix)]
#[path = "../socket_unix.rs"]
mod platform;

#[cfg(windows)]
#[path = "../socket_windows.rs"]
mod platform;

pub use platform::{
    CallError, Client, DEFAULT_CLIENT_TIMEOUT, HandshakeError, IpcStream, Listener, PendingHello,
    ServerConnection, ServerReader, ServerWriter, SocketError, connect_stream, resolve_socket_path,
    resolve_socket_path_from,
};
