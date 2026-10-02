//! Settings → Rooms admin + operator room chat (ADR-0052).

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use std::env;
use std::path::PathBuf;

use serde::Serialize;
use softwake_ipc::{Client, resolve_socket_path};
use softwake_soul::resolve_config_dir;
use softwake_tools::{
    MAX_ROOM_CHAT_LINES, RoomFile, RoomLogLine, delete_room, ensure_rooms_dir, list_rooms,
    resolve_rooms_dir, resolve_rooms_state_dir, tail_room_log, upsert_room,
};

fn rooms_dir() -> Result<PathBuf, String> {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let dir = resolve_rooms_dir(xdg.as_deref(), home.as_deref())?;
    ensure_rooms_dir(&dir)?;
    Ok(dir)
}

fn state_dir() -> Result<PathBuf, String> {
    let xdg = env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    resolve_rooms_state_dir(xdg.as_deref(), home.as_deref())
}

/// Rooms pane snapshot (admin + chat).
#[derive(Debug, Clone, Serialize)]
pub struct RoomsSnapshot {
    pub rooms: Vec<RoomFile>,
    pub selected_id: Option<String>,
    /// Full-ish chat history for the selected room (not just a tiny tail).
    pub log: Vec<RoomLogLine>,
    pub config_hint: String,
    /// True when the UI is composing a brand-new room (New action).
    #[serde(default)]
    pub composing_new: bool,
}

fn snapshot(selected: Option<String>, composing_new: bool) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    let rooms = list_rooms(&dir)?;
    let selected_id = if composing_new {
        None
    } else {
        selected
            .filter(|id| rooms.iter().any(|r| r.id == *id))
            .or_else(|| rooms.first().map(|r| r.id.clone()))
    };
    let log = if let Some(id) = selected_id.as_deref() {
        tail_room_log(&state_dir()?, id, MAX_ROOM_CHAT_LINES).unwrap_or_default()
    } else {
        Vec::new()
    };
    let config = resolve_config_dir(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).as_deref(),
        env::var_os("HOME").map(PathBuf::from).as_deref(),
    )
    .map_or_else(|_| "(unresolved)".into(), |p| p.display().to_string());
    Ok(RoomsSnapshot {
        rooms,
        selected_id,
        log,
        config_hint: format!("{config}/rooms/"),
        composing_new,
    })
}

#[tauri::command]
pub fn rooms_snapshot(selected_id: Option<String>) -> Result<RoomsSnapshot, String> {
    snapshot(selected_id, false)
}

/// Create or update a room (Settings Save). Persists before the returned snapshot read.
#[tauri::command]
pub fn room_save(id: String, title: String, members: Vec<String>) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    let room = upsert_room(&dir, &id, &title, members)?;
    snapshot(Some(room.id), false)
}

/// Backward-compatible create (same as save for new ids).
#[tauri::command]
pub fn room_create(
    id: String,
    title: String,
    members: Vec<String>,
) -> Result<RoomsSnapshot, String> {
    room_save(id, title, members)
}

/// Backward-compatible update (upsert so missing id creates).
#[tauri::command]
pub fn room_update(
    id: String,
    title: Option<String>,
    members: Option<Vec<String>>,
) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    let title = title.unwrap_or_default();
    let members = members.unwrap_or_default();
    let room = upsert_room(&dir, &id, &title, members)?;
    snapshot(Some(room.id), false)
}

#[tauri::command]
pub fn room_delete(id: String) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    delete_room(&dir, &id)?;
    snapshot(None, false)
}

/// Operator posts into the room; daemon fans out optional member replies.
#[tauri::command]
pub fn room_post(room_id: String, text: String) -> Result<RoomsSnapshot, String> {
    let room_id = room_id.trim().to_owned();
    let text = text.trim().to_owned();
    if room_id.is_empty() {
        return Err("select a room before posting".into());
    }
    if text.is_empty() {
        return Err("message is empty".into());
    }
    // Ensure room exists on disk before asking the daemon to read it.
    let dir = rooms_dir()?;
    let path = softwake_tools::room_file_path(&dir, &room_id);
    softwake_tools::load_room(&path)?;

    let socket = resolve_socket_path(None).map_err(|e| e.to_string())?;
    let mut client = Client::connect(&socket).map_err(|e| e.to_string())?;
    client
        .call_room_post(&room_id, &text)
        .map_err(|e| e.to_string())?;
    snapshot(Some(room_id), false)
}

