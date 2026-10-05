//! Replace or delete one member `say` row in a room `log.jsonl`.
//!
//! The room log stays append-only for new lines. A queued-reply re-check
//! rewrites the draft row in place so the room shows one final line
//! ([ADR 0053](../../docs/ADR-0053-room-queued-reply-recheck.md)).

use std::fs;
use std::path::Path;

use softwake_tools::{RoomLogKind, RoomLogLine};

/// Rewrite the first `say` whose profile and text match `draft`.
///
/// `replacement` `None` deletes that row. `Some` keeps `ts_ms` and the other
/// fields and changes `text`. Later rows are copied through. Returns false
/// when no row matches, and does not append a new one.
///
/// # Errors
///
/// Room id rejected, or the log file cannot be read or replaced.
pub(crate) fn rewrite_room_say(
    state_dir: &Path,
    room_id: &str,
    profile_id: &str,
    draft: &str,
    replacement: Option<&str>,
) -> Result<bool, String> {
    if room_id.trim().is_empty() || room_id.contains('/') || room_id.contains("..") {
        return Err("invalid room id".to_owned());
    }
    let path = state_dir.join(room_id).join("log.jsonl");
    if !path.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(&path).map_err(|error| format!("read room log: {error}"))?;
    let mut out = String::new();
    let mut changed = false;
    for line in text.lines() {
        if !changed {
            if let Ok(mut row) = serde_json::from_str::<RoomLogLine>(line) {
                if row.kind == RoomLogKind::Say && row.profile_id == profile_id && row.text == draft
                {
                    changed = true;
                    if let Some(replacement) = replacement {
                        replacement.clone_into(&mut row.text);
                        let serialized = serde_json::to_string(&row)
                            .map_err(|error| format!("serialize log: {error}"))?;
                        out.push_str(&serialized);
                        out.push('\n');
                    }
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !changed {
        return Ok(false);
    }
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, out).map_err(|error| format!("write room log: {error}"))?;
    fs::rename(&tmp, &path).map_err(|error| format!("replace room log: {error}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use softwake_tools::{RoomLogKind, RoomLogLine, append_room_log};

    use super::rewrite_room_say;

    fn temp(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "softwake-room-log-edit-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp");
        dir
    }

    fn say(profile_id: &str, name: &str, text: &str) -> RoomLogLine {
        RoomLogLine {
            ts_ms: 10,
            profile_id: profile_id.to_owned(),
            name: name.to_owned(),
            kind: RoomLogKind::Say,
            text: text.to_owned(),
            iteration: None,
            phase: None,
            to_profile_id: None,
            to_name: None,
            reply: None,
        }
    }

    #[test]
    fn replace_and_delete_touch_only_the_matching_say() {
        let root = temp("edit");
        append_room_log(&root, "standup", &say("ann", "Ann", "one")).expect("ann");
        append_room_log(&root, "standup", &say("bea", "Bea", "two")).expect("bea");
        assert!(
            rewrite_room_say(&root, "standup", "ann", "one", Some("one final")).expect("replace")
        );
        assert!(!rewrite_room_say(&root, "standup", "ann", "missing", Some("nope")).expect("miss"));
        let raw = fs::read_to_string(root.join("standup/log.jsonl")).expect("log");
        let lines: Vec<RoomLogLine> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("row"))
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "one final");
        assert_eq!(lines[0].ts_ms, 10);
        assert_eq!(lines[0].profile_id, "ann");
        assert_eq!(lines[1].text, "two");
        assert!(rewrite_room_say(&root, "standup", "bea", "two", None).expect("delete"));
        let raw = fs::read_to_string(root.join("standup/log.jsonl")).expect("log");
        let lines: Vec<RoomLogLine> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("row"))
            .collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "one final");
        let _ = fs::remove_dir_all(&root);
    }
}
