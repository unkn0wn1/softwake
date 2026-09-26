//! Softwake daemon.
//!
//! With no arguments the process prints the initial voice state and exits.
//! `demo` reads typed commands from stdin. While awake, `ask` and `chat` send
//! the stored instructions and the typed line to the selected provider. That
//! call stays in-process. Protocol generation stays 1. The `live-http` feature
//! performs the real HTTPS call and is off by default
//! ([ADR 0013](../../docs/ADR-0013-session-provider.md)). `serve` owns the voice-state
//! machine and listens on a Unix socket. `ctl` sends one command to that
//! socket. `ctl ask` and `ctl chat` forward one awake turn to that daemon.
//! The demo uses mock capture and does not open a microphone.
//! Native `PipeWire` capture stays behind the daemon `pipewire-capture` feature (audio `pipewire-native`). While awake, `echo`
//! runs immediately. `notify` and `email_send` wait for confirmation. `shell` is confirm-gated and off until Tools Settings enable it.
//! Sleep and hibernate refuse every tool. Confirming `email_send` appends one
//! in-memory message and does not open a socket.

mod announce;
mod capture;
mod chat;
mod cli;
mod ctl;
mod demo;
mod dispatch;
mod email_tool;
mod free_speech;
mod mode_confirm;
mod mode_intent;
mod pcm;
mod playback_timeout;
mod runtime;
mod serve;
mod shell_intent;
mod skill_intent;
mod soul;
mod talk;
mod verbose_log;

#[cfg(test)]
mod e2e;

pub use cli::execute;
