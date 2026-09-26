//! Blocking IPC server for [`crate::runtime::Runtime`].
//!
//! Phase 1 stays on std threads. Each client has a reader and a writer so one
//! slow UI cannot stop another client's request from being applied. Events are
//! queued per client; a full queue drops that event for that client and the
//! next `get_status` shows the truth.
//!
//! `GetStatus` uses `try_lock` on the runtime mutex: when STT/ask/TTS holds the
//! lock, the server returns the last cached [`Status`] so the HUD bloom poll
//! never waits on the voice pipeline. Capture-level may be briefly stale;
//! particles keep last-known level (and a local breath in the UI).
//!
//! The listener is removed when this task drops. A crash skips that drop, and
//! the next bind deletes the file if nothing answers on it.

use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use softwake_ipc::{
    ClientMessage, Command, Event as WireEvent, HandshakeError, IpcStream, Listener,
    PROTOCOL_VERSION, ResponseBody, ServerConnection, ServerMessage, ServerReader, ServerWriter,
    SocketError, Status, VoiceState, connect_stream, resolve_socket_path,
};

use softwake_soul::SoulDir;

use crate::capture::CaptureKind;
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

    /// The selected capture backend could not be opened.
    #[error(transparent)]
    Capture(#[from] crate::capture::CaptureError),
}

/// Bind, print the listening line, and block until the accept thread ends.
///
/// # Errors
///
/// Returns [`ServeError`] when the socket is unavailable or the accept thread
/// panics. A peer that still holds the socket produces [`SocketError::InUse`].
pub(crate) fn run(
    socket: Option<&Path>,
    soul_dir: SoulDir,
    capture: CaptureKind,
    verbosity: u8,
) -> Result<(), ServeError> {
    let path = resolve_socket_path(socket)?;
    let handle = spawn(path.clone(), soul_dir, capture, verbosity)?;
    println!("softwaked serve");
    println!("listening: {}", path.display());
    println!("protocol: {PROTOCOL_VERSION}");
    println!("capture: {}", capture.as_str());
    if verbosity > 0 {
        println!("verbose: {verbosity}");
    }
    handle.wait()
}

/// Bind `path` and accept clients until the handle is dropped.
///
/// # Errors
///
/// Returns [`ServeError`] when the socket cannot be bound.
pub(crate) fn spawn(
    path: PathBuf,
    soul_dir: SoulDir,
    capture: CaptureKind,
    verbosity: u8,
) -> Result<ServeHandle, ServeError> {
    // Build the runtime *before* binding. A sticky Secret Service Unlock (or any
    // other init stall) must not leave a listening socket that queues clients
    // forever with no accept thread.
    let shared = Arc::new(Shared::new(soul_dir, capture, verbosity)?);
    #[cfg(test)]
    let shared_for_handle = Arc::clone(&shared);
    let listener = Listener::bind(&path)?;
    if listener.replaced_stale() {
        eprintln!("softwaked: removed stale socket {}", path.display());
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shutdown);
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
        lock(&self.shared.runtime).session_turns()
    }

    /// Hold the runtime mutex so tests can prove `GetStatus` uses the cache.
    #[cfg(test)]
    pub(crate) fn lock_runtime_for_test(&self) -> MutexGuard<'_, Runtime> {
        lock(&self.shared.runtime)
    }

    /// Seed or overwrite the status cache (thinking line during a held lock).
    #[cfg(test)]
    pub(crate) fn publish_thinking_for_test(&self, detail: &str) {
        publish_thinking(&self.shared, detail);
    }
}

impl Drop for ServeHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Unblock `accept`. Failure is fine when the listener is already gone.
        let _ = connect_stream(&self.path);
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
    /// Last successful status snapshot. Served when `GetStatus` cannot take the
    /// runtime lock because `ask` / `talk_stop` is in flight.
    last_status: Mutex<Option<Status>>,
    subscribers: Mutex<Vec<Subscriber>>,
    clients: Mutex<Vec<ClientSlot>>,
    next_subscriber: AtomicU64,
}

struct Subscriber {
    id: u64,
    tx: SyncSender<Outbound>,
}

struct ClientSlot {
    shutdown: IpcStream,
    thread: JoinHandle<()>,
}

enum Outbound {
    Response { id: u64, body: ResponseBody },
    Event(WireEvent),
}

impl Shared {
    fn new(
        soul_dir: SoulDir,
        capture: CaptureKind,
        verbosity: u8,
    ) -> Result<Self, crate::capture::CaptureError> {
        Ok(Self {
            runtime: Mutex::new(Runtime::with_capture_verbosity(
                soul_dir, capture, verbosity,
            )?),
            last_status: Mutex::new(None),
            subscribers: Mutex::new(Vec::new()),
            clients: Mutex::new(Vec::new()),
            next_subscriber: AtomicU64::new(1),
        })
    }

    fn spawn_client(self: &Arc<Self>, stream: IpcStream) {
        // HUD / tray open a fresh socket per status poll. Without reaping, each
        // finished client leaves its shutdown `IpcStream` clone in `clients`
        // forever and the process hits EMFILE (os error 24).
        self.reap_clients();
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

    /// Drop slots whose handler thread has exited so their socket FDs close.
    fn reap_clients(&self) {
        let mut clients = lock(&self.clients);
        let mut alive = Vec::with_capacity(clients.len());
        for slot in clients.drain(..) {
            if slot.thread.is_finished() {
                let _ = slot.thread.join();
                // `shutdown` drops here and releases the leaked FD.
            } else {
                alive.push(slot);
            }
        }
        *clients = alive;
    }

    fn stop_clients(&self) {
        self.reap_clients();
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

fn client_loop(stream: IpcStream, shared: &Shared) {
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
            let (outcome, pending_auto) = if matches!(command, Command::GetStatus) {
                status_outcome(shared)
            } else {
                let mut runtime = lock(&shared.runtime);
                let outcome = runtime.handle(command);
                let pending_auto = runtime.take_pending_auto_pcm();
                (outcome, pending_auto)
            };
            let ok = reply(shared, tx, id, outcome);
            if let Some(pcm) = pending_auto {
                publish_thinking(shared, "free speech");
                let outcome = lock(&shared.runtime).transcribe_and_ask_pub(&pcm);
                for event in &outcome.events {
                    shared.broadcast(event);
                }
                remember_status(shared, &outcome);
            }
            ok
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
            publish_thinking(shared, "ask");
            let outcome = lock(&shared.runtime).ask(&text);
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::Wake { id }) => {
            let outcome = lock(&shared.runtime).wake_phrase();
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::TalkStart { id }) => {
            let outcome = lock(&shared.runtime).talk_start();
            reply(shared, tx, id, outcome)
        }
        Ok(ClientMessage::TalkStop { id }) => {
            publish_thinking(shared, "press to talk");
            let outcome = lock(&shared.runtime).talk_stop();
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
    remember_status(shared, &outcome);
    for event in &outcome.events {
        shared.broadcast(event);
    }
    tx.send(Outbound::Response {
        id,
        body: outcome.body,
    })
    .is_ok()
}

fn remember_status(shared: &Shared, outcome: &Outcome) {
    if let Some(status) = outcome.body.status() {
        *lock(&shared.last_status) = Some(status.clone());
    }
}

/// Push a thinking line into the status cache before a long lock hold so HUD
/// `GetStatus` `try_lock` misses still show thinking mid-pipeline.
fn publish_thinking(shared: &Shared, detail: &str) {
    let mut guard = lock(&shared.last_status);
    if let Some(status) = guard.as_mut() {
        status.message = Some("thinking…".to_owned());
        status.detail = Some(detail.to_owned());
        status.talking = false;
        status.auto_listening = false;
    } else {
        *guard = Some(Status {
            state: VoiceState::Awake,
            capture_running: true,
            capture_level: None,
            soul_reload_pending: false,
            soul: None,
            message: Some("thinking…".to_owned()),
            detail: Some(detail.to_owned()),
            pending_tool: None,
            last_tool: None,
            talking: false,
            auto_listening: false,
            context_used: None,
            context_limit: None,
            context_compacted: false,
        });
    }
}

/// `GetStatus` without waiting on STT/ask/TTS that already hold the runtime lock.
fn status_outcome(shared: &Shared) -> (Outcome, Option<Vec<i16>>) {
    match shared.runtime.try_lock() {
        Ok(mut runtime) => {
            let outcome = runtime.handle(Command::GetStatus);
            let pending_auto = runtime.take_pending_auto_pcm();
            (outcome, pending_auto)
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            let cached = lock(&shared.last_status).clone();
            let status = cached.unwrap_or_else(placeholder_status);
            (
                Outcome {
                    body: ResponseBody::ok(status),
                    events: Vec::new(),
                },
                None,
            )
        }
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            let mut runtime = poisoned.into_inner();
            let outcome = runtime.handle(Command::GetStatus);
            let pending_auto = runtime.take_pending_auto_pcm();
            (outcome, pending_auto)
        }
    }
}

fn placeholder_status() -> Status {
    Status {
        state: VoiceState::Sleep,
        capture_running: false,
        capture_level: None,
        soul_reload_pending: false,
        soul: None,
        message: None,
        detail: Some("status cache warming".to_owned()),
        pending_tool: None,
        last_tool: None,
        talking: false,
        auto_listening: false,
        context_used: None,
        context_limit: None,
        context_compacted: false,
    }
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
