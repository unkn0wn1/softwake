//! Daemon and UI protocol vocabulary.
//!
//! Payloads and the socket transport come later. Variants name the commands
//! and events the two processes will share. `set_config` is intentionally
//! absent until the daemon can validate a configuration document.

/// Protocol generation negotiated when a UI connects.
pub const PROTOCOL_VERSION: u32 = 1;

/// Request from the UI to the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Ask for the current voice state.
    GetStatus,
    /// Enter hibernate from sleep or awake.
    Hibernate,
    /// Leave hibernate.
    ///
    /// The daemon lands in sleep, not awake.
    WakeFromUi,
    /// Ask the daemon to sleep from awake.
    Sleep,
    /// Re-read the soul pack on the next awake session.
    ReloadSoul,
}

/// Notification from the daemon to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The voice state changed.
    StateChanged,
    /// Partial transcript. Awake only; do not emit this while asleep or hibernating.
    PartialTranscript,
    /// An allowlisted tool started.
    ToolStarted,
    /// An allowlisted tool finished.
    ToolFinished,
    /// A failure the UI should surface.
    Error,
}
