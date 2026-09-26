//! Settings commands for per-profile schedules.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use serde::{Deserialize, Serialize};
use softwake_soul::{list_profiles, load_app_config, resolve_config_dir};
use softwake_tools::{
    ScheduleAction, ScheduleEntry, ScheduleKind, apply_action, load_schedules, now_ms,
    refresh_next_fire, resolve_schedules_file, save_schedules, validate_entry,
};
use std::path::PathBuf;

/// One row as the Timers page shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerRow {
    pub id: String,
    pub profile_id: String,
    pub profile_name: String,
    pub kind: String,
    pub title: String,
    pub message: String,
    pub enabled: bool,
    pub when: String,
    pub next_fire_ms: Option<u64>,
}

/// Timers Settings snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimersSnapshot {
    pub active_profile_id: String,
    pub show_all: bool,
    pub rows: Vec<TimerRow>,
}

fn config_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())
}

fn when_of(entry: &ScheduleEntry) -> String {
    match entry.kind {
        ScheduleKind::Once => entry.at_local.clone().unwrap_or_default(),
        ScheduleKind::Daily => entry.daily_time.clone().unwrap_or_default(),
        ScheduleKind::Cron => entry.cron.clone().unwrap_or_default(),
    }
}

fn rows_for_profile(profile_id: &str, profile_name: &str) -> Result<Vec<TimerRow>, String> {
    let path = resolve_schedules_file(profile_id).map_err(|e| e.to_string())?;
    let file = load_schedules(&path).map_err(|e| e.to_string())?;
    Ok(file
        .entries
        .into_iter()
        .map(|e| {
            let when = when_of(&e);
            TimerRow {
                id: e.id,
                profile_id: profile_id.to_owned(),
                profile_name: profile_name.to_owned(),
                kind: e.kind.as_str().to_owned(),
                title: e.title,
                message: e.message,
                enabled: e.enabled,
                when,
                next_fire_ms: e.next_fire_ms,
            }
        })
        .collect())
}

/// Load Timers Settings.
#[tauri::command]
pub fn timers_snapshot(show_all: bool) -> Result<TimersSnapshot, String> {
    let config = config_dir()?;
    let _ = softwake_soul::ensure_migrated(&config);
    let app = load_app_config(&config).unwrap_or_default();
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    if show_all {
        for meta in &profiles {
            rows.extend(rows_for_profile(&meta.id, &meta.name)?);
        }
    } else {
        let name = profiles
            .iter()
            .find(|p| p.id == app.active_profile)
            .map_or_else(|| app.active_profile.clone(), |p| p.name.clone());
        rows = rows_for_profile(&app.active_profile, &name)?;
    }
    Ok(TimersSnapshot {
        active_profile_id: app.active_profile,
        show_all,
        rows,
    })
}

/// Upsert a timer on a profile (default: active).
#[tauri::command]
pub fn timers_upsert(
    profile_id: Option<String>,
    id: Option<String>,
    kind: String,
    when: String,
    title: String,
    message: String,
    enabled: bool,
) -> Result<TimersSnapshot, String> {
    let config = config_dir()?;
    let app = load_app_config(&config).unwrap_or_default();
    let profile = profile_id.unwrap_or_else(|| app.active_profile.clone());
    let path = resolve_schedules_file(&profile).map_err(|e| e.to_string())?;
    let mut file = load_schedules(&path).map_err(|e| e.to_string())?;
    let now = now_ms();
    let kind = match kind.to_ascii_lowercase().as_str() {
        "once" => ScheduleKind::Once,
        "daily" => ScheduleKind::Daily,
        "cron" => ScheduleKind::Cron,
        other => return Err(format!("unknown kind: {other}")),
    };
    let text = if message.trim().is_empty() {
        title.clone()
    } else {
        message
    };
    if let Some(existing_id) = id.filter(|s| !s.is_empty()) {
        let action = ScheduleAction::Edit {
            id: existing_id,
            kind: Some(kind),
            when: Some(when),
            text: Some(text),
            enabled: Some(enabled),
        };
        apply_action(&mut file, &action, now).map_err(|e| e.to_string())?;
    } else {
        let action = ScheduleAction::Create { kind, when, text };
        apply_action(&mut file, &action, now).map_err(|e| e.to_string())?;
        // apply_action create uses text for title+message; enabled always true — fix if needed
        if let Some(last) = file.entries.last_mut() {
            if !title.trim().is_empty() {
                last.title = title;
            }
            last.enabled = enabled;
            validate_entry(last).map_err(|e| e.to_string())?;
            refresh_next_fire(last, now).map_err(|e| e.to_string())?;
        }
    }
    save_schedules(&path, &file).map_err(|e| e.to_string())?;
    timers_snapshot(false)
}

/// Delete a timer by id on a profile.
#[tauri::command]
pub fn timers_delete(profile_id: String, id: String) -> Result<TimersSnapshot, String> {
    let path = resolve_schedules_file(&profile_id).map_err(|e| e.to_string())?;
    let mut file = load_schedules(&path).map_err(|e| e.to_string())?;
    apply_action(&mut file, &ScheduleAction::Delete { id }, now_ms()).map_err(|e| e.to_string())?;
    save_schedules(&path, &file).map_err(|e| e.to_string())?;
    timers_snapshot(false)
}
