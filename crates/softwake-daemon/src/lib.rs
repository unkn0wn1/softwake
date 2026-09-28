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
//! runs immediately. `notify`, `email_send`, inbox/calendar/Drive read tools, `calendar_create`, `calendar_update`, `calendar_delete`, `skill_save`, and `schedule` wait for confirmation (operator Always allow may skip the prompt). `shell` is confirm-gated and off until Tools Settings enable it.
//! Sleep and hibernate refuse every tool. Confirming `email_send` delivers via
//! OAuth when an account is connected and `live-http` is on; otherwise it
//! appends one in-memory message (or a local draft) and does not open a socket.
//! Confirming a calendar write posts to the primary calendar on that same path.

mod announce;
mod calendar_write;
mod capture;
mod chat;
mod cli;
mod cloud_tools;
mod ctl;
mod ctl_disk;
mod demo;
mod dispatch;
mod email_send;
mod email_tool;
mod free_speech;
mod hud_chat_write;
mod hud_seed;
mod mcp_bridge;
mod mcp_client;
mod mode_confirm;
mod mode_intent;
mod pcm;
mod playback_timeout;
mod remote_agent;
mod reply_latch;
mod runtime;
mod schedule_intent;
mod schedule_tick;
mod serve;
mod shell_intent;
mod skill_intent;
mod slash;
mod soul;
mod talk;
mod telegram;
mod tool_loop;
mod verbose_log;
mod webhook;

#[cfg(test)]
mod e2e;

pub use cli::execute;
