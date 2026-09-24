//! Softwake daemon.
//!
//! With no arguments the process prints the initial voice state and exits.
//! `demo` reads typed commands from stdin. `serve` owns the voice-state
//! machine and listens on a Unix socket. `ctl` sends one command to that
//! socket. The demo uses mock capture and does not open a microphone.
//! Native `PipeWire` stays behind the `pipewire-native` feature. While awake, `echo`
//! runs immediately and `notify` waits for confirmation. `shell` is denied.
//! Sleep and hibernate refuse every tool.

mod cli;
mod ctl;
mod demo;
mod dispatch;
mod pcm;
mod runtime;
mod serve;
mod soul;

#[cfg(test)]
mod e2e;

pub use cli::execute;
