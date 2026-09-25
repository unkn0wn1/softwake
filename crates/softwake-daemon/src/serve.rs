//! Blocking Unix-socket server for [`crate::runtime::Runtime`].
//!
//! Phase 1 stays on std threads. Each client has a reader and a writer so one
//! slow UI cannot stop another client's request from being applied. Events are
//! queued per client; a full queue drops that event for that client and the
//! next `get_status` shows the truth.
//!
//! The listener is removed when this task drops. A crash skips that drop, and
//! the next bind deletes the file if nothing answers on it.

use std::io::Error as IoError;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use softwake_ipc::{
    ClientMessage, Event as WireEvent, HandshakeError, Listener, PROTOCOL_VERSION, ResponseBody,
    ServerConnection, ServerMessage, ServerReader, ServerWriter, SocketError, resolve_socket_path,
};

use softwake_soul::SoulDir;

use crate::runtime::{Outcome, Runtime};

const OUTBOUND_CAPACITY: usize = 32;

/// Failure to bind or to keep the accept thread alive.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ServeError {
    /// The socket could not be chosen or bound.
    #[error(transparent)]
    Socket(#[from] SocketError),

    /// The process could not start the accept thread.
    #[error("spawn accept thread: {0}")]
    Spawn(IoError),

    /// The accept thread panicked.
    #[error("serve thread panicked")]
    Panicked,
}

/// Bind, print the listening line, and block until the accept thread ends.
///
/// # Errors
///
/// Returns [`ServeError`] when the socket is unavailable or the accept thread
/// panics. A peer that still holds the socket produces [`SocketError::InUse`].
pub(crate) fn run(socket: Option<&Path>, soul_dir: SoulDir) -> Result<(), ServeError> {
    let path = resolve_socket_path(socket)?;
    let handle = spawn(path.clone(), soul_dir)?;
    println!("softwaked serve");
    println!("listening: {}", path.display());
    println!("protocol: {PROTOCOL_VERSION}");
    handle.wait()
}

/// Bind `path` and accept clients until the handle is dropped.
///
/// # Errors
///
/// Returns [`ServeError`] when the socket cannot be bound.
pub(crate) fn spawn(path: PathBuf, soul_dir: SoulDir) -> Result<ServeHandle, ServeError> {
    let listener = Listener::bind(&path)?;
    if listener.replaced_stale() {
        eprintln!("softwaked: removed stale socket {}", path.display());
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shutdown);
    let shared = Arc::new(Shared::new(soul_dir));
    #[cfg(test)]
    let shared_for_handle = Arc::clone(&shared);
    let join = thread::Builder::new()
        .name("softwake-accept".to_owned())
        .spawn(move || accept_loop(&listener, &flag, &shared))
        .map_err(ServeError::Spawn)?;
    Ok(ServeHandle {
        shutdown,
        join: Some(join),
        path,
        #[cfg(test)]
        shared: shared_for_handle,
    })
}

pub(crate) struct ServeHandle {
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<(), ServeError>>>,
    path: PathBuf,
    /// Shared with the accept thread so tests can enter awake without a mic.
    #[cfg(test)]
    shared: Arc<Shared>,
}

impl std::fmt::Debug for ServeHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServeHandle")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ServeHandle {
    /// Block until the accept loop returns.
    ///
    /// Drop the handle instead when a test needs the loop to stop.
    ///
    /// # Errors
    ///
    /// Returns [`ServeError::Panicked`] when the accept thread panics, or the
    /// accept error when `accept` fails while the server should stay up.
    pub(crate) fn wait(mut self) -> Result<(), ServeError> {
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        match join.join() {
            Ok(result) => result,
            Err(_panic) => Err(ServeError::Panicked),
        }
    }

    /// Enter awake by calling [`Runtime::wake_phrase`] directly.
    ///
    /// The socket uses the same method. This path does not broadcast. A
    /// watcher that connects afterward still sees the next tool or ask event.
    #[cfg(test)]
    pub(crate) fn wake_phrase_for_test(&self) -> Outcome {
        lock(&self.shared.runtime).wake_phrase()
    }

    /// Install the in-test provider on the running daemon.
    #[cfg(test)]
    pub(crate) fn install_chat_fixture_for_test(&self, fixture: crate::chat::ChatFixture) {
        lock(&self.shared.runtime).install_chat_fixture(fixture);
    }

    /// Posts recorded by the in-test transport.
    #[cfg(test)]
    pub(crate) fn chat_posts_for_test(&self) -> Vec<crate::chat::RecordedPost> {
        lock(&self.shared.runtime).chat_posts()
    }

    /// Instructions stored on the open session, if it is open.
    #[cfg(test)]
    pub(crate) fn session_instructions_for_test(&self) -> Option<String> {
        lock(&self.shared.runtime)
            .session_instructions()
            .map(str::to_owned)
    }

    /// User lines recorded since the session opened.
    #[cfg(test)]
    pub(crate) fn session_turns_for_test(&self) -> Vec<String> {
        lock(&self.shared.runtime).session_turns().to_vec()
    }
}

impl Drop for ServeHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Unblock `accept`. Failure is fine when the listener is already gone.
        let _ = UnixStream::connect(&self.path);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn accept_loop(
    listener: &Listener,
    shutdown: &AtomicBool,
    shared: &Arc<Shared>,
) -> Result<(), ServeError> {
    let result = loop {
        if shutdown.load(Ordering::SeqCst) {
            break Ok(());
        }
        match listener.accept() {
            Ok(stream) => {
                if shutdown.load(Ordering::SeqCst) {
                    break Ok(());
                }
                shared.spawn_client(stream);
            }
            Err(error) => {
                if shutdown.load(Ordering::SeqCst) {
                    break Ok(());
                }
                break Err(ServeError::Socket(error));
            }
        }
    };
    shared.stop_clients();
    result
}

struct Shared {
    runtime: Mutex<Runtime>,
    subscribers: Mutex<Vec<Subscriber>>,
    clients: Mutex<Vec<ClientSlot>>,
    next_subscriber: AtomicU64,
}

struct Subscriber {
    id: u64,
    tx: SyncSender<Outbound>,
}

struct ClientSlot {
    shutdown: UnixStream,
    thread: JoinHandle<()>,
}

enum Outbound {
    Response { id: u64, body: ResponseBody },
    Event(WireEvent),
}

impl Shared {
    fn new(soul_dir: SoulDir) -> Self {
        Self {
            runtime: Mutex::new(Runtime::new(soul_dir)),
            subscribers: Mutex::new(Vec::new()),
            clients: Mutex::new(Vec::new()),
            next_subscriber: AtomicU64::new(1),
        }
    }

    fn spawn_client(self: &Arc<Self>, stream: UnixStream) {
        let shutdown = match stream.try_clone() {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("softwaked: clone client socket: {error}");
                return;
            }
        };
        let shared = Arc::clone(self);
        match thread::Builder::new()
            .name("softwake-ipc-client".to_owned())
            .spawn(move || client_loop(stream, &shared))
        {
            Ok(thread) => lock(&self.clients).push(ClientSlot { shutdown, thread }),
            Err(error) => eprintln!("softwaked: spawn client: {error}"),
        }
    }

    fn stop_clients(&self) {
        let mut clients = lock(&self.clients);
        for client in clients.iter() {
            let _ = client.shutdown.shutdown(std::net::Shutdown::Both);
        }
        for client in clients.drain(..) {
            let _ = client.thread.join();
        }
    }

    fn subscribe(&self, tx: SyncSender<Outbound>) -> u64 {
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        lock(&self.subscribers).push(Subscriber { id, tx });
        id
    }

    fn unsubscribe(&self, id: u64) {
        lock(&self.subscribers).retain(|subscriber| subscriber.id != id);
    }

    fn broadcast(&self, event: &WireEvent) {
        let mut subscribers = lock(&self.subscribers);
        subscribers.retain(|subscriber| {
            match subscriber.tx.try_send(Outbound::Event(event.clone())) {
                Ok(()) | Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }
}

fn client_loop(stream: UnixStream, shared: &Shared) {
    // Subscribe before `hello_ok` so a state change cannot be broadcast in
    // the gap after the client has observed the handshake.
    let pending = match ServerConnection::begin(stream) {
        Ok(pending) => pending,
        Err(error) if handshake_disconnected(&error) => return,
        Err(error) => {
            eprintln!("softwaked: client handshake: {error}");
            return;
        }
    };
    let (tx, rx) = mpsc::sync_channel(OUTBOUND_CAPACITY);
    let subscriber = shared.subscribe(tx.clone());
    let connection = match pending.accept() {
        Ok(connection) => connection,
        Err(error) => {
            shared.unsubscribe(subscriber);
            if !handshake_disconnected(&error) {
                eprintln!("softwaked: client handshake: {error}");
            }
            return;
        }
    };
    let (mut reader, writer) = connection.split();
    let writer_thread = match thread::Builder::new()
        .name("softwake-ipc-writer".to_owned())
        .spawn(move || write_loop(writer, &rx))
    {
        Ok(thread) => thread,
        Err(error) => {
            eprintln!("softwaked: spawn writer: {error}");
            shared.unsubscribe(subscriber);
            return;
        }
    };
    while handle_next(shared, &tx, &mut reader) {}
    shared.unsubscribe(subscriber);
    drop(tx);
    let _ = writer_thread.join();
}

fn handle_next(shared: &Shared, tx: &SyncSender<Outbound>, reader: &mut ServerReader) -> bool {
    match reader.read() {
        Ok(ClientMessage::Request { id, command }) => {
            let outcome = lock(&shared.runtime).handle(command);
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::ToolRequest { id, name, args }) => {
            let outcome = lock(&shared.runtime).invoke_tool(&name, &args);
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::ConfirmTool {
            id,
            pending_id,
            name,
        }) => {
            let outcome = lock(&shared.runtime).confirm_tool(&pending_id, name.as_deref());
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::CancelTool {
            id,
            pending_id,
            name,
        }) => {
            let outcome = lock(&shared.runtime).cancel_tool(&pending_id, name.as_deref());
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::Ask { id, text }) => {
            let outcome = lock(&shared.runtime).ask(&text);
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::Wake { id }) => {
            let outcome = lock(&shared.runtime).wake_phrase();
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::Hello { .. }) => false,
        Err(error) if error.is_disconnect() => false,
        Err(error) => {
            eprintln!("softwaked: client: {error}");
            false
        }
    }
}

fn reply(shared: &Shared, tx: &SyncSender<Outbound>, id: u64, outcome: Outcome) -> bool {
    for event in &outcome.events {
        shared.broadcast(event);
    }
    tx.send(Outbound::Response {
        id,
        body: outcome.body,
    })
    .is_ok()
}

fn write_loop(mut writer: ServerWriter, inbound: &Receiver<Outbound>) {
    while let Ok(message) = inbound.recv() {
        let encoded = match message {
            Outbound::Response { id, body } => ServerMessage::Response { id, body },
            Outbound::Event(event) => ServerMessage::Event { body: event },
        };
        if let Err(error) = writer.write(&encoded) {
            if !error.is_disconnect() {
                eprintln!("softwaked: write: {error}");
            }
            break;
        }
    }
}

fn handshake_disconnected(error: &HandshakeError) -> bool {
    match error {
        HandshakeError::Transport(inner) => inner.is_disconnect(),
        HandshakeError::Version { .. } => false,
    }
}

/// A panicked handler must not freeze the other clients. Applying a command
/// does not panic; poison means a bug already happened on this lock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
