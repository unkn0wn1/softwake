//! Multi-agent rooms + append-only progress logs (ADR-0052).
//!
//! Config: `$XDG_CONFIG_HOME/softwake/rooms/<id>.json`
//! Log:    `$XDG_STATE_HOME/softwake/rooms/<id>/log.jsonl`

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use softwake_soul::resolve_config_dir;

/// Default cool-down between automatic room turns (ms).
pub const ROOM_COOLDOWN_MS: u64 = 1500;

/// Max members per room.
pub const MAX_ROOM_MEMBERS: usize = 16;

/// Max rooms kept on disk.
pub const MAX_ROOMS: usize = 64;

/// Max JSONL lines retained when trimming (soft cap on read helpers).
pub const MAX_LOG_TAIL: usize = 200;

/// On-disk room document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomFile {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Stable id (folder-safe slug).
    pub id: String,
    /// Display title.
    #[serde(default)]
    pub title: String,
    /// Profile ids that may speak.
    #[serde(default)]
    pub members: Vec<String>,
    /// Created wall time (ms).
    #[serde(default)]
    pub created_ms: u64,
    /// Last update wall time (ms).
    #[serde(default)]
    pub updated_ms: u64,
    /// Last turn finish time (ms) for cool-down.
    #[serde(default)]
    pub last_turn_ms: u64,
    /// Profile id that last spoke (turn-taking hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_speaker: Option<String>,
}

fn one() -> u32 {
    1
}

/// Kind of room log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomLogKind {
    /// Member chat turn.
    Say,
    /// Peer DM that touched this room.
    Dm,
    /// Goal-loop progress.
    GoalProgress,
    /// Softwake system note.
    System,
}

impl RoomLogKind {
    /// Stable spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Say => "say",
            Self::Dm => "dm",
            Self::GoalProgress => "goal_progress",
            Self::System => "system",
        }
    }
}

/// One JSONL progress / transcript line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLogLine {
    /// Event time (ms).
    pub ts_ms: u64,
    /// Speaking / owning profile id.
    pub profile_id: String,
    /// Display name when known.
    #[serde(default)]
    pub name: String,
    /// Line kind.
    pub kind: RoomLogKind,
    /// Body text.
    pub text: String,
    /// Optional goal iteration index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    /// Optional goal phase (`plan` / `execute` / `verify` / …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// Softwake rooms config directory.
///
/// # Errors
///
/// Config root unresolved.
pub fn resolve_rooms_dir(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let config = resolve_config_dir(xdg_config_home, home).map_err(|e| e.to_string())?;
    Ok(config.join("rooms"))
}

/// Softwake rooms state (logs) directory.
///
/// # Errors
///
/// State root unresolved.
pub fn resolve_rooms_state_dir(
    xdg_state_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(xdg) = xdg_state_home.filter(|p| !p.as_os_str().is_empty()) {
        return Ok(xdg.join("softwake").join("rooms"));
    }
    if let Some(h) = home.filter(|p| !p.as_os_str().is_empty()) {
        return Ok(h
            .join(".local")
            .join("state")
            .join("softwake")
            .join("rooms"));
    }
    Err("rooms state directory unresolved".into())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn valid_room_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && !id.starts_with('.')
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Path to one room JSON file.
#[must_use]
pub fn room_file_path(rooms_dir: &Path, room_id: &str) -> PathBuf {
    rooms_dir.join(format!("{room_id}.json"))
}

/// Load a room document.
///
/// # Errors
///
/// Missing / invalid JSON.
pub fn load_room(path: &Path) -> Result<RoomFile, String> {
    let bytes = fs::read(path).map_err(|e| format!("read room: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse room: {e}"))
}

/// Save a room document (creates parent dirs).
///
/// # Errors
///
/// I/O or serialization.
pub fn save_room(rooms_dir: &Path, room: &RoomFile) -> Result<PathBuf, String> {
    if !valid_room_id(&room.id) {
        return Err("invalid room id".into());
    }
    if room.members.len() > MAX_ROOM_MEMBERS {
        return Err(format!("room may have at most {MAX_ROOM_MEMBERS} members"));
    }
    fs::create_dir_all(rooms_dir).map_err(|e| format!("rooms dir: {e}"))?;
    let path = room_file_path(rooms_dir, &room.id);
    let body = serde_json::to_vec_pretty(room).map_err(|e| format!("serialize room: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("write room: {e}"))?;
    Ok(path)
}

/// List rooms in `rooms_dir` (sorted by id).
///
/// # Errors
///
/// I/O listing.
pub fn list_rooms(rooms_dir: &Path) -> Result<Vec<RoomFile>, String> {
    if !rooms_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(rooms_dir).map_err(|e| format!("list rooms: {e}"))? {
        let entry = entry.map_err(|e| format!("list rooms: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(room) = load_room(&path) {
            out.push(room);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Create a room (fails if id exists).
///
/// # Errors
///
/// Invalid id, too many rooms, or I/O.
pub fn create_room(
    rooms_dir: &Path,
    id: &str,
    title: &str,
    members: Vec<String>,
) -> Result<RoomFile, String> {
    if !valid_room_id(id) {
        return Err("invalid room id".into());
    }
    let existing = list_rooms(rooms_dir)?;
    if existing.len() >= MAX_ROOMS {
        return Err(format!("at most {MAX_ROOMS} rooms"));
    }
    if existing.iter().any(|r| r.id == id) {
        return Err(format!("room `{id}` already exists"));
    }
    let mut members = members;
    members.sort();
    members.dedup();
    if members.len() > MAX_ROOM_MEMBERS {
        return Err(format!("room may have at most {MAX_ROOM_MEMBERS} members"));
    }
    let now = now_ms();
    let room = RoomFile {
        version: 1,
        id: id.trim().to_owned(),
        title: title.trim().to_owned(),
        members,
        created_ms: now,
        updated_ms: now,
        last_turn_ms: 0,
        last_speaker: None,
    };
    save_room(rooms_dir, &room)?;
    Ok(room)
}

/// Update members / title.
///
/// # Errors
///
/// Missing room or validation.
pub fn update_room(
    rooms_dir: &Path,
    id: &str,
    title: Option<&str>,
    members: Option<Vec<String>>,
) -> Result<RoomFile, String> {
    let path = room_file_path(rooms_dir, id);
    let mut room = load_room(&path)?;
    if let Some(title) = title {
        title.trim().clone_into(&mut room.title);
    }
    if let Some(mut members) = members {
        members.sort();
        members.dedup();
        if members.len() > MAX_ROOM_MEMBERS {
            return Err(format!("room may have at most {MAX_ROOM_MEMBERS} members"));
        }
        room.members = members;
    }
    room.updated_ms = now_ms();
    save_room(rooms_dir, &room)?;
    Ok(room)
}

/// Delete a room JSON (logs retained unless caller removes state dir).
///
/// # Errors
///
/// I/O.
pub fn delete_room(rooms_dir: &Path, id: &str) -> Result<(), String> {
    let path = room_file_path(rooms_dir, id);
    if path.is_file() {
        fs::remove_file(&path).map_err(|e| format!("delete room: {e}"))?;
    }
    Ok(())
}

/// Whether cool-down has elapsed since `last_turn_ms`.
#[must_use]
pub fn room_cooldown_elapsed(last_turn_ms: u64, now: u64, cooldown_ms: u64) -> bool {
    if last_turn_ms == 0 {
        return true;
    }
    now.saturating_sub(last_turn_ms) >= cooldown_ms
}

/// Mark a turn finished (updates cool-down fields).
///
/// # Errors
///
/// Missing room / I/O.
pub fn mark_room_turn(
    rooms_dir: &Path,
    room_id: &str,
    speaker_profile_id: &str,
) -> Result<RoomFile, String> {
    let path = room_file_path(rooms_dir, room_id);
    let mut room = load_room(&path)?;
    let now = now_ms();
    room.last_turn_ms = now;
    room.last_speaker = Some(speaker_profile_id.to_owned());
    room.updated_ms = now;
    save_room(rooms_dir, &room)?;
    Ok(room)
}

fn log_path(state_dir: &Path, room_id: &str) -> PathBuf {
    state_dir.join(room_id).join("log.jsonl")
}

/// Append one log line.
///
/// # Errors
///
/// I/O.
pub fn append_room_log(state_dir: &Path, room_id: &str, line: &RoomLogLine) -> Result<(), String> {
    if !valid_room_id(room_id) {
        return Err("invalid room id".into());
    }
    let dir = state_dir.join(room_id);
    fs::create_dir_all(&dir).map_err(|e| format!("room log dir: {e}"))?;
    let path = log_path(state_dir, room_id);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open room log: {e}"))?;
    let mut row = serde_json::to_string(line).map_err(|e| format!("serialize log: {e}"))?;
    row.push('\n');
    file.write_all(row.as_bytes())
        .map_err(|e| format!("write room log: {e}"))?;
    Ok(())
}

/// Read the last `limit` log lines (default [`MAX_LOG_TAIL`]).
///
/// # Errors
///
/// I/O.
pub fn tail_room_log(
    state_dir: &Path,
    room_id: &str,
    limit: usize,
) -> Result<Vec<RoomLogLine>, String> {
    let path = log_path(state_dir, room_id);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("read room log: {e}"))?;
    let mut lines: Vec<RoomLogLine> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let keep = limit.clamp(1, MAX_LOG_TAIL);
    if lines.len() > keep {
        lines = lines.split_off(lines.len() - keep);
    }
    Ok(lines)
}

/// Helper: append a `goal_progress` line.
///
/// # Errors
///
/// I/O.
pub fn log_goal_progress(
    state_dir: &Path,
    room_id: &str,
    profile_id: &str,
    name: &str,
    iteration: u32,
    phase: &str,
    text: &str,
) -> Result<(), String> {
    append_room_log(
        state_dir,
        room_id,
        &RoomLogLine {
            ts_ms: now_ms(),
            profile_id: profile_id.to_owned(),
            name: name.to_owned(),
            kind: RoomLogKind::GoalProgress,
            text: text.to_owned(),
            iteration: Some(iteration),
            phase: Some(phase.to_owned()),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("softwake-rooms-{tag}-{n}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_list_log_cooldown() {
        let root = temp("basic");
        let rooms = root.join("rooms");
        let state = root.join("state");
        let created = create_room(
            &rooms,
            "standup",
            "Standup",
            vec!["default".into(), "sally".into()],
        )
        .unwrap();
        assert_eq!(created.members.len(), 2);
        let listed = list_rooms(&rooms).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(room_cooldown_elapsed(0, 1000, ROOM_COOLDOWN_MS));
        assert!(!room_cooldown_elapsed(1000, 1001, ROOM_COOLDOWN_MS));
        assert!(room_cooldown_elapsed(
            1000,
            1000 + ROOM_COOLDOWN_MS,
            ROOM_COOLDOWN_MS
        ));
        mark_room_turn(&rooms, "standup", "sally").unwrap();
        append_room_log(
            &state,
            "standup",
            &RoomLogLine {
                ts_ms: now_ms(),
                profile_id: "sally".into(),
                name: "Sally".into(),
                kind: RoomLogKind::Say,
                text: "hello".into(),
                iteration: None,
                phase: None,
            },
        )
        .unwrap();
        log_goal_progress(&state, "standup", "default", "Softwake", 1, "plan", "draft").unwrap();
        let tail = tail_room_log(&state, "standup", 10).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[1].kind, RoomLogKind::GoalProgress);
        let _ = fs::remove_dir_all(&root);
    }
}
