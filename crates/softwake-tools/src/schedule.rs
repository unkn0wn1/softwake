//! Per-profile Softwake schedules (timers / reminders / cron-like).
//!
//! Persist under `profiles/<id>/schedules.json`. Wall clock is host local
//! (`chrono::Local`). Operators in Asia/Bangkok get Bangkok times when the
//! host TZ is `Asia/Bangkok`. Softwake does not install system crontab.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, NaiveDateTime, NaiveTime, Timelike, Weekday};
use serde::{Deserialize, Serialize};
use softwake_soul::{load_app_config, profile_pack_dir, resolve_config_dir};

/// File name under a profile pack directory.
pub const SCHEDULES_FILE_NAME: &str = "schedules.json";

/// Max entries kept per profile.
pub const MAX_ENTRIES: usize = 64;

/// Grace window for catch-up fires after softwaked was down (ms).
pub const CATCH_UP_GRACE_MS: u64 = 15 * 60 * 1000;

/// Documented timezone mode. v1 is always local wall clock.
pub const TIMEZONE_LOCAL: &str = "local";

/// On-disk schedules document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulesFile {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Always [`TIMEZONE_LOCAL`] in v1.
    #[serde(default = "timezone_local")]
    pub timezone: String,
    /// Schedule rows.
    #[serde(default)]
    pub entries: Vec<ScheduleEntry>,
}

fn one() -> u32 {
    1
}

fn timezone_local() -> String {
    TIMEZONE_LOCAL.to_owned()
}

impl Default for SchedulesFile {
    fn default() -> Self {
        Self {
            version: 1,
            timezone: TIMEZONE_LOCAL.to_owned(),
            entries: Vec::new(),
        }
    }
}

/// Kind of schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    /// One local datetime, then disabled after fire.
    Once,
    /// Every day at HH:MM local.
    Daily,
    /// Five-field cron subset (min hour dom mon dow).
    Cron,
}

impl ScheduleKind {
    /// Stable spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Daily => "daily",
            Self::Cron => "cron",
        }
    }
}

impl std::fmt::Display for ScheduleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happens when a schedule fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleActionKind {
    /// Fixed notify / TTS string (ADR-0024).
    #[default]
    Notify,
    /// Run a bounded agent turn with `message` as the prompt (ADR-0036).
    AgentTask,
}

impl ScheduleActionKind {
    /// Stable spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Notify => "notify",
            Self::AgentTask => "agent_task",
        }
    }
}

impl std::fmt::Display for ScheduleActionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One schedule row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleEntry {
    /// Stable id.
    pub id: String,
    /// Kind.
    pub kind: ScheduleKind,
    /// Fire behavior (`notify` default for back-compat).
    #[serde(default)]
    pub action: ScheduleActionKind,
    /// Short label.
    #[serde(default)]
    pub title: String,
    /// Spoken / notify body, or agent prompt when `action` is `agent_task`.
    pub message: String,
    /// Whether the scheduler may fire this row.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `once`: naive local `YYYY-MM-DDTHH:MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_local: Option<String>,
    /// `daily`: `HH:MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_time: Option<String>,
    /// `cron`: five fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    /// Next fire unix ms (local instant as unix).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire_ms: Option<u64>,
    /// Last fire unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_ms: Option<u64>,
    /// Created unix ms.
    pub created_ms: u64,
    /// Updated unix ms.
    pub updated_ms: u64,
}

fn default_true() -> bool {
    true
}

/// Schedule store / parse errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    /// Neither XDG config nor HOME is set.
    #[error("cannot resolve Softwake config directory: XDG_CONFIG_HOME and HOME are unset")]
    NoConfigDir,
    /// JSON or IO failure.
    #[error("{0}")]
    Io(String),
    /// Validation failure.
    #[error("{0}")]
    Invalid(String),
    /// Entry missing.
    #[error("schedule not found: {0}")]
    NotFound(String),
    /// Cap reached.
    #[error("schedule cap reached ({MAX_ENTRIES})")]
    CapReached,
}

impl From<softwake_soul::SoulError> for ScheduleError {
    fn from(value: softwake_soul::SoulError) -> Self {
        Self::Io(value.to_string())
    }
}

/// Resolve `profiles/<active>/schedules.json`.
///
/// # Errors
///
/// Returns [`ScheduleError::NoConfigDir`] when config cannot be resolved.
pub fn resolve_active_schedules_file() -> Result<PathBuf, ScheduleError> {
    let config = config_dir()?;
    let app = load_app_config(&config).unwrap_or_default();
    Ok(profile_pack_dir(&config, &app.active_profile).join(SCHEDULES_FILE_NAME))
}

/// Resolve schedules path for a profile id.
///
/// # Errors
///
/// Returns [`ScheduleError::NoConfigDir`] when config cannot be resolved.
pub fn resolve_schedules_file(profile_id: &str) -> Result<PathBuf, ScheduleError> {
    let config = config_dir()?;
    Ok(profile_pack_dir(&config, profile_id).join(SCHEDULES_FILE_NAME))
}

/// List profile ids that have a schedules file (and all known profiles if listing dirs).
///
/// # Errors
///
/// Returns config resolution errors.
pub fn list_profile_ids() -> Result<Vec<String>, ScheduleError> {
    let config = config_dir()?;
    let profiles = softwake_soul::list_profiles(&config).unwrap_or_default();
    Ok(profiles.into_iter().map(|p| p.id).collect())
}

fn config_dir() -> Result<PathBuf, ScheduleError> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|_| ScheduleError::NoConfigDir)
}

/// Load schedules from `path`. Missing file → empty default.
///
/// # Errors
///
/// Returns IO/JSON errors.
pub fn load_schedules(path: &Path) -> Result<SchedulesFile, ScheduleError> {
    if !path.exists() {
        return Ok(SchedulesFile::default());
    }
    let raw = fs::read_to_string(path).map_err(|e| ScheduleError::Io(e.to_string()))?;
    let mut file: SchedulesFile =
        serde_json::from_str(&raw).map_err(|e| ScheduleError::Io(e.to_string()))?;
    if file.version == 0 {
        file.version = 1;
    }
    if file.timezone.trim().is_empty() {
        TIMEZONE_LOCAL.clone_into(&mut file.timezone);
    }
    Ok(file)
}

/// Atomically write schedules JSON.
///
/// # Errors
///
/// Returns IO errors.
pub fn save_schedules(path: &Path, file: &SchedulesFile) -> Result<(), ScheduleError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ScheduleError::Io(e.to_string()))?;
    }
    let raw = serde_json::to_string_pretty(file).map_err(|e| ScheduleError::Io(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw.as_bytes()).map_err(|e| ScheduleError::Io(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| ScheduleError::Io(e.to_string()))?;
    Ok(())
}

/// Unix ms now.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Allocate a new id.
#[must_use]
pub fn new_schedule_id() -> String {
    format!("sch-{}", now_ms())
}

/// Parse `HH:MM`.
///
/// # Errors
///
/// Invalid time.
pub fn parse_hhmm(text: &str) -> Result<NaiveTime, ScheduleError> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text.trim(), "%H:%M:%S"))
        .map_err(|_| ScheduleError::Invalid(format!("invalid time (want HH:MM): {text}")))
}

/// Parse naive local `YYYY-MM-DDTHH:MM`.
///
/// # Errors
///
/// Invalid datetime.
pub fn parse_local_datetime(text: &str) -> Result<NaiveDateTime, ScheduleError> {
    let t = text.trim();
    NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M"))
        .map_err(|_| {
            ScheduleError::Invalid(format!(
                "invalid local datetime (want YYYY-MM-DDTHH:MM): {text}"
            ))
        })
}

/// Five-field cron field matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CronField {
    /// Allowed values (empty = any via wildcard stored differently).
    any: bool,
    values: Vec<u32>,
}

impl CronField {
    fn parse(raw: &str, min: u32, max: u32) -> Result<Self, ScheduleError> {
        let raw = raw.trim();
        if raw == "*" {
            return Ok(Self {
                any: true,
                values: Vec::new(),
            });
        }
        let mut values = Vec::new();
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some((a, b)) = part.split_once('-') {
                let start: u32 = a
                    .parse()
                    .map_err(|_| ScheduleError::Invalid(format!("bad cron range: {part}")))?;
                let end: u32 = b
                    .parse()
                    .map_err(|_| ScheduleError::Invalid(format!("bad cron range: {part}")))?;
                if start > end || start < min || end > max {
                    return Err(ScheduleError::Invalid(format!("cron out of range: {part}")));
                }
                values.extend(start..=end);
            } else {
                let n: u32 = part
                    .parse()
                    .map_err(|_| ScheduleError::Invalid(format!("bad cron value: {part}")))?;
                if n < min || n > max {
                    return Err(ScheduleError::Invalid(format!("cron out of range: {part}")));
                }
                values.push(n);
            }
        }
        values.sort_unstable();
        values.dedup();
        if values.is_empty() {
            return Err(ScheduleError::Invalid("empty cron field".to_owned()));
        }
        Ok(Self { any: false, values })
    }

    fn matches(&self, value: u32) -> bool {
        self.any || self.values.binary_search(&value).is_ok()
    }
}

/// Parsed five-field cron.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpr {
    minute: CronField,
    hour: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
}

impl CronExpr {
    /// Parse `min hour dom mon dow` (dow 0–6 Sun–Sat).
    ///
    /// # Errors
    ///
    /// Invalid expression.
    pub fn parse(text: &str) -> Result<Self, ScheduleError> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(ScheduleError::Invalid(format!(
                "cron needs 5 fields (min hour dom mon dow): {text}"
            )));
        }
        Ok(Self {
            minute: CronField::parse(parts[0], 0, 59)?,
            hour: CronField::parse(parts[1], 0, 23)?,
            dom: CronField::parse(parts[2], 1, 31)?,
            month: CronField::parse(parts[3], 1, 12)?,
            dow: CronField::parse(parts[4], 0, 6)?,
        })
    }

    fn matches_datetime(&self, dt: NaiveDateTime) -> bool {
        let dow = match dt.weekday() {
            Weekday::Sun => 0,
            Weekday::Mon => 1,
            Weekday::Tue => 2,
            Weekday::Wed => 3,
            Weekday::Thu => 4,
            Weekday::Fri => 5,
            Weekday::Sat => 6,
        };
        self.minute.matches(dt.minute())
            && self.hour.matches(dt.hour())
            && self.dom.matches(dt.day())
            && self.month.matches(dt.month())
            && self.dow.matches(dow)
    }
}

/// Convert local naive to unix ms using the host local offset at that wall time.
#[must_use]
pub fn naive_local_to_unix_ms(naive: NaiveDateTime) -> Option<u64> {
    let local = naive.and_local_timezone(Local).single()?;
    let ms = local.timestamp_millis();
    u64::try_from(ms).ok()
}

/// Next fire for an entry at or after `now` (unix ms), host local.
///
/// # Errors
///
/// Invalid kind fields.
pub fn compute_next_fire_ms(
    entry: &ScheduleEntry,
    now_ms: u64,
) -> Result<Option<u64>, ScheduleError> {
    if !entry.enabled {
        return Ok(None);
    }
    match entry.kind {
        ScheduleKind::Once => {
            let text = entry
                .at_local
                .as_deref()
                .ok_or_else(|| ScheduleError::Invalid("once requires at_local".to_owned()))?;
            let naive = parse_local_datetime(text)?;
            let fire = naive_local_to_unix_ms(naive)
                .ok_or_else(|| ScheduleError::Invalid("ambiguous local once time".to_owned()))?;
            if fire >= now_ms {
                Ok(Some(fire))
            } else {
                Ok(None)
            }
        }
        ScheduleKind::Daily => {
            let text = entry
                .daily_time
                .as_deref()
                .ok_or_else(|| ScheduleError::Invalid("daily requires daily_time".to_owned()))?;
            let time = parse_hhmm(text)?;
            let now = Local::now();
            let today = now.date_naive();
            let mut candidate = today.and_time(time);
            let mut fire = naive_local_to_unix_ms(candidate)
                .ok_or_else(|| ScheduleError::Invalid("ambiguous daily time".to_owned()))?;
            if fire < now_ms {
                let tomorrow = today
                    .succ_opt()
                    .ok_or_else(|| ScheduleError::Invalid("date overflow".to_owned()))?;
                candidate = tomorrow.and_time(time);
                fire = naive_local_to_unix_ms(candidate)
                    .ok_or_else(|| ScheduleError::Invalid("ambiguous daily time".to_owned()))?;
            }
            Ok(Some(fire))
        }
        ScheduleKind::Cron => {
            let text = entry
                .cron
                .as_deref()
                .ok_or_else(|| ScheduleError::Invalid("cron requires cron field".to_owned()))?;
            let expr = CronExpr::parse(text)?;
            let now = Local::now();
            // Scan minute-by-minute up to 366 days.
            let start = now.naive_local() + chrono::Duration::minutes(1);
            let start = start
                .with_second(0)
                .and_then(|t| t.with_nanosecond(0))
                .unwrap_or(start);
            for i in 0..(366 * 24 * 60) {
                let candidate = start + chrono::Duration::minutes(i);
                if expr.matches_datetime(candidate) {
                    if let Some(ms) = naive_local_to_unix_ms(candidate) {
                        if ms >= now_ms {
                            return Ok(Some(ms));
                        }
                    }
                }
            }
            Ok(None)
        }
    }
}

/// Ensure `next_fire_ms` is filled when possible.
///
/// # Errors
///
/// Propagates compute errors.
pub fn refresh_next_fire(entry: &mut ScheduleEntry, now_ms: u64) -> Result<(), ScheduleError> {
    entry.next_fire_ms = compute_next_fire_ms(entry, now_ms)?;
    Ok(())
}

/// Validate and normalize a new/edited entry (does not assign id/timestamps).
///
/// # Errors
///
/// Validation failures.
pub fn validate_entry(entry: &mut ScheduleEntry) -> Result<(), ScheduleError> {
    let message = entry.message.trim().to_owned();
    let title = entry.title.trim().to_owned();
    entry.message = message;
    entry.title = title;
    if entry.message.is_empty() {
        if entry.title.is_empty() {
            return Err(ScheduleError::Invalid(
                "message or title is required".to_owned(),
            ));
        }
        entry.message.clone_from(&entry.title);
    }
    if entry.title.is_empty() {
        entry.title.clone_from(&entry.message);
    }
    match entry.kind {
        ScheduleKind::Once => {
            let text = entry
                .at_local
                .as_deref()
                .ok_or_else(|| ScheduleError::Invalid("once requires at_local".to_owned()))?;
            let _ = parse_local_datetime(text)?;
            entry.daily_time = None;
            entry.cron = None;
        }
        ScheduleKind::Daily => {
            let text = entry
                .daily_time
                .as_deref()
                .ok_or_else(|| ScheduleError::Invalid("daily requires daily_time".to_owned()))?;
            let _ = parse_hhmm(text)?;
            entry.at_local = None;
            entry.cron = None;
        }
        ScheduleKind::Cron => {
            let text = entry
                .cron
                .as_deref()
                .ok_or_else(|| ScheduleError::Invalid("cron requires expression".to_owned()))?;
            let _ = CronExpr::parse(text)?;
            entry.at_local = None;
            entry.daily_time = None;
        }
    }
    Ok(())
}

/// Whether a due entry should fire now (catch-up grace).
#[must_use]
pub fn should_fire(entry: &ScheduleEntry, now_ms: u64) -> bool {
    if !entry.enabled {
        return false;
    }
    let Some(next) = entry.next_fire_ms else {
        return false;
    };
    if next > now_ms {
        return false;
    }
    now_ms.saturating_sub(next) <= CATCH_UP_GRACE_MS
}

/// Advance after a fire (or skip-miss).
///
/// # Errors
///
/// Compute errors.
pub fn advance_after_fire(entry: &mut ScheduleEntry, now_ms: u64) -> Result<(), ScheduleError> {
    entry.last_fired_ms = Some(now_ms);
    entry.updated_ms = now_ms;
    match entry.kind {
        ScheduleKind::Once => {
            entry.enabled = false;
            entry.next_fire_ms = None;
        }
        ScheduleKind::Daily | ScheduleKind::Cron => {
            // Move past the fired slot.
            let after = now_ms.saturating_add(1);
            entry.next_fire_ms = compute_next_fire_ms(entry, after)?;
        }
    }
    Ok(())
}

/// Skip a missed fire older than grace.
///
/// # Errors
///
/// Compute errors.
pub fn skip_missed(entry: &mut ScheduleEntry, now_ms: u64) -> Result<(), ScheduleError> {
    entry.updated_ms = now_ms;
    match entry.kind {
        ScheduleKind::Once => {
            entry.enabled = false;
            entry.next_fire_ms = None;
        }
        ScheduleKind::Daily | ScheduleKind::Cron => {
            entry.next_fire_ms = compute_next_fire_ms(entry, now_ms)?;
        }
    }
    Ok(())
}

/// Parsed mutate action for the `schedule` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleAction {
    /// Create a row.
    Create {
        /// Timing kind.
        kind: ScheduleKind,
        /// Fire action (`notify` vs `agent_task`).
        action: ScheduleActionKind,
        /// When field (`at_local` / `daily_time` / cron).
        when: String,
        /// Title/message body (rest of args).
        text: String,
    },
    /// Edit fields on an id.
    Edit {
        /// Id.
        id: String,
        /// Optional timing kind.
        kind: Option<ScheduleKind>,
        /// Optional fire action.
        action: Option<ScheduleActionKind>,
        /// Optional when string.
        when: Option<String>,
        /// Optional text.
        text: Option<String>,
        /// Optional enabled.
        enabled: Option<bool>,
    },
    /// Delete by id.
    Delete {
        /// Id.
        id: String,
    },
    /// List lines.
    List,
}

/// Parse `schedule` tool argv.
///
/// # Errors
///
/// Bad args.
#[allow(clippy::too_many_lines)]
pub fn parse_schedule_args(args: &[String]) -> Result<ScheduleAction, ScheduleError> {
    let mut iter = args.iter().map(String::as_str);
    let action = iter
        .next()
        .ok_or_else(|| ScheduleError::Invalid("schedule needs action".to_owned()))?
        .to_ascii_lowercase();
    match action.as_str() {
        "list" => Ok(ScheduleAction::List),
        "delete" => {
            let id = iter
                .next()
                .ok_or_else(|| ScheduleError::Invalid("schedule delete needs id".to_owned()))?;
            Ok(ScheduleAction::Delete { id: id.to_owned() })
        }
        "create" => {
            let first = iter
                .next()
                .ok_or_else(|| ScheduleError::Invalid("schedule create needs kind".to_owned()))?;
            let (action_kind, kind_raw) = if first.eq_ignore_ascii_case("agent_task")
                || first.eq_ignore_ascii_case("agent")
            {
                let kind_raw = iter.next().ok_or_else(|| {
                    ScheduleError::Invalid("schedule create agent_task needs kind".to_owned())
                })?;
                (ScheduleActionKind::AgentTask, kind_raw)
            } else if first.eq_ignore_ascii_case("notify") {
                let kind_raw = iter.next().ok_or_else(|| {
                    ScheduleError::Invalid("schedule create notify needs kind".to_owned())
                })?;
                (ScheduleActionKind::Notify, kind_raw)
            } else {
                (ScheduleActionKind::Notify, first)
            };
            let kind = parse_kind(kind_raw)?;
            let when = iter
                .next()
                .ok_or_else(|| ScheduleError::Invalid("schedule create needs when".to_owned()))?
                .to_owned();
            let rest: Vec<&str> = iter.collect();
            if rest.is_empty() {
                return Err(ScheduleError::Invalid(
                    "schedule create needs message text".to_owned(),
                ));
            }
            Ok(ScheduleAction::Create {
                kind,
                action: action_kind,
                when,
                text: rest.join(" "),
            })
        }
        "edit" => {
            let id = iter
                .next()
                .ok_or_else(|| ScheduleError::Invalid("schedule edit needs id".to_owned()))?
                .to_owned();
            let mut kind = None;
            let mut action = None;
            let mut when = None;
            let mut text = None;
            let mut enabled = None;
            let mut rest: Vec<&str> = Vec::new();
            while let Some(tok) = iter.next() {
                let lower = tok.to_ascii_lowercase();
                match lower.as_str() {
                    "kind" => {
                        let v = iter.next().ok_or_else(|| {
                            ScheduleError::Invalid("edit kind needs value".to_owned())
                        })?;
                        kind = Some(parse_kind(v)?);
                    }
                    "action" => {
                        let v = iter.next().ok_or_else(|| {
                            ScheduleError::Invalid("edit action needs value".to_owned())
                        })?;
                        action = Some(parse_action_kind(v)?);
                    }
                    "when" => {
                        let v = iter.next().ok_or_else(|| {
                            ScheduleError::Invalid("edit when needs value".to_owned())
                        })?;
                        when = Some(v.to_owned());
                    }
                    "enabled" => {
                        let v = iter.next().ok_or_else(|| {
                            ScheduleError::Invalid("edit enabled needs value".to_owned())
                        })?;
                        enabled = Some(parse_bool(v)?);
                    }
                    "message" | "text" | "title" => {
                        rest.extend(iter);
                        break;
                    }
                    _ => rest.push(tok),
                }
            }
            if !rest.is_empty() {
                text = Some(rest.join(" "));
            }
            Ok(ScheduleAction::Edit {
                id,
                kind,
                action,
                when,
                text,
                enabled,
            })
        }
        other => Err(ScheduleError::Invalid(format!(
            "unknown schedule action: {other} (want create|edit|delete|list)"
        ))),
    }
}

fn parse_kind(raw: &str) -> Result<ScheduleKind, ScheduleError> {
    match raw.to_ascii_lowercase().as_str() {
        "once" => Ok(ScheduleKind::Once),
        "daily" => Ok(ScheduleKind::Daily),
        "cron" => Ok(ScheduleKind::Cron),
        other => Err(ScheduleError::Invalid(format!(
            "unknown kind: {other} (want once|daily|cron)"
        ))),
    }
}

fn parse_action_kind(raw: &str) -> Result<ScheduleActionKind, ScheduleError> {
    match raw.to_ascii_lowercase().as_str() {
        "notify" | "reminder" => Ok(ScheduleActionKind::Notify),
        "agent_task" | "agent" | "task" => Ok(ScheduleActionKind::AgentTask),
        other => Err(ScheduleError::Invalid(format!(
            "unknown action: {other} (want notify|agent_task)"
        ))),
    }
}

fn parse_bool(raw: &str) -> Result<bool, ScheduleError> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(ScheduleError::Invalid(format!("bad bool: {other}"))),
    }
}

fn apply_when(entry: &mut ScheduleEntry, when: &str) -> Result<(), ScheduleError> {
    match entry.kind {
        ScheduleKind::Once => {
            let _ = parse_local_datetime(when)?;
            entry.at_local = Some(when.trim().to_owned());
        }
        ScheduleKind::Daily => {
            let _ = parse_hhmm(when)?;
            entry.daily_time = Some(when.trim().to_owned());
        }
        ScheduleKind::Cron => {
            let _ = CronExpr::parse(when)?;
            entry.cron = Some(when.trim().to_owned());
        }
    }
    Ok(())
}

/// Apply a mutate/list action to a schedules file. Returns operator detail.
///
/// # Errors
///
/// Validation / not found / cap.
#[allow(clippy::too_many_lines)]
pub fn apply_action(
    file: &mut SchedulesFile,
    action: &ScheduleAction,
    now_ms: u64,
) -> Result<String, ScheduleError> {
    match action {
        ScheduleAction::List => {
            if file.entries.is_empty() {
                return Ok("schedules: (none)".to_owned());
            }
            let lines: Vec<String> = file
                .entries
                .iter()
                .map(|e| {
                    format!(
                        "{} {} action={} {} enabled={} next={} | {} — {}",
                        e.id,
                        e.kind,
                        e.action,
                        when_label(e),
                        e.enabled,
                        e.next_fire_ms
                            .map_or_else(|| "-".to_owned(), |ms| ms.to_string()),
                        e.title,
                        e.message
                    )
                })
                .collect();
            Ok(format!("schedules:\n{}", lines.join("\n")))
        }
        ScheduleAction::Delete { id } => {
            let before = file.entries.len();
            file.entries.retain(|e| e.id != *id);
            if file.entries.len() == before {
                return Err(ScheduleError::NotFound(id.clone()));
            }
            Ok(format!("schedule deleted: {id}"))
        }
        ScheduleAction::Create {
            kind,
            action,
            when,
            text,
        } => {
            if file.entries.len() >= MAX_ENTRIES {
                return Err(ScheduleError::CapReached);
            }
            let mut entry = ScheduleEntry {
                id: new_schedule_id(),
                kind: *kind,
                action: *action,
                title: text.clone(),
                message: text.clone(),
                enabled: true,
                at_local: None,
                daily_time: None,
                cron: None,
                next_fire_ms: None,
                last_fired_ms: None,
                created_ms: now_ms,
                updated_ms: now_ms,
            };
            apply_when(&mut entry, when)?;
            validate_entry(&mut entry)?;
            refresh_next_fire(&mut entry, now_ms)?;
            let detail = format!(
                "schedule created: {} {} action={} {} next={}",
                entry.id,
                entry.kind,
                entry.action,
                when_label(&entry),
                entry
                    .next_fire_ms
                    .map_or_else(|| "-".to_owned(), |ms| ms.to_string())
            );
            file.entries.push(entry);
            Ok(detail)
        }
        ScheduleAction::Edit {
            id,
            kind,
            action,
            when,
            text,
            enabled,
        } => {
            let entry = file
                .entries
                .iter_mut()
                .find(|e| e.id == *id)
                .ok_or_else(|| ScheduleError::NotFound(id.clone()))?;
            if let Some(k) = kind {
                entry.kind = *k;
            }
            if let Some(a) = action {
                entry.action = *a;
            }
            if let Some(w) = when {
                apply_when(entry, w)?;
            }
            if let Some(t) = text {
                entry.title.clone_from(t);
                entry.message.clone_from(t);
            }
            if let Some(en) = enabled {
                entry.enabled = *en;
            }
            entry.updated_ms = now_ms;
            validate_entry(entry)?;
            refresh_next_fire(entry, now_ms)?;
            Ok(format!("schedule updated: {id}"))
        }
    }
}

fn when_label(entry: &ScheduleEntry) -> String {
    match entry.kind {
        ScheduleKind::Once => entry.at_local.clone().unwrap_or_default(),
        ScheduleKind::Daily => entry.daily_time.clone().unwrap_or_default(),
        ScheduleKind::Cron => entry.cron.clone().unwrap_or_default(),
    }
}

/// Max chars spoken for an agent-task result (TTS courtesy).
pub const AGENT_TASK_SPEAK_MAX: usize = 400;

/// Format fire notify / speak lines for fixed notify schedules.
#[must_use]
pub fn fire_notify_line(entry: &ScheduleEntry) -> String {
    if entry.title == entry.message {
        format!("timer: {}", entry.message)
    } else {
        format!("timer: {}: {}", entry.title, entry.message)
    }
}

/// Spoken reminder line for fixed notify schedules.
#[must_use]
pub fn fire_speak_line(entry: &ScheduleEntry) -> String {
    format!("Reminder: {}", entry.message)
}

/// Truncate text on a char boundary for TTS / short notify.
#[must_use]
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out = text.chars().take(max.saturating_sub(1)).collect::<String>();
    out.push('…');
    out
}

/// Notify sink line after an agent-task fire.
#[must_use]
pub fn fire_agent_notify_line(title: &str, summary: &str) -> String {
    let summary = summary.trim();
    let title = title.trim();
    if title.is_empty() || title == summary {
        format!("agent-task: {summary}")
    } else {
        format!("agent-task: {title}: {summary}")
    }
}

/// Spoken / Telegram body after an agent-task fire (truncated for TTS callers).
#[must_use]
pub fn fire_agent_speak_line(summary: &str) -> String {
    format!(
        "Agent task: {}",
        truncate_chars(summary.trim(), AGENT_TASK_SPEAK_MAX)
    )
}

/// User prompt text for a scheduled agent task (title context + message body).
#[must_use]
pub fn agent_task_user_prompt(entry: &ScheduleEntry) -> String {
    let title = entry.title.trim();
    let body = entry.message.trim();
    if title.is_empty() || title == body {
        format!("[scheduled agent task]\n{body}")
    } else {
        format!("[scheduled agent task: {title}]\n{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Hold dir open via path parent — use a simple struct
    struct Tmp {
        dir: PathBuf,
        path: PathBuf,
    }
    impl Tmp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "softwake-sched-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join(SCHEDULES_FILE_NAME);
            Self { dir, path }
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn round_trip_json() {
        let tmp = Tmp::new();
        let mut file = SchedulesFile::default();
        let mut entry = ScheduleEntry {
            id: "sch-1".into(),
            kind: ScheduleKind::Daily,
            action: ScheduleActionKind::Notify,
            title: "Standup".into(),
            message: "Standup".into(),
            enabled: true,
            at_local: None,
            daily_time: Some("07:30".into()),
            cron: None,
            next_fire_ms: None,
            last_fired_ms: None,
            created_ms: 1,
            updated_ms: 1,
        };
        refresh_next_fire(&mut entry, now_ms()).unwrap();
        file.entries.push(entry);
        save_schedules(&tmp.path, &file).unwrap();
        let loaded = load_schedules(&tmp.path).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].daily_time.as_deref(), Some("07:30"));
        assert!(loaded.entries[0].next_fire_ms.is_some());
    }

    #[test]
    fn cron_weekday_parses() {
        let expr = CronExpr::parse("0 9 * * 1-5").unwrap();
        let monday = NaiveDate::from_ymd_opt(2026, 9, 28)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        assert_eq!(monday.weekday(), Weekday::Mon);
        assert!(expr.matches_datetime(monday));
        let sunday = NaiveDate::from_ymd_opt(2026, 9, 27)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        assert_eq!(sunday.weekday(), Weekday::Sun);
        assert!(!expr.matches_datetime(sunday));
    }

    #[test]
    fn once_past_has_no_next() {
        let mut entry = ScheduleEntry {
            id: "sch-2".into(),
            kind: ScheduleKind::Once,
            action: ScheduleActionKind::Notify,
            title: "Past".into(),
            message: "Past".into(),
            enabled: true,
            at_local: Some("2020-01-01T00:00".into()),
            daily_time: None,
            cron: None,
            next_fire_ms: None,
            last_fired_ms: None,
            created_ms: 1,
            updated_ms: 1,
        };
        refresh_next_fire(&mut entry, now_ms()).unwrap();
        assert!(entry.next_fire_ms.is_none());
    }

    #[test]
    fn catch_up_grace() {
        let entry = ScheduleEntry {
            id: "sch-3".into(),
            kind: ScheduleKind::Once,
            action: ScheduleActionKind::Notify,
            title: "t".into(),
            message: "t".into(),
            enabled: true,
            at_local: None,
            daily_time: None,
            cron: None,
            next_fire_ms: Some(1_000),
            last_fired_ms: None,
            created_ms: 1,
            updated_ms: 1,
        };
        assert!(should_fire(&entry, 1_000 + CATCH_UP_GRACE_MS));
        assert!(!should_fire(&entry, 1_000 + CATCH_UP_GRACE_MS + 1));
        assert!(!should_fire(&entry, 500));
    }

    #[test]
    fn parse_create_daily() {
        let args = vec![
            "create".into(),
            "daily".into(),
            "07:30".into(),
            "Morning".into(),
            "note".into(),
        ];
        let action = parse_schedule_args(&args).unwrap();
        match action {
            ScheduleAction::Create {
                kind,
                action,
                when,
                text,
            } => {
                assert_eq!(kind, ScheduleKind::Daily);
                assert_eq!(action, ScheduleActionKind::Notify);
                assert_eq!(when, "07:30");
                assert_eq!(text, "Morning note");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn apply_create_and_list() {
        let mut file = SchedulesFile::default();
        let action = ScheduleAction::Create {
            kind: ScheduleKind::Daily,
            action: ScheduleActionKind::Notify,
            when: "08:00".into(),
            text: "Hi".into(),
        };
        let detail = apply_action(&mut file, &action, now_ms()).unwrap();
        assert!(detail.contains("schedule created"));
        assert_eq!(file.entries.len(), 1);
        let listed = apply_action(&mut file, &ScheduleAction::List, now_ms()).unwrap();
        assert!(listed.contains("Hi"));
        assert!(listed.contains("action=notify"));
    }

    #[test]
    fn missing_action_defaults_to_notify() {
        let raw = r#"{
            "version": 1,
            "timezone": "local",
            "entries": [{
                "id": "sch-old",
                "kind": "daily",
                "title": "Old",
                "message": "Old",
                "enabled": true,
                "daily_time": "07:00",
                "created_ms": 1,
                "updated_ms": 1
            }]
        }"#;
        let file: SchedulesFile = serde_json::from_str(raw).unwrap();
        assert_eq!(file.entries[0].action, ScheduleActionKind::Notify);
    }

    #[test]
    fn parse_create_agent_task() {
        let args = vec![
            "create".into(),
            "agent_task".into(),
            "daily".into(),
            "07:30".into(),
            "Summarize".into(),
            "inbox".into(),
        ];
        let action = parse_schedule_args(&args).unwrap();
        match action {
            ScheduleAction::Create {
                kind,
                action,
                when,
                text,
            } => {
                assert_eq!(kind, ScheduleKind::Daily);
                assert_eq!(action, ScheduleActionKind::AgentTask);
                assert_eq!(when, "07:30");
                assert_eq!(text, "Summarize inbox");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn agent_task_lines_and_truncate() {
        assert_eq!(
            fire_agent_notify_line("Inbox", "hello"),
            "agent-task: Inbox: hello"
        );
        assert_eq!(fire_agent_speak_line("hello"), "Agent task: hello");
        let long = "x".repeat(500);
        let spoken = fire_agent_speak_line(&long);
        assert!(spoken.chars().count() <= "Agent task: ".chars().count() + AGENT_TASK_SPEAK_MAX);
        assert!(spoken.ends_with('…'));
    }

    #[test]
    fn round_trip_agent_task_action() {
        let tmp = Tmp::new();
        let mut file = SchedulesFile::default();
        let mut entry = ScheduleEntry {
            id: "sch-a".into(),
            kind: ScheduleKind::Once,
            action: ScheduleActionKind::AgentTask,
            title: "Brief".into(),
            message: "Check calendar".into(),
            enabled: true,
            at_local: Some("2099-01-01T09:00".into()),
            daily_time: None,
            cron: None,
            next_fire_ms: None,
            last_fired_ms: None,
            created_ms: 1,
            updated_ms: 1,
        };
        refresh_next_fire(&mut entry, now_ms()).unwrap();
        file.entries.push(entry);
        save_schedules(&tmp.path, &file).unwrap();
        let loaded = load_schedules(&tmp.path).unwrap();
        assert_eq!(loaded.entries[0].action, ScheduleActionKind::AgentTask);
        let prompt = agent_task_user_prompt(&loaded.entries[0]);
        assert!(prompt.contains("Check calendar"));
        assert!(prompt.contains("Brief"));
    }
}
