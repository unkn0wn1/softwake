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
mod home;
mod load;
mod paths;
mod profile;

pub use error::{SoulError, SoulFile};
pub use glossary::{
    ConfirmEcho, EchoReply, EchoReplyError, Glossary, GlossaryError, classify_echo_reply,
};
pub use home::{
    HOMES_DIR_NAME, ensure_profile_home, ensure_profile_home_in, profile_home_dir,
    resolve_data_dir, resolve_data_dir_from_env,
};
pub use load::{MAX_FILE_BYTES, SoulPack, SoulStatus, load, try_load, try_load_effective};
pub use paths::{SoulDir, SoulPaths, resolve_soul_dir, resolve_soul_dir_from};
pub use profile::{
    APP_CONFIG_FILE_NAME, AppConfig, DEFAULT_AGENT_NAME, DEFAULT_PROFILE_ID, DEFAULT_WEBHOOK_PORT,
    FREE_SPEECH_END_SILENCE_MS_DEFAULT, FREE_SPEECH_END_SILENCE_MS_MAX,
    FREE_SPEECH_END_SILENCE_MS_MIN, KWS_THRESHOLD_MILLI_MAX, KWS_THRESHOLD_MILLI_MIN,
    LEGACY_SOUL_DIR_NAME, PROFILE_META_FILE_NAME, PROFILE_ROLE_CODING, PROFILE_ROLE_GENERAL,
    PROFILES_DIR_NAME, ProfileMeta, TTS_PLAYBACK_TIMEOUT_MS_DEFAULT, TTS_PLAYBACK_TIMEOUT_MS_MAX,
    TTS_PLAYBACK_TIMEOUT_MS_MIN, clamp_free_speech_end_silence_ms, clamp_kws_threshold_milli,
    clamp_tts_playback_timeout_ms, create_profile, ensure_migrated, legacy_soul_dir, list_profiles,
    load_app_config, load_profile_meta, profile_name_in, profile_owner_from_pack_dir,
    profile_pack_dir, rename_profile, resolve_active_pack_dir, resolve_config_dir,
    resolve_main_profile_id, set_active_profile, set_allow_all, set_free_speech_end_silence_ms,
    set_global_doc_flags, set_kws_thresholds, set_profile_role, set_tts_playback_timeout_ms,
    set_voice_agent_s2s, set_webhook_enabled, set_webhook_port, write_app_config,
    write_profile_meta,
};
