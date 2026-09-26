//! Unix domain socket path, version handshake, and blocking reads (Linux).
//!
//! The default path is `$XDG_RUNTIME_DIR/softwake/softwaked.sock`. When
//! `XDG_RUNTIME_DIR` is unset, the fallback is `/tmp/softwake-$UID/softwaked.sock`.
//! The uid is the owner of `/proc/self`. `SOFTWAKE_SOCKET` and an explicit path
//! override both. Windows uses TCP localhost; see `socket_windows`.
//!
//! A hello message is the first frame on every connection. A version other
//! than [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION) is rejected and the
//! connection closes.

use std::fs::{self, Permissions};
use std::io::{self, BufReader, ErrorKind};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};

/// Stream type for this platform (Unix domain socket).
pub type IpcStream = UnixStream;

/// Connect a raw stream used to unblock `accept` on shutdown.
///
/// # Errors
///
/// Returns the OS connect error when the listener is gone or unreachable.
pub fn connect_stream(path: &Path) -> io::Result<IpcStream> {
    UnixStream::connect(path)
}
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::framing::{TransportError, read_message, write_message};
use crate::{ClientMessage, Command, IpcError, PROTOCOL_VERSION, ServerMessage, Status};

/// File name inside the softwake runtime directory.
const SOCKET_FILE_NAME: &str = "softwaked.sock";

/// How long a client waits for a frame before returning [`TransportError::TimedOut`].
pub const DEFAULT_CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long startup waits while checking whether an existing socket is live.
///
/// A live peer accepts quickly. A stale file is refused immediately. A peer
/// that neither accepts nor refuses is treated as live so startup does not
/// delete a socket it could not prove was abandoned.
const SOCKET_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// How many non-response frames [`Client::call`] will skip while waiting.
const MAX_MESSAGES_PER_CALL: u32 = 64;

/// Failure choosing or binding the daemon socket.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    /// `--socket` was passed without a path.
    #[error("socket path is empty")]
    EmptyPath,

    /// Neither `XDG_RUNTIME_DIR` nor a Linux uid was available.
    #[error(
        "no socket path: XDG_RUNTIME_DIR is unset and the process uid could not be read; set SOFTWAKE_SOCKET"
    )]
    NoRuntimeDir,

    /// `connect` succeeded, or the probe did not finish in time.
    #[error("socket is in use at {path}; another softwaked serve is listening")]
    InUse {
        /// Socket another process still holds.
        path: PathBuf,
    },

    /// The path exists and is not a socket, so it is left untouched.
    #[error("refusing to replace {path}; it is not a Softwake IPC endpoint")]
    NotASocket {
        /// Path that was not removed.
        path: PathBuf,
    },

    /// Creating, removing, or binding the socket failed.
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

/// Resolve the socket path from the process environment.
///
/// Precedence is the explicit path, then `SOFTWAKE_SOCKET`, then
/// `XDG_RUNTIME_DIR`, then `/tmp/softwake-$UID/softwaked.sock`.
///
/// # Errors
///
/// Returns [`SocketError::EmptyPath`] when `explicit` is empty, and
/// [`SocketError::NoRuntimeDir`] when no variable and no uid are available.
pub fn resolve_socket_path(explicit: Option<&Path>) -> Result<PathBuf, SocketError> {
    resolve_socket_path_from(
        explicit,
        std::env::var("SOFTWAKE_SOCKET").ok().as_deref(),
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        current_uid(),
    )
}

/// Resolve a socket path from already-loaded strings.
///
/// `environment` is `SOFTWAKE_SOCKET`. `xdg_runtime_dir` is `XDG_RUNTIME_DIR`.
/// Blank variable values are treated as unset. A blank explicit path is an
/// error because the caller asked for a path and did not give one.
///
/// # Errors
///
/// See [`resolve_socket_path`].
pub fn resolve_socket_path_from(
    explicit: Option<&Path>,
    environment: Option<&str>,
    xdg_runtime_dir: Option<&str>,
    uid: Option<u32>,
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
    if let Some(runtime) = trimmed_nonempty(xdg_runtime_dir) {
        return Ok(PathBuf::from(runtime)
            .join("softwake")
            .join(SOCKET_FILE_NAME));
    }
    let Some(uid) = uid else {
        return Err(SocketError::NoRuntimeDir);
    };
    Ok(PathBuf::from(format!("/tmp/softwake-{uid}")).join(SOCKET_FILE_NAME))
}

fn trimmed_nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata("/proc/self").ok().map(|meta| meta.uid())
}

/// Listening socket for `softwaked serve`.
///
/// Dropping the listener removes the socket file. A crashed process skips
/// that drop; the next [`Listener::bind`] removes the file when nothing
/// accepts connections on it.
#[derive(Debug)]
pub struct Listener {
    inner: UnixListener,
    path: PathBuf,
    replaced_stale: bool,
}

impl Listener {
    /// Bind `path`, creating the parent directory when it is missing.
    ///
    /// A socket file whose peer refuses the connection is stale and is
    /// removed. A peer that accepts, or that does not answer the probe, keeps
    /// the file and this returns [`SocketError::InUse`]. A non-socket file is
    /// left in place.
    ///
    /// The parent directory is mode `0700` when this call creates it. The
    /// socket itself is mode `0600`.
    ///
    /// # Errors
    ///
    /// See [`SocketError`].
    pub fn bind(path: &Path) -> Result<Self, SocketError> {
        let replaced_stale = prepare_bind(path)?;
        let inner = UnixListener::bind(path).map_err(|source| {
            // Another process won the race between the probe and bind.
            if source.kind() == ErrorKind::AddrInUse {
                SocketError::InUse {
                    path: path.to_owned(),
                }
            } else {
                SocketError::io("bind", path, source)
            }
        })?;
        fs::set_permissions(path, Permissions::from_mode(0o600)).map_err(|source| {
            let _ = fs::remove_file(path);
            SocketError::io("set socket mode", path, source)
        })?;
        Ok(Self {
            inner,
            path: path.to_owned(),
            replaced_stale,
        })
    }

    /// Bound path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether [`Self::bind`] deleted an abandoned socket file.
    #[must_use]
    pub const fn replaced_stale(&self) -> bool {
        self.replaced_stale
    }

    /// Accept one connection.
    ///
    /// Interrupted and aborted accepts are retried. Other failures are returned.
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
        let _ = fs::remove_file(&self.path);
    }
}

fn prepare_bind(path: &Path) -> Result<bool, SocketError> {
    ensure_parent(path)?;
    if !path.exists() {
        return Ok(false);
    }
    let metadata = fs::metadata(path).map_err(|source| SocketError::io("stat", path, source))?;
    if !metadata.file_type().is_socket() {
        return Err(SocketError::NotASocket {
            path: path.to_owned(),
        });
    }
    if probe_live(path)? {
        return Err(SocketError::InUse {
            path: path.to_owned(),
        });
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SocketError::io("remove stale socket", path, error)),
    }
}

fn ensure_parent(path: &Path) -> Result<(), SocketError> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    let created = !parent.exists();
    fs::create_dir_all(parent)
        .map_err(|source| SocketError::io("create directory", parent, source))?;
    if created {
        fs::set_permissions(parent, Permissions::from_mode(0o700))
            .map_err(|source| SocketError::io("set directory mode", parent, source))?;
    }
    Ok(())
}

fn probe_live(path: &Path) -> Result<bool, SocketError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let probe_path = path.to_owned();
    thread::Builder::new()
        .name("softwake-socket-probe".to_owned())
        .spawn(move || {
            let _ = sender.send(UnixStream::connect(probe_path));
        })
        .map_err(|source| SocketError::io("probe", path, source))?;
    match receiver.recv_timeout(SOCKET_PROBE_TIMEOUT) {
        Ok(Ok(_stream)) => Ok(true),
        Ok(Err(error)) if is_absent(error.kind()) => Ok(false),
        Ok(Err(error)) => Err(SocketError::io("probe", path, error)),
        // Still unanswered: do not delete a socket that might be live.
        Err(_) => Ok(true),
    }
}

fn is_absent(kind: ErrorKind) -> bool {
    matches!(kind, ErrorKind::ConnectionRefused | ErrorKind::NotFound)
}

struct Endpoint {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Endpoint {
    fn new(stream: UnixStream) -> Result<Self, io::Error> {
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

    fn split(self) -> (BufReader<UnixStream>, UnixStream) {
        (self.reader, self.writer)
    }
}

/// A hello that matched, before [`PendingHello::accept`] writes the reply.
///
/// Dropping this without [`PendingHello::accept`] closes the connection.
/// The daemon subscribes the client to events before accepting so a state
/// change cannot land in the gap after the client observes `hello_ok`.
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
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl ServerConnection {
    /// Read the first frame. On a version match, return a [`PendingHello`]
    /// that has not yet written `hello_ok`.
    ///
    /// A mismatch writes [`ServerMessage::HelloRejected`] and returns
    /// [`HandshakeError::Version`]. The caller drops the connection.
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
            | ClientMessage::ReloadKws { .. }
            | ClientMessage::ReloadUtterance { .. }
            | ClientMessage::ReloadPlayback { .. } => {
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
    reader: BufReader<UnixStream>,
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
    writer: UnixStream,
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
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Client {
    /// Connect, send hello, and wait for [`ServerMessage::HelloOk`].
    ///
    /// The socket read and write timeouts are [`DEFAULT_CLIENT_TIMEOUT`].
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when the daemon is absent, too slow, or speaks
    /// another protocol version.
    pub fn connect(path: &Path) -> Result<Self, CallError> {
        let stream =
            UnixStream::connect(path).map_err(|source| SocketError::io("connect", path, source))?;
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
    /// Events that arrive before the matching response are skipped. A command
    /// error is [`CallError::Rejected`] and does not change the daemon.
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
    /// Events that arrive before the matching response are skipped. A refusal
    /// is [`CallError::Rejected`]. The daemon runs the tool only while awake.
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
    /// Events that arrive before the matching response are skipped. A refusal
    /// is [`CallError::Rejected`]. The daemon enters awake only through
    /// `wake_phrase`.
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
    /// Events that arrive before the matching response are skipped. A refusal
    /// is [`CallError::Rejected`]. The daemon completes one turn only while awake.
    /// Assistant text is [`Status::message`]. The bearer is not part of the frame.
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

    /// Re-read free-speech end silence from `softwake.json` / env.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure or a rejected reload.
    pub fn reload_utterance(&mut self) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::ReloadUtterance { id }, id)
    }

    /// Re-read the TTS playback reaper deadline from `softwake.json` / env.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] on transport failure or a rejected reload.
    pub fn reload_playback(&mut self) -> Result<Status, CallError> {
        let id = self.allocate_id();
        self.round_trip(&ClientMessage::ReloadPlayback { id }, id)
    }

    /// Confirm the pending tool and return the status after it runs.
    ///
    /// Events that arrive before the matching response are skipped. A refusal
    /// is [`CallError::Rejected`]. The daemon runs the tool only while awake.
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
    /// See [`Self::confirm_tool`]. A name that does not match the pending tool
    /// is [`CallError::Rejected`].
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
    use std::io::BufReader;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use super::{
        Client, HandshakeError, Listener, SOCKET_FILE_NAME, ServerConnection, SocketError,
        resolve_socket_path, resolve_socket_path_from,
    };
    use crate::framing::{read_message, write_message};
    use crate::{
        ClientMessage, Command, Event, IpcError, PROTOCOL_VERSION, ResponseBody, ServerMessage,
        Status, VoiceState,
    };

    struct TempSocket {
        dir: PathBuf,
        path: PathBuf,
    }

    impl TempSocket {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("sw-ipc-{}-{n}", std::process::id()));
            fs::create_dir_all(&dir).expect("temp dir");
            Self {
                path: dir.join("s"),
                dir,
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempSocket {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn path_precedence_is_flag_then_env_then_xdg_then_uid() {
        let explicit = Path::new("/var/run/explicit.sock");
        assert_eq!(
            resolve_socket_path_from(Some(explicit), Some("/env.sock"), Some("/xdg"), Some(9))
                .expect("explicit"),
            explicit
        );
        assert_eq!(
            resolve_socket_path_from(None, Some("  /from-env.sock  "), Some("/xdg"), Some(9))
                .expect("env"),
            PathBuf::from("/from-env.sock")
        );
        assert_eq!(
            resolve_socket_path_from(None, Some("   "), Some("/run/user/1"), Some(9)).expect("xdg"),
            PathBuf::from(format!("/run/user/1/softwake/{SOCKET_FILE_NAME}"))
        );
        assert_eq!(
            resolve_socket_path_from(None, None, None, Some(1000)).expect("uid"),
            PathBuf::from(format!("/tmp/softwake-1000/{SOCKET_FILE_NAME}"))
        );
        assert!(matches!(
            resolve_socket_path_from(None, None, Some(""), None),
            Err(SocketError::NoRuntimeDir)
        ));
        assert!(matches!(
            resolve_socket_path_from(Some(Path::new("")), Some("/env"), Some("/xdg"), Some(1)),
            Err(SocketError::EmptyPath)
        ));
    }

    #[test]
    fn explicit_resolve_ignores_the_process_environment() {
        let path = Path::new("/tmp/softwake-explicit.sock");
        assert_eq!(resolve_socket_path(Some(path)).expect("path"), path);
    }

    #[test]
    fn bind_sets_modes_replaces_a_stale_socket_and_refuses_a_live_one() {
        let temp = TempSocket::new();
        let nested = temp.dir.join("nested").join("s");
        let listener = Listener::bind(&nested).expect("bind");
        assert!(!listener.replaced_stale());
        let parent_mode = fs::metadata(nested.parent().expect("parent"))
            .expect("parent meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, 0o700);
        let socket_mode = fs::metadata(&nested)
            .expect("socket meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(socket_mode, 0o600);

        let stale_path = temp.path().to_owned();
        let stale = UnixListener::bind(&stale_path).expect("stale listener");
        drop(stale);
        let replaced = Listener::bind(&stale_path).expect("replace stale");
        assert!(replaced.replaced_stale());
        let error = Listener::bind(&stale_path).expect_err("live");
        assert!(matches!(error, SocketError::InUse { .. }), "{error}");
        drop(replaced);
        drop(listener);
    }

    #[test]
    fn bind_refuses_to_replace_a_regular_file() {
        let temp = TempSocket::new();
        fs::write(temp.path(), b"not a socket").expect("file");
        let error = Listener::bind(temp.path()).expect_err("file");
        assert!(matches!(error, SocketError::NotASocket { .. }));
        assert_eq!(fs::read(temp.path()).expect("kept"), b"not a socket");
    }

    #[test]
    fn handshake_round_trip_skips_events_and_rejects_a_mismatch() {
        let temp = TempSocket::new();
        let path = temp.path().to_owned();
        let listener = Listener::bind(&path).expect("bind");
        let server = thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let mut connection = ServerConnection::handshake(stream).expect("hello");
            let request = connection.read().expect("request");
            let ClientMessage::Request { id, command } = request else {
                panic!("expected request, got {request:?}");
            };
            assert_eq!(command, Command::GetStatus);
            connection
                .write(&ServerMessage::Event {
                    body: Event::StateChanged {
                        state: VoiceState::Sleep,
                        previous: VoiceState::Sleep,
                        capture_running: true,
                        detail: None,
                    },
                })
                .expect("event");
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
        assert!(status.capture_running);
        server.join().expect("server");

        let listener = Listener::bind(&path).expect("rebind");
        let server = thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let error = ServerConnection::handshake(stream).expect_err("mismatch");
            assert!(matches!(
                error,
                HandshakeError::Version {
                    got: 99,
                    expected: PROTOCOL_VERSION,
                    ..
                }
            ));
        });
        let stream = UnixStream::connect(&path).expect("connect raw");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        write_message(
            &mut writer,
            &ClientMessage::Hello {
                protocol_version: 99,
            },
        )
        .expect("hello");
        let reply: ServerMessage = read_message(&mut reader).expect("rejected");
        assert!(matches!(
            reply,
            ServerMessage::HelloRejected {
                protocol_version: 99,
                expected: PROTOCOL_VERSION,
                ..
            }
        ));
        server.join().expect("reject server");
    }

    #[test]
    fn rejected_commands_surface_as_call_errors() {
        let temp = TempSocket::new();
        let path = temp.path().to_owned();
        let listener = Listener::bind(&path).expect("bind");
        let server = thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let mut connection = ServerConnection::handshake(stream).expect("hello");
            let ClientMessage::Request { id, .. } = connection.read().expect("request") else {
                panic!("request");
            };
            connection
                .write(&ServerMessage::Response {
                    id,
                    body: ResponseBody::Err {
                        error: IpcError::IllegalTransition {
                            from: VoiceState::Sleep,
                            command: Command::Sleep,
                            reason: "already asleep".to_owned(),
                        },
                    },
                })
                .expect("write");
        });
        let mut client = Client::connect(&path).expect("connect");
        let error = client.call(Command::Sleep).expect_err("rejected");
        assert_eq!(
            error.to_string(),
            "cannot apply sleep from sleep: already asleep"
        );
        server.join().expect("server");
    }
}
