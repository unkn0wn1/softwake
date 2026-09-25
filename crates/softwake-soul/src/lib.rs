//! Load and validate the four-file context pack.
//!
//! [`SoulDir`] names the directory that holds `soul.md`, `user.md`,
//! `rules.md`, and `glossary.md`. When no flag or `SOFTWAKE_SOUL_DIR` is set,
//! that directory is the active multi-profile pack under `profiles/<id>/`. [`load`] and [`try_load`] read those
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
mod profile;

pub use error::{SoulError, SoulFile};
pub use glossary::{
    ConfirmEcho, EchoReply, EchoReplyError, Glossary, GlossaryError, classify_echo_reply,
};
pub use load::{MAX_FILE_BYTES, SoulPack, SoulStatus, load, try_load};
pub use paths::{SoulDir, SoulPaths, resolve_soul_dir, resolve_soul_dir_from};
pub use profile::{
    APP_CONFIG_FILE_NAME, AppConfig, DEFAULT_AGENT_NAME, DEFAULT_PROFILE_ID, LEGACY_SOUL_DIR_NAME,
    PROFILE_META_FILE_NAME, PROFILES_DIR_NAME, ProfileMeta, create_profile, ensure_migrated,
    legacy_soul_dir, list_profiles, load_app_config, load_profile_meta, profile_name_in,
    profile_pack_dir, rename_profile, resolve_active_pack_dir, resolve_config_dir,
    set_active_profile, write_app_config, write_profile_meta,
};
