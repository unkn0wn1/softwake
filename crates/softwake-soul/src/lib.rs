//! Load and validate the phase-1 soul pack.
//!
//! [`SoulDir`] names the directory that holds `soul.md` and `user.md`.
//! [`load`] and [`try_load`] read those files into a [`SoulPack`].
//! [`SoulPack::render_instructions`] builds the system prompt for an awake
//! session: identity, user profile, and a runtime-policy stub.
//!
//! This crate does not open a socket, a microphone, or a model client.

mod error;
mod load;
mod paths;

pub use error::{SoulError, SoulFile};
pub use load::{MAX_FILE_BYTES, SoulPack, SoulStatus, load, try_load};
pub use paths::{SoulDir, SoulPaths, resolve_soul_dir, resolve_soul_dir_from};
