//! TCP localhost IPC for Windows (port file at the resolved path).
//!
//! Softwake v1 on Windows binds `127.0.0.1:0`, writes `127.0.0.1:PORT\n` to the
//! path returned by [`resolve_socket_path`], and clients read that file before
//! connecting. `SOFTWAKE_SOCKET` / `--socket` still override the path. A value
//! that looks like `host:port` (no path separators) is treated as a direct
//! address and no port file is used.

use std::fs::{self, OpenOptions};
use std::io::{self, BufReader, ErrorKind, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::framing::{TransportError, read_message, write_message};
use crate::{ClientMessage, Command, IpcError, PROTOCOL_VERSION, ServerMessage, Status};

/// File name inside the Softwake runtime directory on Windows.
const PORT_FILE_NAME: &str = "softwaked.port";

/// How long a client waits for a frame before returning [`TransportError::TimedOut`].
pub const DEFAULT_CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

const SOCKET_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

const MAX_MESSAGES_PER_CALL: u32 = 64;

/// Stream type for this platform (TCP).
pub type IpcStream = TcpStream;

/// Failure choosing or binding the daemon endpoint.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    /// `--socket` was passed without a path.
    #[error("socket path is empty")]
    EmptyPath,

    /// No default runtime directory was available.
    #[error(
        "no socket path: LOCALAPPDATA/TEMP unavailable; set SOFTWAKE_SOCKET to a port-file path or host:port"
    )]
    NoRuntimeDir,

    /// `connect` succeeded, or the probe did not finish in time.
    #[error("socket is in use at {path}; another softwaked serve is listening")]
    InUse {
        /// Endpoint another process still holds.
        path: PathBuf,
    },

    /// The path exists and is not a Softwake port file / address.
    #[error("refusing to replace {path}; it is not a Softwake IPC endpoint")]
    NotASocket {
        /// Path that was not removed.
        path: PathBuf,
    },

    /// Creating, removing, or binding failed.
    #[error("{action} {path}: {source}")]
    Io {
        /// Verb shown to the operator, such as `bind`.
        action: &'static str,
        /// Path the operation used.
        path: PathBuf,
        /// Filesystem or socket error.
        #[source]
        source: io::Error,
    },
}

impl SocketError {
    fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_owned(),
            source,
        }
    }

    fn is_transient_accept(&self) -> bool {
        match self {
            Self::Io { source, .. } => {
                matches!(
                    source.kind(),
                    ErrorKind::Interrupted | ErrorKind::ConnectionAborted
                )
            }
            _ => false,
        }
    }
}

/// Handshake or later client-call failure.
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    /// The socket path could not be used.
    #[error(transparent)]
    Socket(#[from] SocketError),

    /// A frame could not be read or written.
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// The daemon rejected the hello.
    #[error(transparent)]
    Handshake(#[from] HandshakeError),

    /// The daemon rejected the command. State is unchanged.
    #[error(transparent)]
    Rejected(IpcError),
}

/// The first message was not a compatible hello.
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    /// The client spoke a different [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION).
    #[error("protocol version {got} is not supported; expected {expected}: {message}")]
    Version {
        /// Version the client sent.
        got: u32,
        /// Version this process speaks.
        expected: u32,
        /// Text sent to the peer.
        message: String,
    },

    /// The peer closed or sent a frame that was not a hello.
    #[error(transparent)]
    Transport(#[from] TransportError),
}

impl From<io::Error> for HandshakeError {
    fn from(error: io::Error) -> Self {
        Self::Transport(TransportError::from(error))
    }
}

impl From<io::Error> for CallError {
    fn from(error: io::Error) -> Self {
        Self::Transport(TransportError::from(error))
    }
}

/// Resolve the endpoint path from the process environment.
///
/// Precedence is the explicit path, then `SOFTWAKE_SOCKET`, then
/// `%LOCALAPPDATA%/softwake/softwaked.port`, then `%TEMP%/softwake/softwaked.port`.
///
/// # Errors
///
/// Returns [`SocketError::EmptyPath`] when `explicit` is empty, and
/// [`SocketError::NoRuntimeDir`] when no directory is available.
pub fn resolve_socket_path(explicit: Option<&Path>) -> Result<PathBuf, SocketError> {
    resolve_socket_path_from(
        explicit,
        std::env::var("SOFTWAKE_SOCKET").ok().as_deref(),
        std::env::var("LOCALAPPDATA").ok().as_deref(),
        std::env::var("TEMP").ok().as_deref(),
    )
}

/// Resolve an endpoint path from already-loaded strings (testable).
///
/// `environment` is `SOFTWAKE_SOCKET`. `local_app_data` is `LOCALAPPDATA`.
/// `temp` is `TEMP`. Blank values are treated as unset.
///
/// # Errors
///
/// See [`resolve_socket_path`].
pub fn resolve_socket_path_from(
    explicit: Option<&Path>,
    environment: Option<&str>,
    local_app_data: Option<&str>,
    temp: Option<&str>,
) -> Result<PathBuf, SocketError> {
    if let Some(path) = explicit {
        if path.as_os_str().is_empty() {
            return Err(SocketError::EmptyPath);
        }
        return Ok(path.to_owned());
    }
    if let Some(path) = trimmed_nonempty(environment) {
        return Ok(PathBuf::from(path));
    }
    if let Some(base) = trimmed_nonempty(local_app_data) {
        return Ok(PathBuf::from(base).join("softwake").join(PORT_FILE_NAME));
    }
    if let Some(base) = trimmed_nonempty(temp) {
        return Ok(PathBuf::from(base).join("softwake").join(PORT_FILE_NAME));
    }
    Err(SocketError::NoRuntimeDir)
}

fn trimmed_nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// True when `path` is a direct `host:port` address (no directory separators).
fn is_direct_address(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    if text.contains('/') || text.contains('\\') {
        return false;
    }
    parse_socket_addr(text).is_ok()
}

fn parse_socket_addr(text: &str) -> io::Result<SocketAddr> {
    let mut addrs = text.to_socket_addrs()?;
    addrs
        .next()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "empty socket address"))
}

fn read_port_file(path: &Path) -> Result<SocketAddr, SocketError> {
    let text = fs::read_to_string(path).map_err(|source| SocketError::io("read", path, source))?;
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return Err(SocketError::io(
            "parse",
            path,
            io::Error::new(ErrorKind::InvalidData, "empty port file"),
        ));
    }
    parse_socket_addr(line).map_err(|source| SocketError::io("parse", path, source))
}

fn write_port_file(path: &Path, addr: SocketAddr) -> Result<(), SocketError> {
    ensure_parent(path)?;
    let temp = path.with_extension("port.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)
            .map_err(|source| SocketError::io("create port file", &temp, source))?;
        writeln!(file, "{addr}")
            .map_err(|source| SocketError::io("write port file", &temp, source))?;
        file.sync_all()
            .map_err(|source| SocketError::io("sync port file", &temp, source))?;
    }
    fs::rename(&temp, path).map_err(|source| SocketError::io("rename port file", path, source))?;
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<(), SocketError> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    fs::create_dir_all(parent)
        .map_err(|source| SocketError::io("create directory", parent, source))?;
    Ok(())
}

/// Connect a raw stream used to unblock `accept` on shutdown.
///
/// # Errors
///
/// Returns the OS connect error when the listener is gone or unreachable.
pub fn connect_stream(path: &Path) -> io::Result<IpcStream> {
    let addr = if is_direct_address(path) {
        parse_socket_addr(path.to_str().unwrap_or(""))?
    } else {
        read_port_file(path).map_err(|error| io::Error::new(ErrorKind::Other, error.to_string()))?
    };
    TcpStream::connect(addr)
}

/// Listening endpoint for `softwaked serve`.
#[derive(Debug)]
pub struct Listener {
    inner: TcpListener,
    path: PathBuf,
    replaced_stale: bool,
    uses_port_file: bool,
}

impl Listener {
    /// Bind the Softwake IPC endpoint at `path`.
    ///
    /// When `path` is a direct `host:port`, binds that address and does not
    /// write a file. Otherwise binds `127.0.0.1:0` and writes the address to
    /// the port file at `path`.
    ///
    /// # Errors
    ///
    /// See [`SocketError`].
    pub fn bind(path: &Path) -> Result<Self, SocketError> {
        if path.as_os_str().is_empty() {
            return Err(SocketError::EmptyPath);
        }
        if is_direct_address(path) {
            let addr = parse_socket_addr(path.to_str().unwrap_or(""))
                .map_err(|source| SocketError::io("parse", path, source))?;
            if probe_live_addr(addr)? {
                return Err(SocketError::InUse {
                    path: path.to_owned(),
                });
            }
            let inner = TcpListener::bind(addr).map_err(|source| {
                if source.kind() == ErrorKind::AddrInUse {
                    SocketError::InUse {
                        path: path.to_owned(),
                    }
                } else {
                    SocketError::io("bind", path, source)
                }
            })?;
            return Ok(Self {
                inner,
                path: path.to_owned(),
                replaced_stale: false,
                uses_port_file: false,
            });
        }

        let replaced_stale = prepare_bind(path)?;
        let inner = TcpListener::bind("127.0.0.1:0")
            .map_err(|source| SocketError::io("bind", path, source))?;
        let addr = inner
            .local_addr()
            .map_err(|source| SocketError::io("local_addr", path, source))?;
        if let Err(error) = write_port_file(path, addr) {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        Ok(Self {
            inner,
            path: path.to_owned(),
            replaced_stale,
            uses_port_file: true,
        })
    }

    /// Bound path (port file or direct address string).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether [`Self::bind`] deleted an abandoned port file.
    #[must_use]
    pub const fn replaced_stale(&self) -> bool {
        self.replaced_stale
    }

    /// Accept one connection.
    ///
    /// # Errors
    ///
    /// Returns [`SocketError::Io`] when accept fails for a reason the caller
    /// should see.
    pub fn accept(&self) -> Result<IpcStream, SocketError> {
        loop {
            match self.accept_once() {
                Err(error) if error.is_transient_accept() => {}
                other => return other,
            }
        }
    }

    fn accept_once(&self) -> Result<IpcStream, SocketError> {
        self.inner
            .accept()
            .map(|(stream, _address)| stream)
            .map_err(|source| SocketError::io("accept", &self.path, source))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if self.uses_port_file {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn prepare_bind(path: &Path) -> Result<bool, SocketError> {
    ensure_parent(path)?;
    if !path.exists() {
        return Ok(false);
    }
    match read_port_file(path) {
        Ok(addr) => {
            if probe_live_addr(addr)? {
                return Err(SocketError::InUse {
                    path: path.to_owned(),
                });
            }
            match fs::remove_file(path) {
                Ok(()) => Ok(true),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
                Err(error) => Err(SocketError::io("remove stale port file", path, error)),
            }
        }
        Err(_) => Err(SocketError::NotASocket {
            path: path.to_owned(),
        }),
    }
}

fn probe_live_addr(addr: SocketAddr) -> Result<bool, SocketError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::Builder::new()
        .name("softwake-socket-probe".to_owned())
        .spawn(move || {
            let _ = sender.send(TcpStream::connect(addr));
        })
        .map_err(|source| SocketError::io("probe", Path::new("tcp"), source))?;
    match receiver.recv_timeout(SOCKET_PROBE_TIMEOUT) {
        Ok(Ok(_stream)) => Ok(true),
        Ok(Err(error)) if is_absent(error.kind()) => Ok(false),
        Ok(Err(error)) => Err(SocketError::io("probe", Path::new("tcp"), error)),
        Err(_) => Ok(true),
    }
}

fn is_absent(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::ConnectionRefused | ErrorKind::NotFound | ErrorKind::TimedOut
    )
}

struct Endpoint {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Endpoint {
    fn new(stream: TcpStream) -> Result<Self, io::Error> {
        let reader_stream = stream.try_clone()?;
        Ok(Self {
            reader: BufReader::new(reader_stream),
            writer: stream,
        })
    }

    fn read<T: DeserializeOwned>(&mut self) -> Result<T, TransportError> {
        read_message(&mut self.reader)
    }

    fn write(&mut self, message: &impl Serialize) -> Result<(), TransportError> {
        write_message(&mut self.writer, message)
    }

    fn split(self) -> (BufReader<TcpStream>, TcpStream) {
        (self.reader, self.writer)
    }
}

/// A hello that matched, before [`PendingHello::accept`] writes the reply.
#[must_use = "call accept() to finish the hello exchange"]
pub struct PendingHello {
    endpoint: Endpoint,
}

impl PendingHello {
    /// Tell the client the versions match.
    ///
    /// # Errors
    ///
    /// Returns [`HandshakeError`] when the reply cannot be written.
    pub fn accept(mut self) -> Result<ServerConnection, HandshakeError> {
        self.endpoint.write(&ServerMessage::HelloOk {
            protocol_version: PROTOCOL_VERSION,
        })?;
        let (reader, writer) = self.endpoint.split();
        Ok(ServerConnection { reader, writer })
    }
}

/// One accepted client after a successful hello.
#[derive(Debug)]
pub struct ServerConnection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl ServerConnection {
    /// Read the first frame. On a version match, return a [`PendingHello`]
    /// that has not yet written `hello_ok`.
    ///
    /// # Errors
    ///
    /// Returns [`HandshakeError`] when the first frame is not a matching hello.
    pub fn begin(stream: IpcStream) -> Result<PendingHello, HandshakeError> {
        let mut endpoint = Endpoint::new(stream)?;
        let message = endpoint.read::<ClientMessage>()?;
        match message {
            ClientMessage::Hello { protocol_version } if protocol_version == PROTOCOL_VERSION => {
                Ok(PendingHello { endpoint })
            }
            ClientMessage::Hello { protocol_version } => {
                let message = format!(
                    "protocol version {protocol_version} is not supported; server speaks {PROTOCOL_VERSION}"
                );
                endpoint.write(&ServerMessage::HelloRejected {
                    protocol_version,
                    expected: PROTOCOL_VERSION,
                    message: message.clone(),
                })?;
                Err(HandshakeError::Version {
                    got: protocol_version,
                    expected: PROTOCOL_VERSION,
                    message,
                })
            }
            ClientMessage::Request { .. }
            | ClientMessage::ToolRequest { .. }
            | ClientMessage::ConfirmTool { .. }
            | ClientMessage::CancelTool { .. }
            | ClientMessage::Ask { .. }
            | ClientMessage::Wake { .. }
            | ClientMessage::TalkStart { .. }
            | ClientMessage::TalkStop { .. }
            | ClientMessage::SetVoiceTest { .. }
            | ClientMessage::ReloadKws { .. } => {
                let message = "expected a hello message".to_owned();
                endpoint.write(&ServerMessage::HelloRejected {
                    protocol_version: PROTOCOL_VERSION,
                    expected: PROTOCOL_VERSION,
                    message: message.clone(),
                })?;
                Err(HandshakeError::Transport(TransportError::protocol(message)))
            }
        }
    }

    /// Read a hello and answer it when the versions match.
    ///
    /// # Errors
    ///
    /// Returns [`HandshakeError`] when the first frame is not a matching hello.
    pub fn handshake(stream: IpcStream) -> Result<Self, HandshakeError> {
        Self::begin(stream)?.accept()
    }

    /// Read the next client message.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the frame is missing or invalid.
    pub fn read(&mut self) -> Result<ClientMessage, TransportError> {
        read_message(&mut self.reader)
    }

    /// Write one daemon message.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the write fails.
    pub fn write(&mut self, message: &ServerMessage) -> Result<(), TransportError> {
        write_message(&mut self.writer, message)
    }

    /// Split the connection so one thread can read while another writes.
    #[must_use]
    pub fn split(self) -> (ServerReader, ServerWriter) {
        (
            ServerReader {
                reader: self.reader,
            },
            ServerWriter {
                writer: self.writer,
            },
        )
    }
}

/// Read half of a [`ServerConnection`].
#[derive(Debug)]
pub struct ServerReader {
    reader: BufReader<TcpStream>,
}

impl ServerReader {
    /// Read the next client message.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the frame is missing or invalid.
    pub fn read(&mut self) -> Result<ClientMessage, TransportError> {
        read_message(&mut self.reader)
    }
}

/// Write half of a [`ServerConnection`].
#[derive(Debug)]
pub struct ServerWriter {
    writer: TcpStream,
}

impl ServerWriter {
    /// Write one daemon message.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the write fails.
    pub fn write(&mut self, message: &ServerMessage) -> Result<(), TransportError> {
        write_message(&mut self.writer, message)
    }
}

/// Connected client that has finished the hello exchange.
#[derive(Debug)]
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl Client {
    /// Connect, send hello, and wait for [`ServerMessage::HelloOk`].
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when the daemon is absent, too slow, or speaks
    /// another protocol version.
    pub fn connect(path: &Path) -> Result<Self, CallError> {
        let stream =
            connect_stream(path).map_err(|source| SocketError::io("connect", path, source))?;
        stream.set_read_timeout(Some(DEFAULT_CLIENT_TIMEOUT))?;
        stream.set_write_timeout(Some(DEFAULT_CLIENT_TIMEOUT))?;
        let mut endpoint = Endpoint::new(stream)?;
        endpoint.write(&ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        })?;
        match endpoint.read::<ServerMessage>()? {
            ServerMessage::HelloOk { protocol_version } if protocol_version == PROTOCOL_VERSION => {
                let (reader, writer) = endpoint.split();
                Ok(Self {
                    reader,
                    writer,
                    next_id: 0,
                })
            }
            ServerMessage::HelloOk { protocol_version } => Err(HandshakeError::Version {
                got: protocol_version,
                expected: PROTOCOL_VERSION,
                message: format!(
                    "daemon speaks protocol {protocol_version}; this client speaks {PROTOCOL_VERSION}"
                ),
            }
            .into()),
            ServerMessage::HelloRejected {
                protocol_version,
                expected,
                message,
            } => Err(HandshakeError::Version {
                got: protocol_version,
                expected,
                message,
            }
            .into()),
            _ => Err(TransportError::protocol("expected a hello response").into()),
        }
    }

    /// Change the read timeout. `None` waits without a deadline.
    ///
    /// # Errors
    ///
    /// Returns the socket error when the option cannot be set.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), CallError> {
        self.reader.get_ref().set_read_timeout(timeout)?;
        Ok(())
    }

    /// Send one command and return its status.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure, a mismatched response id,
    /// or a rejected command.
    pub fn call(&mut self, command: Command) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::Request { id, command }, id)
    }

    /// Send one tool request and return its status.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure, a mismatched response id,
    /// or a rejected tool call.
    pub fn call_tool(&mut self, name: &str, args: &[String]) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(
            &ClientMessage::ToolRequest {
                id,
                name: name.to_owned(),
                args: args.to_vec(),
            },
            id,
        )
    }

    /// Send one wake and return its status.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure, a mismatched response id,
    /// or a rejected wake.
    pub fn call_wake(&mut self) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::Wake { id }, id)
    }

    /// Send one ask and return its status.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure, a mismatched response id,
    /// or a rejected ask.
    pub fn call_ask(&mut self, text: &str) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(
            &ClientMessage::Ask {
                id,
                text: text.to_owned(),
            },
            id,
        )
    }

    /// Arm press-to-talk on the daemon.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure or a rejected start.
    pub fn call_talk_start(&mut self) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::TalkStart { id }, id)
    }

    /// Release press-to-talk and return the ask status.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure or a rejected stop.
    pub fn call_talk_stop(&mut self) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::TalkStop { id }, id)
    }

    /// Turn voice test mode on or off and return the status.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure or a rejected update.
    pub fn set_voice_test(&mut self, enabled: bool) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::SetVoiceTest { id, enabled }, id)
    }

    /// Rebuild the KWS detector from current `softwake.json` / env thresholds.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure or a rejected reload.
    pub fn reload_kws(&mut self) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::ReloadKws { id }, id)
    }

    /// Confirm the pending tool and return the status after it runs.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure, a mismatched response id,
    /// or a rejected confirmation.
    pub fn confirm_tool(&mut self, pending_id: &str) -> Result<Status, CallError> {
        self.confirm_tool_named(pending_id, None)
    }

    /// [`Self::confirm_tool`] with an optional name check.
    ///
    /// # Errors
    ///
    /// See [`Self::confirm_tool`].
    pub fn confirm_tool_named(
        &mut self,
        pending_id: &str,
        name: Option<&str>,
    ) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(
            &ClientMessage::ConfirmTool {
                id,
                pending_id: pending_id.to_owned(),
                name: name.map(str::to_owned),
            },
            id,
        )
    }

    /// Clear the pending confirmation without running the tool.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure, a mismatched response id,
    /// or an unknown pending id.
    pub fn cancel_tool(&mut self, pending_id: &str) -> Result<Status, CallError> {
        self.cancel_tool_named(pending_id, None)
    }

    /// [`Self::cancel_tool`] with an optional name check.
    ///
    /// # Errors
    ///
    /// See [`Self::cancel_tool`].
    pub fn cancel_tool_named(
        &mut self,
        pending_id: &str,
        name: Option<&str>,
    ) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(
            &ClientMessage::CancelTool {
                id,
                pending_id: pending_id.to_owned(),
                name: name.map(str::to_owned),
            },
            id,
        )
    }

    fn allocate_id(&mut self) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        self.next_id
    }

    fn round_trip(&mut self, message: &ClientMessage, id: u64) -> Result<Status, CallError> {
        write_message(&mut self.writer, message)?;
        for _ in 0..MAX_MESSAGES_PER_CALL {
            match read_message::<ServerMessage, _>(&mut self.reader)? {
                ServerMessage::Response {
                    id: response_id,
                    body,
                } if response_id == id => {
                    return match body {
                        crate::ResponseBody::Ok { snapshot } => Ok(snapshot),
                        crate::ResponseBody::Err { error } => Err(CallError::Rejected(error)),
                    };
                }
                ServerMessage::Response {
                    id: response_id, ..
                } => {
                    return Err(TransportError::protocol(format!(
                        "response id {response_id} does not match request {id}"
                    ))
                    .into());
                }
                ServerMessage::Event { .. } => {}
                ServerMessage::HelloOk { .. } | ServerMessage::HelloRejected { .. } => {
                    return Err(TransportError::protocol(
                        "unexpected hello while waiting for a response",
                    )
                    .into());
                }
            }
        }
        Err(TransportError::protocol("too many messages before a response").into())
    }

    /// Read the next daemon message, including events.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when the frame cannot be read.
    pub fn read(&mut self) -> Result<ServerMessage, CallError> {
        Ok(read_message(&mut self.reader)?)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use super::{
        Client, Listener, PORT_FILE_NAME, ServerConnection, SocketError, resolve_socket_path,
        resolve_socket_path_from,
    };
    use crate::{
        ClientMessage, Command, PROTOCOL_VERSION, ResponseBody, ServerMessage, Status, VoiceState,
    };

    struct TempEndpoint {
        dir: PathBuf,
        path: PathBuf,
    }

    impl TempEndpoint {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("sw-ipc-{}-{n}", std::process::id()));
            fs::create_dir_all(&dir).expect("temp dir");
            Self {
                path: dir.join("s.port"),
                dir,
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempEndpoint {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn path_precedence_is_flag_then_env_then_localappdata_then_temp() {
        let explicit = Path::new(r"C:\explicit.port");
        assert_eq!(
            resolve_socket_path_from(
                Some(explicit),
                Some(r"C:\env.port"),
                Some(r"C:\lad"),
                Some(r"C:\tmp")
            )
            .expect("explicit"),
            explicit
        );
        assert_eq!(
            resolve_socket_path_from(
                None,
                Some(r"  C:\from-env.port  "),
                Some(r"C:\lad"),
                Some(r"C:\tmp")
            )
            .expect("env"),
            PathBuf::from(r"C:\from-env.port")
        );
        assert_eq!(
            resolve_socket_path_from(
                None,
                Some("   "),
                Some(r"C:\Users\a\AppData\Local"),
                Some(r"C:\tmp")
            )
            .expect("lad"),
            PathBuf::from(format!(
                r"C:\Users\a\AppData\Local\softwake\{PORT_FILE_NAME}"
            ))
        );
        assert_eq!(
            resolve_socket_path_from(None, None, None, Some(r"C:\Temp")).expect("temp"),
            PathBuf::from(format!(r"C:\Temp\softwake\{PORT_FILE_NAME}"))
        );
        assert!(matches!(
            resolve_socket_path_from(None, None, Some(""), None),
            Err(SocketError::NoRuntimeDir)
        ));
    }

    #[test]
    fn explicit_resolve_ignores_the_process_environment() {
        let path = Path::new(r"C:\softwake-explicit.port");
        assert_eq!(resolve_socket_path(Some(path)).expect("path"), path);
    }

    #[test]
    fn bind_writes_port_file_and_handshake_round_trips() {
        let temp = TempEndpoint::new();
        let path = temp.path().to_owned();
        let listener = Listener::bind(&path).expect("bind");
        assert!(path.exists());
        let contents = fs::read_to_string(&path).expect("port file");
        assert!(contents.contains("127.0.0.1:"), "{contents}");

        let server = thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let mut connection = ServerConnection::handshake(stream).expect("hello");
            let request = connection.read().expect("request");
            let ClientMessage::Request { id, command } = request else {
                panic!("expected request, got {request:?}");
            };
            assert_eq!(command, Command::GetStatus);
            connection
                .write(&ServerMessage::Response {
                    id,
                    body: ResponseBody::ok(Status {
                        state: VoiceState::Sleep,
                        capture_running: true,
                        capture_level: None,
                        soul_reload_pending: false,
                        soul: None,
                        message: None,
                        detail: None,
                        pending_tool: None,
                        last_tool: None,
                        talking: false,
                        auto_listening: false,
                        context_used: None,
                        context_limit: None,
                        context_compacted: false,
                        voice_test: false,
                    }),
                })
                .expect("response");
            let _held = listener;
        });

        let mut client = Client::connect(&path).expect("connect");
        let status = client.call(Command::GetStatus).expect("status");
        assert_eq!(status.state, VoiceState::Sleep);
        server.join().expect("server");
    }

    #[test]
    fn bind_refuses_to_replace_a_regular_file() {
        let temp = TempEndpoint::new();
        fs::write(temp.path(), b"not a port file").expect("file");
        let error = Listener::bind(temp.path()).expect_err("file");
        assert!(matches!(error, SocketError::NotASocket { .. }));
        assert_eq!(fs::read(temp.path()).expect("kept"), b"not a port file");
    }
}
