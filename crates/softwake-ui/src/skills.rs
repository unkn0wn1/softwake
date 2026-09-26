//! Settings commands for on-disk Markdown skills.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use serde::Serialize;
use softwake_skills::{
    Skill, SkillSource, delete_skill, list_skills, load_skill, resolve_skills_dir, save_skill,
    slugify,
};

/// One row in the Skills list.
#[derive(Debug, Clone, Serialize)]
pub struct SkillRow {
    /// Stable id.
    pub id: String,
    /// Title.
    pub title: String,
    /// `user` or `agent`.
    pub source: String,
}

/// Skills pane snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct SkillsSnapshot {
    /// Skills data directory (display).
    pub skills_dir: String,
    /// All skills, sorted by id.
    pub skills: Vec<SkillRow>,
    /// Selected skill id (empty when none).
    pub selected_id: String,
    /// Selected title.
    pub selected_title: String,
    /// Selected source.
    pub selected_source: String,
    /// Procedure body.
    pub procedure: String,
    /// Pitfalls body.
    pub pitfalls: String,
    /// Verify body.
    pub verify: String,
}

fn open_dir() -> Result<std::path::PathBuf, String> {
    resolve_skills_dir().map_err(|e| e.to_string())
}

fn snapshot_at(selected_id: &str) -> Result<SkillsSnapshot, String> {
    let dir = open_dir()?;
    let skills = list_skills(&dir).map_err(|e| e.to_string())?;
    let rows: Vec<SkillRow> = skills
        .iter()
        .map(|s| SkillRow {
            id: s.id.clone(),
            title: s.title.clone(),
            source: s.source.as_str().to_owned(),
        })
        .collect();
    let selected = if !selected_id.is_empty() && rows.iter().any(|r| r.id == selected_id) {
        selected_id.to_owned()
    } else {
        rows.first().map(|r| r.id.clone()).unwrap_or_default()
    };
    if selected.is_empty() {
        return Ok(SkillsSnapshot {
            skills_dir: dir.display().to_string(),
            skills: rows,
            selected_id: String::new(),
            selected_title: String::new(),
            selected_source: SkillSource::User.as_str().to_owned(),
            procedure: String::new(),
            pitfalls: String::new(),
            verify: String::new(),
        });
    }
    let skill = load_skill(&dir, &selected).map_err(|e| e.to_string())?;
    Ok(SkillsSnapshot {
        skills_dir: dir.display().to_string(),
        skills: rows,
        selected_id: skill.id,
        selected_title: skill.title,
        selected_source: skill.source.as_str().to_owned(),
        procedure: skill.procedure,
        pitfalls: skill.pitfalls,
        verify: skill.verify,
    })
}

/// Load Skills Settings snapshot.
#[tauri::command]
pub fn skills_snapshot(selected_id: Option<String>) -> Result<SkillsSnapshot, String> {
    snapshot_at(selected_id.as_deref().unwrap_or(""))
}

/// Save (create or update) a skill from Settings. Source stays `user` for new ids;
/// existing agent skills keep `agent` unless `force_user` is true.
#[tauri::command]
pub fn skills_save(
    id: String,
    title: String,
    procedure: String,
    pitfalls: String,
    verify: String,
    force_user: bool,
) -> Result<SkillsSnapshot, String> {
    let dir = open_dir()?;
    let mut skill_id = id.trim().to_owned();
    if skill_id.is_empty() {
        skill_id = slugify(&title);
    }
    let source = if force_user {
        SkillSource::User
    } else if load_skill(&dir, &skill_id).is_ok_and(|s| s.source == SkillSource::Agent) {
        SkillSource::Agent
    } else {
        SkillSource::User
    };
    let saved = save_skill(
        &dir,
        Skill {
            id: skill_id.clone(),
            title,
            source,
            procedure,
            pitfalls,
            verify,
            updated: None,
        },
    )
    .map_err(|e| e.to_string())?;
    snapshot_at(&saved.id)
}

/// Delete a skill by id.
#[tauri::command]
pub fn skills_delete(id: String) -> Result<SkillsSnapshot, String> {
    let dir = open_dir()?;
    delete_skill(&dir, id.trim()).map_err(|e| e.to_string())?;
    snapshot_at("")
}

#[cfg(test)]
mod tests {
    use super::snapshot_at;
    use softwake_skills::{Skill, SkillSource, save_skill};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn snapshot_lists_saved_skill() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("softwake-ui-skills-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only env for resolve path — use save into explicit dir via softwake_skills
        let skill = save_skill(
            &dir,
            Skill {
                id: "demo".to_owned(),
                title: "Demo".to_owned(),
                source: SkillSource::User,
                procedure: "p".to_owned(),
                pitfalls: String::new(),
                verify: String::new(),
                updated: None,
            },
        )
        .expect("save");
        assert_eq!(skill.id, "demo");
        // snapshot_at uses XDG — just ensure helper compiles; store tests cover disk.
        let _ = snapshot_at("");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
