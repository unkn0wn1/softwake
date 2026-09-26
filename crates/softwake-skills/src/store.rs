//! Markdown skill files on disk.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::MAX_SECTION_BYTES;

const SKILLS_DIR_NAME: &str = "skills";
const SOFTWAKE_DIR_NAME: &str = "softwake";
const DOCUMENT_VERSION: u32 = 1;

/// Who authored the skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// Settings → Skills New/Save.
    #[default]
    User,
    /// `skill_save` tool after confirm.
    Agent,
}

impl SkillSource {
    /// Stable spelling for front matter and UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One skill document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// Stable id (`[a-z0-9-]{1,64}`). Filename stem.
    pub id: String,
    /// Operator-facing title.
    pub title: String,
    /// User or agent.
    pub source: SkillSource,
    /// Steps to follow.
    pub procedure: String,
    /// Known failure modes.
    pub pitfalls: String,
    /// How to check success.
    pub verify: String,
    /// Optional RFC3339-ish stamp (unix secs as decimal string when unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// Skill store failure.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    /// Id rejected.
    #[error("invalid skill id: {0}")]
    InvalidId(String),
    /// Title blank.
    #[error("skill title is blank")]
    BlankTitle,
    /// Section too large.
    #[error("skill section too large ({len} > {max})")]
    TooLarge {
        /// Observed length.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// Neither XDG data nor HOME is set.
    #[error("cannot resolve Softwake data directory: XDG_DATA_HOME and HOME are unset")]
    NoDataDir,
    /// Parse failure.
    #[error("invalid skill at {}: {source}", path.display())]
    Invalid {
        /// Path that failed.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// I/O failure.
    #[error("skill I/O at {}: {source}", path.display())]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Missing skill.
    #[error("unknown skill `{0}`")]
    NotFound(String),
}

/// Resolve the authored skills directory from the environment.
///
/// # Errors
///
/// [`SkillError::NoDataDir`] when neither `XDG_DATA_HOME` nor `HOME` is set.
pub fn resolve_skills_dir() -> Result<PathBuf, SkillError> {
    resolve_skills_dir_from(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"))
}

/// Resolve skills dir from explicit env values (tests).
///
/// # Errors
///
/// [`SkillError::NoDataDir`] when both are unset/empty.
pub fn resolve_skills_dir_from(
    xdg_data: Option<impl AsRef<std::ffi::OsStr>>,
    home: Option<impl AsRef<std::ffi::OsStr>>,
) -> Result<PathBuf, SkillError> {
    if let Some(xdg) = xdg_data {
        let path = PathBuf::from(xdg.as_ref());
        if !path.as_os_str().is_empty() {
            return Ok(path.join(SOFTWAKE_DIR_NAME).join(SKILLS_DIR_NAME));
        }
    }
    if let Some(home) = home {
        let path = PathBuf::from(home.as_ref());
        if !path.as_os_str().is_empty() {
            return Ok(path
                .join(".local")
                .join("share")
                .join(SOFTWAKE_DIR_NAME)
                .join(SKILLS_DIR_NAME));
        }
    }
    Err(SkillError::NoDataDir)
}

/// Slugify a title into a skill id.
#[must_use]
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in title.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
        if out.len() >= 64 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "skill".to_owned()
    } else {
        out
    }
}

/// Validate a skill id.
fn validate_id(id: &str) -> Result<(), SkillError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || id.starts_with('-')
        || id.ends_with('-')
        || id.contains("--")
    {
        return Err(SkillError::InvalidId(id.to_owned()));
    }
    Ok(())
}

fn validate_sections(skill: &Skill) -> Result<(), SkillError> {
    if skill.title.trim().is_empty() {
        return Err(SkillError::BlankTitle);
    }
    for part in [&skill.procedure, &skill.pitfalls, &skill.verify] {
        if part.len() > MAX_SECTION_BYTES {
            return Err(SkillError::TooLarge {
                len: part.len(),
                max: MAX_SECTION_BYTES,
            });
        }
    }
    Ok(())
}

fn skill_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.md"))
}

fn ensure_dir(dir: &Path) -> Result<(), SkillError> {
    #[cfg(unix)]
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|source| SkillError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir).map_err(|source| SkillError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn now_stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .to_string()
}

/// Encode a skill as Markdown with YAML front matter.
#[must_use]
pub fn encode(skill: &Skill) -> String {
    let updated = skill.updated.clone().unwrap_or_else(now_stamp);
    format!(
        "---\nversion: {DOCUMENT_VERSION}\ntitle: {title}\nsource: {source}\nupdated: {updated}\n---\n\n## Procedure\n\n{procedure}\n\n## Pitfalls\n\n{pitfalls}\n\n## Verify\n\n{verify}\n",
        title = yaml_escape(&skill.title),
        source = skill.source.as_str(),
        procedure = skill.procedure.trim_end(),
        pitfalls = skill.pitfalls.trim_end(),
        verify = skill.verify.trim_end(),
    )
}

fn yaml_escape(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .any(|c| matches!(c, ':' | '#' | '"' | '\'' | '\n' | '{' | '}' | '[' | ']'))
        || value.starts_with(' ')
        || value.ends_with(' ')
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

/// Parse a skill Markdown file.
///
/// # Errors
///
/// Missing front matter or required headings.
pub fn decode(id: &str, body: &str) -> Result<Skill, Box<dyn std::error::Error + Send + Sync>> {
    let (meta, rest) = split_front_matter(body)?;
    let title = meta_get(&meta, "title").unwrap_or(id).to_owned();
    let source = match meta_get(&meta, "source").unwrap_or("user") {
        "agent" => SkillSource::Agent,
        _ => SkillSource::User,
    };
    let updated = meta_get(&meta, "updated").map(str::to_owned);
    let procedure = section_body(rest, "procedure").unwrap_or_default();
    let pitfalls = section_body(rest, "pitfalls").unwrap_or_default();
    let verify = section_body(rest, "verify").unwrap_or_default();
    Ok(Skill {
        id: id.to_owned(),
        title,
        source,
        procedure,
        pitfalls,
        verify,
        updated,
    })
}

fn split_front_matter(
    body: &str,
) -> Result<(String, &str), Box<dyn std::error::Error + Send + Sync>> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("---") {
        return Err("missing YAML front matter".into());
    }
    let after = &trimmed[3..];
    let end = after
        .find("\n---")
        .ok_or("unterminated YAML front matter")?;
    let meta = after[..end].trim().to_owned();
    let rest = after[end + 4..].trim_start();
    Ok((meta, rest))
}

fn meta_get<'a>(meta: &'a str, key: &str) -> Option<&'a str> {
    for line in meta.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            let value = rest.trim();
            if let Some(inner) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                return Some(inner);
            }
            return Some(value);
        }
    }
    None
}

fn section_body(body: &str, name: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let needle = format!("## {name}");
    let start = lower.find(&needle)?;
    let after_heading = &body[start + needle.len()..];
    let after_heading = after_heading.strip_prefix('\n').unwrap_or(after_heading);
    let next = after_heading
        .to_ascii_lowercase()
        .find("\n## ")
        .unwrap_or(after_heading.len());
    Some(after_heading[..next].trim().to_owned())
}

/// List skills in `dir` (missing dir → empty).
///
/// # Errors
///
/// I/O when the directory exists but cannot be read.
pub fn list_skills(dir: &Path) -> Result<Vec<Skill>, SkillError> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(SkillError::Io {
                path: dir.to_path_buf(),
                source: error,
            });
        }
    };
    let mut skills = Vec::new();
    for entry in read {
        let entry = entry.map_err(|source| SkillError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if validate_id(stem).is_err() {
            continue;
        }
        match load_skill(dir, stem) {
            Ok(skill) => skills.push(skill),
            Err(SkillError::Invalid { .. }) => {}
            Err(error) => return Err(error),
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
}

/// Load one skill by id.
///
/// # Errors
///
/// Missing file or parse failure.
pub fn load_skill(dir: &Path, id: &str) -> Result<Skill, SkillError> {
    validate_id(id)?;
    let path = skill_path(dir, id);
    let bytes = fs::read(&path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            SkillError::NotFound(id.to_owned())
        } else {
            SkillError::Io {
                path: path.clone(),
                source,
            }
        }
    })?;
    let text = String::from_utf8(bytes).map_err(|source| SkillError::Invalid {
        path: path.clone(),
        source: Box::new(source),
    })?;
    decode(id, &text).map_err(|source| SkillError::Invalid { path, source })
}

/// Save a skill (create or replace).
///
/// # Errors
///
/// Validation or I/O.
pub fn save_skill(dir: &Path, mut skill: Skill) -> Result<Skill, SkillError> {
    if skill.id.trim().is_empty() {
        skill.id = slugify(&skill.title);
    }
    validate_id(&skill.id)?;
    validate_sections(&skill)?;
    skill.updated = Some(now_stamp());
    ensure_dir(dir)?;
    let path = skill_path(dir, &skill.id);
    let body = encode(&skill);
    write_atomic(&path, body.as_bytes())?;
    Ok(skill)
}

/// Delete a skill file.
///
/// # Errors
///
/// Missing id or I/O.
pub fn delete_skill(dir: &Path, id: &str) -> Result<(), SkillError> {
    validate_id(id)?;
    let path = skill_path(dir, id);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(SkillError::NotFound(id.to_owned()))
        }
        Err(error) => Err(SkillError::Io {
            path,
            source: error,
        }),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SkillError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_dir(parent)?;
    let tmp = path.with_extension("md.tmp");
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp).map_err(|source| SkillError::Io {
            path: tmp.clone(),
            source,
        })?;
        file.write_all(bytes).map_err(|source| SkillError::Io {
            path: tmp.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| SkillError::Io {
            path: tmp.clone(),
            source,
        })?;
    }
    fs::rename(&tmp, path).map_err(|source| SkillError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("softwake-skills-{nanos}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trip_user_skill() {
        let dir = temp_dir();
        let saved = save_skill(
            &dir,
            Skill {
                id: String::new(),
                title: "Check Swap".to_owned(),
                source: SkillSource::User,
                procedure: "Run free -h".to_owned(),
                pitfalls: "Do not sudo".to_owned(),
                verify: "Swap line appears".to_owned(),
                updated: None,
            },
        )
        .expect("save");
        assert_eq!(saved.id, "check-swap");
        assert_eq!(saved.source, SkillSource::User);
        let loaded = load_skill(&dir, "check-swap").expect("load");
        assert_eq!(loaded.title, "Check Swap");
        assert_eq!(loaded.procedure, "Run free -h");
        let listed = list_skills(&dir).expect("list");
        assert_eq!(listed.len(), 1);
        delete_skill(&dir, "check-swap").expect("delete");
        assert!(list_skills(&dir).expect("list").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_bad_id() {
        assert!(validate_id("Bad").is_err());
        assert!(validate_id("-x").is_err());
        assert!(validate_id("a--b").is_err());
    }

    #[test]
    fn list_missing_dir_is_empty() {
        let dir = temp_dir().join("missing");
        assert!(list_skills(&dir).expect("list").is_empty());
    }

    #[test]
    fn resolve_prefers_xdg_data() {
        let path = resolve_skills_dir_from(Some("/data"), Some("/home/me")).expect("dir");
        assert_eq!(path, PathBuf::from("/data/softwake/skills"));
    }
}
