//! Load and validate the four-file context pack.
//!
//! [`SoulDir`] names the directory that holds `soul.md`, `user.md`,
//! `rules.md`, and `glossary.md`. [`load`] and [`try_load`] read those
//! files into a [`SoulPack`]. [`SoulPack::render_instructions`] builds the
//! instructions for an awake session: identity, user profile, rules,
//! glossary, and a runtime-policy stub. [`Glossary::expand`] replaces a
//! whole-token alias. [`SoulPack::confirm_echo`] builds a readback. Neither
//! one runs a command.
//!
//! This crate does not open a socket, a microphone, or a model client.

mod error;
mod glossary;
mod load;
mod paths;

pub use error::{SoulError, SoulFile};
pub use glossary::{
    ConfirmEcho, EchoReply, EchoReplyError, Glossary, GlossaryError, classify_echo_reply,
};
pub use load::{MAX_FILE_BYTES, SoulPack, SoulStatus, load, try_load};
pub use paths::{SoulDir, SoulPaths, resolve_soul_dir, resolve_soul_dir_from};
