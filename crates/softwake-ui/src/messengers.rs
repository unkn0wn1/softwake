//! Settings commands for Messengers (Telegram + desktop flags).

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use serde::{Deserialize, Serialize};
use softwake_providers::{SecretBag, SecretStore, update_bag};
use softwake_soul::{list_profiles, load_app_config, resolve_config_dir};
use softwake_tools::{
    ChannelFlags, TelegramChannel, load_messengers, resolve_messengers_file, save_messengers,
};
use std::path::PathBuf;

/// One channel row in the Messengers subnav.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessengerChannelRow {
    pub id: String,
    pub label: String,
    pub bound: bool,
}

/// Snapshot for Settings → Messengers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools, reason = "mirrors Settings checkboxes")]
pub struct MessengersSnapshot {
    pub profile_id: String,
    pub profile_name: String,
    pub channels: Vec<MessengerChannelRow>,
    pub selected_channel: String,
    pub has_bot_token: bool,
    pub storage_backend: String,
    pub storage_message: String,
    pub telegram_enabled: bool,
    pub telegram_default: bool,
    pub telegram_receive_all: bool,
    pub telegram_voice: bool,
    pub telegram_chat_id: String,
    pub desktop_default: bool,
    pub desktop_receive_all: bool,
    pub desktop_voice: bool,
    pub profiles: Vec<MessengerProfileRow>,
}

/// Per-profile channel flags (for the detail table).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct MessengerProfileRow {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub telegram_default: bool,
    pub telegram_receive_all: bool,
    pub telegram_voice: bool,
    pub telegram_enabled: bool,
    pub telegram_chat_id: String,
    pub desktop_default: bool,
    pub desktop_receive_all: bool,
    pub desktop_voice: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools, reason = "mirrors Settings checkboxes")]
pub struct MessengersSaveArgs {
    pub profile_id: String,
    pub telegram_enabled: bool,
    pub telegram_default: bool,
    pub telegram_receive_all: bool,
    pub telegram_voice: bool,
    pub telegram_chat_id: String,
    pub desktop_default: bool,
    pub desktop_receive_all: bool,
    pub desktop_voice: bool,
    /// Empty = leave token unchanged.
    pub bot_token: Option<String>,
    pub clear_bot_token: bool,
}

fn config_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())
}

fn open_secrets() -> Result<Box<dyn SecretStore + Send>, String> {
    let path = softwake_providers::resolve_secrets_file().map_err(|e| e.to_string())?;
    softwake_providers::open_store(&path).map_err(|e| e.to_string())
}

fn token_present(bag: &SecretBag) -> bool {
    bag.telegram_bot_token
        .as_ref()
        .is_some_and(|t| !t.trim().is_empty())
}

fn channels() -> Vec<MessengerChannelRow> {
    vec![
        MessengerChannelRow {
            id: "telegram".into(),
            label: "Telegram".into(),
            bound: false,
        },
        MessengerChannelRow {
            id: "desktop".into(),
            label: "Desktop HUD".into(),
            bound: true,
        },
    ]
}

fn snapshot_for(
    profile_id: &str,
    selected_channel: &str,
    bag: &SecretBag,
    backend: &str,
    message: &str,
) -> Result<MessengersSnapshot, String> {
    let config = config_dir()?;
    let _ = softwake_soul::ensure_migrated(&config);
    let app = load_app_config(&config).unwrap_or_default();
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    let meta = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .unwrap_or_else(|| softwake_soul::ProfileMeta::new(profile_id, profile_id));
    let path = resolve_messengers_file(profile_id).map_err(|e| e.to_string())?;
    let file = load_messengers(&path).map_err(|e| e.to_string())?;
    let mut chans = channels();
    if let Some(tg) = chans.iter_mut().find(|c| c.id == "telegram") {
        tg.bound = file.telegram.is_bound();
    }
    let selected = if selected_channel == "desktop" || selected_channel == "telegram" {
        selected_channel.to_owned()
    } else {
        "telegram".into()
    };
    let mut profile_rows = Vec::new();
    for p in &profiles {
        let ppath = resolve_messengers_file(&p.id).map_err(|e| e.to_string())?;
        let pf = load_messengers(&ppath).unwrap_or_default();
        profile_rows.push(MessengerProfileRow {
            id: p.id.clone(),
            name: p.name.clone(),
            active: p.id == app.active_profile,
            telegram_default: pf.telegram.default,
            telegram_receive_all: pf.telegram.receive_all,
            telegram_voice: pf.telegram.voice,
            telegram_enabled: pf.telegram.enabled,
            telegram_chat_id: pf.telegram.chat_id.clone().unwrap_or_default(),
            desktop_default: pf.desktop.default,
            desktop_receive_all: pf.desktop.receive_all,
            desktop_voice: pf.desktop.voice,
        });
    }
    Ok(MessengersSnapshot {
        profile_id: meta.id,
        profile_name: meta.name,
        channels: chans,
        selected_channel: selected,
        has_bot_token: token_present(bag),
        storage_backend: backend.to_owned(),
        storage_message: message.to_owned(),
        telegram_enabled: file.telegram.enabled,
        telegram_default: file.telegram.default,
        telegram_receive_all: file.telegram.receive_all,
        telegram_voice: file.telegram.voice,
        telegram_chat_id: file.telegram.chat_id.clone().unwrap_or_default(),
        desktop_default: file.desktop.default,
        desktop_receive_all: file.desktop.receive_all,
        desktop_voice: file.desktop.voice,
        profiles: profile_rows,
    })
}

/// Load Messengers Settings for a profile + channel.
#[tauri::command]
pub fn messengers_snapshot(
    profile_id: Option<String>,
    selected_channel: Option<String>,
) -> Result<MessengersSnapshot, String> {
    let config = config_dir()?;
    let _ = softwake_soul::ensure_migrated(&config);
    let app = load_app_config(&config).unwrap_or_default();
    let id = profile_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(app.active_profile);
    let channel = selected_channel.unwrap_or_else(|| "telegram".into());
    let store = open_secrets()?;
    let bag = store.load().map_err(|e| e.to_string())?;
    let report = store.report();
    snapshot_for(
        &id,
        &channel,
        &bag,
        report.backend.as_str(),
        &report.message,
    )
}

/// Save channel flags and optional bot token.
#[tauri::command]
pub fn messengers_save(args: MessengersSaveArgs) -> Result<MessengersSnapshot, String> {
    let profile_id = args.profile_id.trim();
    if profile_id.is_empty() {
        return Err("profile id required".into());
    }
    let path = resolve_messengers_file(profile_id).map_err(|e| e.to_string())?;
    let mut file = load_messengers(&path).unwrap_or_default();
    file.version = 1;
    file.desktop = ChannelFlags {
        default: args.desktop_default,
        receive_all: args.desktop_receive_all,
        voice: args.desktop_voice,
    };
    let chat = args.telegram_chat_id.trim();
    file.telegram = TelegramChannel {
        enabled: args.telegram_enabled,
        default: args.telegram_default,
        receive_all: args.telegram_receive_all,
        voice: args.telegram_voice,
        chat_id: if chat.is_empty() {
            None
        } else {
            Some(chat.to_owned())
        },
    };
    save_messengers(&path, &file).map_err(|e| e.to_string())?;

    let store = open_secrets()?;
    if args.clear_bot_token {
        update_bag(store.as_ref(), |bag| {
            bag.telegram_bot_token = None;
        })
        .map_err(|e| e.to_string())?;
    } else if let Some(token) = args.bot_token {
        let trimmed = token.trim().to_owned();
        if !trimmed.is_empty() {
            update_bag(store.as_ref(), |bag| {
                bag.telegram_bot_token = Some(trimmed);
            })
            .map_err(|e| e.to_string())?;
        }
    }
    let bag = store.load().map_err(|e| e.to_string())?;
    let report = store.report();
    snapshot_for(
        profile_id,
        "telegram",
        &bag,
        report.backend.as_str(),
        &report.message,
    )
}

/// Clear the Telegram bot token only.
#[tauri::command]
pub fn messengers_clear_token(profile_id: Option<String>) -> Result<MessengersSnapshot, String> {
    let store = open_secrets()?;
    update_bag(store.as_ref(), |bag| {
        bag.telegram_bot_token = None;
    })
    .map_err(|e| e.to_string())?;
    messengers_snapshot(profile_id, Some("telegram".into()))
}
