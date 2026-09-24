//! Softwake daemon.
//!
//! With no arguments the process prints the initial voice state and exits.
//! `demo` reads typed commands from stdin. `serve` owns the voice-state
//! machine and listens on a Unix socket. `ctl` sends one command to that
//! socket. The microphone and `PipeWire` are not wired. While awake, the
//! allowlisted `echo` tool can run; sleep and hibernate refuse it.

mod cli;
mod ctl;
mod demo;
mod dispatch;
mod runtime;
mod serve;
mod soul;

#[cfg(test)]
mod e2e;

pub use cli::execute;
