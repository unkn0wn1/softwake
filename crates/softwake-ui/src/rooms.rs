//! Settings → Rooms (ADR-0052).

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use std::env;
use std::path::PathBuf;

use serde::Serialize;
use softwake_soul::resolve_config_dir;
use softwake_tools::{
    RoomFile, RoomLogLine, create_room, delete_room, list_rooms, resolve_rooms_dir,
    resolve_rooms_state_dir, tail_room_log, update_room,
};

fn rooms_dir() -> Result<PathBuf, String> {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    resolve_rooms_dir(xdg.as_deref(), home.as_deref())
}

fn state_dir() -> Result<PathBuf, String> {
    let xdg = env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    resolve_rooms_state_dir(xdg.as_deref(), home.as_deref())
}

/// Rooms pane snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct RoomsSnapshot {
    pub rooms: Vec<RoomFile>,
    pub selected_id: Option<String>,
    pub log: Vec<RoomLogLine>,
    pub config_hint: String,
}

fn snapshot(selected: Option<String>) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    let rooms = list_rooms(&dir)?;
    let selected_id = selected
        .filter(|id| rooms.iter().any(|r| r.id == *id))
        .or_else(|| rooms.first().map(|r| r.id.clone()));
    let log = if let Some(id) = selected_id.as_deref() {
        tail_room_log(&state_dir()?, id, 80).unwrap_or_default()
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
    })
}

#[tauri::command]
pub fn rooms_snapshot(selected_id: Option<String>) -> Result<RoomsSnapshot, String> {
    snapshot(selected_id)
}

#[tauri::command]
pub fn room_create(
    id: String,
    title: String,
    members: Vec<String>,
) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    create_room(&dir, &id, &title, members)?;
    snapshot(Some(id))
}

#[tauri::command]
pub fn room_update(
    id: String,
    title: Option<String>,
    members: Option<Vec<String>>,
) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    update_room(&dir, &id, title.as_deref(), members)?;
    snapshot(Some(id))
}

#[tauri::command]
pub fn room_delete(id: String) -> Result<RoomsSnapshot, String> {
    let dir = rooms_dir()?;
    delete_room(&dir, &id)?;
    snapshot(None)
}
