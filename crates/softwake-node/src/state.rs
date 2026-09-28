//! Durable companion state: presence, leases, outbox, mirrored schedules,
//! soul / skills / tools, Telegram sticky ownership + mirrored vault.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use softwake_tools::{
    CATCH_UP_GRACE_MS, ScheduleActionKind, SchedulesFile, advance_after_fire, fire_notify_line,
    should_fire, skip_missed,
};

/// Grace without heartbeat → offline (90s).
pub const PRESENCE_GRACE_MS: u64 = 90_000;
/// Default lease TTL.
pub const LEASE_TTL_MS: u64 = 120_000;
/// Max outbox items retained.
pub const MAX_OUTBOX_ITEMS: usize = 500;

/// Laptop presence as reported / derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Present,
    Sleeping,
    Hibernated,
    Offline,
}

impl PresenceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Sleeping => "sleeping",
            Self::Hibernated => "hibernated",
            Self::Offline => "offline",
        }
    }
}

/// Who currently should own the Telegram long-poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramOwner {
    None,
    Laptop,
    Companion,
}

impl TelegramOwner {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Laptop => "laptop",
            Self::Companion => "companion",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceRecord {
    pub state: PresenceState,
    pub last_heartbeat_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease_id: String,
    pub profile_id: String,
    pub schedule_id: String,
    pub fire_ms: u64,
    pub claimer: String,
    pub expires_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: String,
    pub profile_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    pub ts_ms: u64,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
}

/// One skill row in `PUT /v1/skills`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirroredSkill {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub procedure: String,
    #[serde(default)]
    pub pitfalls: String,
    #[serde(default)]
    pub verify: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OutboxFile {
    #[serde(default)]
    items: Vec<OutboxItem>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LeasesFile {
    #[serde(default)]
    leases: Vec<LeaseRecord>,
}

/// Shared mutable node state.
pub struct NodeState {
    data_dir: PathBuf,
    presence: Mutex<PresenceRecord>,
    leases: Mutex<HashMap<String, LeaseRecord>>,
    outbox: Mutex<Vec<OutboxItem>>,
    /// `profile_id` → schedules
    schedules: Mutex<HashMap<String, SchedulesFile>>,
    /// Last claimed sticky owner (presence still wins for eligibility).
    telegram_owner: Mutex<TelegramOwner>,
}

impl NodeState {
    pub fn open(data_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&data_dir);
        let presence = PresenceRecord {
            state: PresenceState::Offline,
            last_heartbeat_ms: 0,
        };
        let leases = load_leases(&data_dir);
        let outbox = load_outbox(&data_dir);
        let schedules = load_all_schedules(&data_dir);
        let telegram_owner = load_telegram_owner(&data_dir);
        Self {
            data_dir,
            presence: Mutex::new(presence),
            leases: Mutex::new(leases),
            outbox: Mutex::new(outbox),
            schedules: Mutex::new(schedules),
            telegram_owner: Mutex::new(telegram_owner),
        }
    }

    pub fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn set_presence(&self, state: PresenceState, ts_ms: u64) {
        let mut g = self.presence.lock().expect("presence");
        g.state = state;
        g.last_heartbeat_ms = ts_ms;
    }

    pub fn effective_presence(&self, now: u64) -> PresenceRecord {
        let g = self.presence.lock().expect("presence");
        let mut rec = g.clone();
        if rec.last_heartbeat_ms == 0
            || now.saturating_sub(rec.last_heartbeat_ms) > PRESENCE_GRACE_MS
        {
            rec.state = PresenceState::Offline;
        }
        rec
    }

    pub fn laptop_is_present(&self, now: u64) -> bool {
        self.effective_presence(now).state == PresenceState::Present
    }

    fn lease_key(profile_id: &str, schedule_id: &str, fire_ms: u64) -> String {
        format!("{profile_id}|{schedule_id}|{fire_ms}")
    }

    /// Claim a fire lease. `Ok(lease_id)` or `Err(holder)`.
    pub fn claim_lease(
        &self,
        profile_id: &str,
        schedule_id: &str,
        fire_ms: u64,
        claimer: &str,
        ttl_ms: u64,
    ) -> Result<String, String> {
        let now = Self::now_ms();
        let key = Self::lease_key(profile_id, schedule_id, fire_ms);
        let mut map = self.leases.lock().expect("leases");
        map.retain(|_, v| v.expires_ms > now);
        if let Some(existing) = map.get(&key) {
            if existing.claimer != claimer {
                return Err(existing.claimer.clone());
            }
            return Ok(existing.lease_id.clone());
        }
        let lease_id = format!("lease-{now}-{}", map.len());
        let rec = LeaseRecord {
            lease_id: lease_id.clone(),
            profile_id: profile_id.to_owned(),
            schedule_id: schedule_id.to_owned(),
            fire_ms,
            claimer: claimer.to_owned(),
            expires_ms: now.saturating_add(ttl_ms.max(1_000)),
        };
        map.insert(key, rec);
        persist_leases(&self.data_dir, &map);
        Ok(lease_id)
    }

    pub fn push_outbox(&self, item: OutboxItem) {
        let mut items = self.outbox.lock().expect("outbox");
        items.push(item);
        while items.len() > MAX_OUTBOX_ITEMS {
            items.remove(0);
        }
        persist_outbox(&self.data_dir, &items);
    }

    pub fn outbox_since(&self, since_ts: u64, since_id: Option<&str>) -> Vec<OutboxItem> {
        let items = self.outbox.lock().expect("outbox");
        items
            .iter()
            .filter(|i| {
                if i.ts_ms > since_ts {
                    return true;
                }
                if let Some(sid) = since_id {
                    return i.ts_ms == since_ts && i.id.as_str() > sid;
                }
                false
            })
            .cloned()
            .collect()
    }

    pub fn put_schedules(&self, profile_id: &str, file: SchedulesFile) {
        let path = profile_schedules_path(&self.data_dir, profile_id);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            let _ = fs::write(&path, format!("{json}\n"));
        }
        let mut map = self.schedules.lock().expect("schedules");
        map.insert(profile_id.to_owned(), file);
    }

    /// Tick mirrored companion/auto schedules; returns fires performed.
    pub fn tick_schedules(&self) -> usize {
        let now = Self::now_ms();
        let laptop_present = self.laptop_is_present(now);
        let mut fired = 0_usize;
        let profile_ids: Vec<String> = {
            let map = self.schedules.lock().expect("schedules");
            map.keys().cloned().collect()
        };
        for profile_id in profile_ids {
            fired += self.tick_profile(&profile_id, now, laptop_present);
        }
        fired
    }

    #[allow(clippy::too_many_lines)]
    fn tick_profile(&self, profile_id: &str, now: u64, laptop_present: bool) -> usize {
        let mut map = self.schedules.lock().expect("schedules");
        let Some(file) = map.get_mut(profile_id) else {
            return 0;
        };
        let mut changed = false;
        let mut fired = 0_usize;
        let mut events: Vec<(
            String,
            String,
            u64,
            String,
            String,
            Option<softwake_tools::ScheduleEntry>,
        )> = Vec::new();
        for entry in &mut file.entries {
            if !entry.enabled {
                continue;
            }
            match entry.run_on {
                softwake_tools::ScheduleRunOn::Local => continue,
                softwake_tools::ScheduleRunOn::Companion => {}
                softwake_tools::ScheduleRunOn::Auto => {
                    if laptop_present {
                        continue;
                    }
                }
            }
            let Some(next) = entry.next_fire_ms else {
                continue;
            };
            if next > now {
                continue;
            }
            if should_fire(entry, now) {
                let fire_ms = next;
                match self.claim_lease(profile_id, &entry.id, fire_ms, "companion", LEASE_TTL_MS) {
                    Ok(lease_id) => {
                        match entry.action {
                            ScheduleActionKind::Notify => {
                                let summary = fire_notify_line(entry);
                                events.push((
                                    entry.id.clone(),
                                    lease_id,
                                    fire_ms,
                                    "fire_ack".to_owned(),
                                    summary,
                                    None,
                                ));
                            }
                            ScheduleActionKind::AgentTask => {
                                // Clone entry; run LLM after dropping the schedules lock.
                                events.push((
                                    entry.id.clone(),
                                    lease_id,
                                    fire_ms,
                                    "agent_task_result".to_owned(),
                                    String::new(),
                                    Some(entry.clone()),
                                ));
                            }
                        }
                        if advance_after_fire(entry, now).is_ok() {
                            changed = true;
                            fired += 1;
                        }
                    }
                    Err(_holder) => {
                        // Lost lease — still advance so we do not spin
                        if advance_after_fire(entry, now).is_ok() {
                            changed = true;
                        }
                    }
                }
            } else if now.saturating_sub(next) > CATCH_UP_GRACE_MS
                && skip_missed(entry, now).is_ok()
            {
                changed = true;
            }
        }
        if changed {
            let path = profile_schedules_path(&self.data_dir, profile_id);
            if let Ok(json) = serde_json::to_string_pretty(file) {
                let _ = fs::write(path, format!("{json}\n"));
            }
        }
        drop(map);
        for (schedule_id, lease_id, _fire_ms, kind, summary, agent_entry) in events {
            let summary = if let Some(entry) = agent_entry {
                crate::agent::run_schedule_agent_task(self, profile_id, &entry)
            } else {
                summary
            };
            // Telegram fan-out for notify rows when messengers want timer push.
            if kind == "fire_ack" {
                crate::telegram::maybe_fanout_timer(self, profile_id, &summary);
            }
            self.push_outbox(OutboxItem {
                id: format!("ob-{}-{schedule_id}", Self::now_ms()),
                profile_id: profile_id.to_owned(),
                kind,
                schedule_id: Some(schedule_id),
                ts_ms: Self::now_ms(),
                summary,
                lease_id: Some(lease_id),
            });
        }
        fired
    }

    /// Desired Telegram owner from presence + mirrored token.
    pub fn desired_telegram_owner(&self, now: u64) -> TelegramOwner {
        if !self.has_telegram_token() {
            // Solo / no mirror: laptop will keep polling when awake; companion cannot.
            if self.laptop_is_present(now) {
                return TelegramOwner::Laptop;
            }
            return TelegramOwner::None;
        }
        if self.laptop_is_present(now) {
            TelegramOwner::Laptop
        } else {
            TelegramOwner::Companion
        }
    }

    pub fn telegram_ownership_snapshot(&self, now: u64) -> (TelegramOwner, PresenceRecord, bool) {
        let desired = self.desired_telegram_owner(now);
        let presence = self.effective_presence(now);
        let has_token = self.has_telegram_token();
        // Align stored owner with desired whenever queried.
        {
            let mut g = self.telegram_owner.lock().expect("tg owner");
            if *g != desired {
                *g = desired;
                persist_telegram_owner(&self.data_dir, desired);
            }
        }
        (desired, presence, has_token)
    }

    /// Claim sticky ownership. `Ok(owner)` or `Err((owner, reason))`.
    pub fn claim_telegram_owner(
        &self,
        claimer: &str,
        now: u64,
    ) -> Result<TelegramOwner, (TelegramOwner, String)> {
        let desired = self.desired_telegram_owner(now);
        let want = match claimer {
            "laptop" => TelegramOwner::Laptop,
            "companion" => TelegramOwner::Companion,
            _ => return Err((desired, "bad claimer".into())),
        };
        if want != desired {
            return Err((desired, format!("desired owner is {}", desired.as_str())));
        }
        let mut g = self.telegram_owner.lock().expect("tg owner");
        *g = want;
        persist_telegram_owner(&self.data_dir, want);
        Ok(want)
    }

    pub fn has_telegram_token(&self) -> bool {
        read_vault_secret(&self.data_dir, "telegram_bot_token")
            .is_some_and(|t| !t.trim().is_empty())
    }

    pub fn telegram_bot_token(&self) -> Option<String> {
        read_vault_secret(&self.data_dir, "telegram_bot_token")
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
    }

    pub fn set_telegram_bot_token(&self, token: Option<&str>) {
        write_vault_secret(
            &self.data_dir,
            "telegram_bot_token",
            token.map(str::trim).filter(|t| !t.is_empty()),
        );
        // Recompute owner after token change.
        let now = Self::now_ms();
        let _ = self.telegram_ownership_snapshot(now);
    }

    pub fn xai_api_key(&self) -> Option<String> {
        if let Ok(env) = std::env::var("SOFTWAKE_NODE_XAI_API_KEY") {
            let t = env.trim().to_owned();
            if !t.is_empty() {
                return Some(t);
            }
        }
        read_vault_secret(&self.data_dir, "xai_api_key")
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
    }

    pub fn set_xai_api_key(&self, key: Option<&str>) {
        write_vault_secret(
            &self.data_dir,
            "xai_api_key",
            key.map(str::trim).filter(|t| !t.is_empty()),
        );
    }

    pub fn put_messengers(&self, profile_id: &str, body: &str) -> Result<(), String> {
        let parsed: softwake_tools::MessengersFile =
            serde_json::from_str(body).map_err(|e| e.to_string())?;
        let path = profile_messengers_path(&self.data_dir, profile_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&parsed).map_err(|e| e.to_string())?;
        write_mode_0600(&path, &format!("{json}\n")).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_messengers(&self, profile_id: &str) -> softwake_tools::MessengersFile {
        let path = profile_messengers_path(&self.data_dir, profile_id);
        let Ok(bytes) = fs::read(&path) else {
            return softwake_tools::MessengersFile::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    pub fn find_profile_for_chat(&self, chat_id: &str) -> String {
        let root = self.data_dir.join("profiles");
        let Ok(entries) = fs::read_dir(&root) else {
            return "default".into();
        };
        for entry in entries.flatten() {
            if !matches!(entry.file_type().map(|t| t.is_dir()), Ok(true)) {
                continue;
            }
            let profile_id = entry.file_name().to_string_lossy().into_owned();
            let file = self.load_messengers(&profile_id);
            if file
                .telegram
                .chat_id
                .as_ref()
                .is_some_and(|id| id.trim() == chat_id.trim())
            {
                return profile_id;
            }
        }
        "default".into()
    }

    /// Mirrored soul pack directory for a profile.
    #[must_use]
    pub fn profile_soul_dir(&self, profile_id: &str) -> PathBuf {
        self.data_dir.join("profiles").join(profile_id).join("soul")
    }

    /// Mirrored skills directory (global on the node).
    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }

    /// Path to mirrored tools.json.
    #[must_use]
    pub fn tools_path(&self) -> PathBuf {
        self.data_dir.join("tools.json")
    }

    /// Load mirrored Tools Settings (defaults when missing).
    #[must_use]
    pub fn load_tools_settings(&self) -> softwake_tools::ToolsSettings {
        let path = self.tools_path();
        softwake_tools::FileToolsSettings::new(&path)
            .ok()
            .and_then(|s| s.load().ok())
            .unwrap_or_default()
    }

    /// Persist mirrored soul pack (four markdown files).
    pub fn put_soul_pack(
        &self,
        profile_id: &str,
        soul_md: &str,
        user_md: &str,
        rules_md: &str,
        glossary_md: &str,
    ) -> Result<(), String> {
        let dir = self.profile_soul_dir(profile_id);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (name, body) in [
            ("soul.md", soul_md),
            ("user.md", user_md),
            ("rules.md", rules_md),
            ("glossary.md", glossary_md),
        ] {
            write_mode_0600(&dir.join(name), body).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Persist mirrored tools.json body.
    pub fn put_tools_json(&self, body: &str) -> Result<(), String> {
        let path = self.tools_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // Validate shape.
        let parsed: softwake_tools::ToolsSettings =
            serde_json::from_str(body).map_err(|e| e.to_string())?;
        let store = softwake_tools::FileToolsSettings::new(&path).map_err(|e| e.to_string())?;
        store.save(&parsed).map_err(|e| e.to_string())
    }

    /// Replace mirrored skills catalog.
    pub fn put_skills_catalog(&self, skills: &[MirroredSkill]) -> Result<(), String> {
        let dir = self.skills_dir();
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Clear existing .md skills
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    let _ = fs::remove_file(path);
                }
            }
        }
        for skill in skills.iter().take(softwake_skills::MAX_CATALOG_ENTRIES) {
            let source = match skill.source.as_str() {
                "agent" => softwake_skills::SkillSource::Agent,
                _ => softwake_skills::SkillSource::User,
            };
            let doc = softwake_skills::Skill {
                id: skill.id.clone(),
                title: skill.title.clone(),
                source,
                procedure: skill.procedure.clone(),
                pitfalls: skill.pitfalls.clone(),
                verify: skill.verify.clone(),
                updated: None,
            };
            softwake_skills::save_skill(&dir, doc).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Format mirrored schedules list for the schedule tool.
    #[must_use]
    pub fn format_schedule_list(&self, profile_id: &str) -> String {
        let map = self.schedules.lock().expect("schedules");
        let Some(file) = map.get(profile_id) else {
            return "no mirrored schedules".into();
        };
        if file.entries.is_empty() {
            return "no mirrored schedules".into();
        }
        file.entries
            .iter()
            .map(|e| {
                format!(
                    "{} action={} run_on={} enabled={} title={}",
                    e.id,
                    e.action.as_str(),
                    e.run_on.as_str(),
                    e.enabled,
                    e.title
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn bind_chat_id(&self, profile_id: &str, chat_id: &str) {
        let mut file = self.load_messengers(profile_id);
        let needs = file
            .telegram
            .chat_id
            .as_ref()
            .is_none_or(|c| c.trim().is_empty());
        if needs {
            file.telegram.chat_id = Some(chat_id.to_owned());
            file.telegram.enabled = true;
            if let Ok(json) = serde_json::to_string_pretty(&file) {
                let _ = self.put_messengers(profile_id, &json);
            }
        }
    }
}

fn data_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}

fn profile_schedules_path(dir: &Path, profile_id: &str) -> PathBuf {
    dir.join("profiles").join(profile_id).join("schedules.json")
}

fn load_outbox(dir: &Path) -> Vec<OutboxItem> {
    let path = data_file(dir, "outbox.json");
    let Ok(bytes) = fs::read(&path) else {
        return Vec::new();
    };
    serde_json::from_slice::<OutboxFile>(&bytes)
        .map(|f| f.items)
        .unwrap_or_default()
}

fn persist_outbox(dir: &Path, items: &[OutboxItem]) {
    let path = data_file(dir, "outbox.json");
    let file = OutboxFile {
        items: items.to_vec(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, format!("{json}\n"));
    }
}

fn load_leases(dir: &Path) -> HashMap<String, LeaseRecord> {
    let path = data_file(dir, "leases.json");
    let Ok(bytes) = fs::read(&path) else {
        return HashMap::new();
    };
    let file: LeasesFile = serde_json::from_slice(&bytes).unwrap_or_default();
    let mut map = HashMap::new();
    for lease in file.leases {
        let key = NodeState::lease_key(&lease.profile_id, &lease.schedule_id, lease.fire_ms);
        map.insert(key, lease);
    }
    map
}

fn persist_leases(dir: &Path, map: &HashMap<String, LeaseRecord>) {
    let path = data_file(dir, "leases.json");
    let file = LeasesFile {
        leases: map.values().cloned().collect(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, format!("{json}\n"));
    }
}

fn profile_messengers_path(dir: &Path, profile_id: &str) -> PathBuf {
    dir.join("profiles")
        .join(profile_id)
        .join("messengers.json")
}

fn vault_dir(dir: &Path) -> PathBuf {
    dir.join("vault")
}

fn read_vault_secret(dir: &Path, name: &str) -> Option<String> {
    let path = vault_dir(dir).join(name);
    fs::read_to_string(path).ok()
}

fn write_vault_secret(dir: &Path, name: &str, value: Option<&str>) {
    let root = vault_dir(dir);
    let _ = fs::create_dir_all(&root);
    let path = root.join(name);
    match value {
        Some(v) => {
            let _ = write_mode_0600(&path, &format!("{v}\n"));
        }
        None => {
            let _ = fs::remove_file(&path);
        }
    }
}

fn write_mode_0600(path: &Path, contents: &str) -> std::io::Result<()> {
    fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[derive(Deserialize)]
struct TelegramOwnerFile {
    owner: TelegramOwner,
}

fn load_telegram_owner(dir: &Path) -> TelegramOwner {
    let path = data_file(dir, "telegram-ownership.json");
    let Ok(bytes) = fs::read(&path) else {
        return TelegramOwner::None;
    };
    serde_json::from_slice::<TelegramOwnerFile>(&bytes).map_or(TelegramOwner::None, |f| f.owner)
}

fn persist_telegram_owner(dir: &Path, owner: TelegramOwner) {
    let path = data_file(dir, "telegram-ownership.json");
    let body = serde_json::json!({"owner": owner});
    if let Ok(json) = serde_json::to_string_pretty(&body) {
        let _ = fs::write(path, format!("{json}\n"));
    }
}

fn load_all_schedules(dir: &Path) -> HashMap<String, SchedulesFile> {
    let mut map = HashMap::new();
    let root = dir.join("profiles");
    let Ok(entries) = fs::read_dir(root) else {
        return map;
    };
    for entry in entries.flatten() {
        if !matches!(entry.file_type().map(|t| t.is_dir()), Ok(true)) {
            continue;
        }
        let profile_id = entry.file_name().to_string_lossy().into_owned();
        let path = profile_schedules_path(dir, &profile_id);
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(file) = serde_json::from_slice::<SchedulesFile>(&bytes) {
                map.insert(profile_id, file);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use softwake_tools::{ScheduleEntry, ScheduleKind, ScheduleRunOn};

    struct DropDir(PathBuf);
    impl Drop for DropDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn presence_grace_goes_offline() {
        let dir = std::env::temp_dir().join(format!("sw-node-pres-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let _guard = DropDir(dir.clone());
        let state = NodeState::open(dir);
        let now = NodeState::now_ms();
        state.set_presence(
            PresenceState::Present,
            now.saturating_sub(PRESENCE_GRACE_MS + 1),
        );
        assert_eq!(state.effective_presence(now).state, PresenceState::Offline);
        state.set_presence(PresenceState::Present, now);
        assert_eq!(state.effective_presence(now).state, PresenceState::Present);
    }

    #[test]
    fn lease_rejects_second_claimer() {
        let dir = std::env::temp_dir().join(format!("sw-node-lease-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let _guard = DropDir(dir.clone());
        let state = NodeState::open(dir);
        let a = state
            .claim_lease("default", "sched1", 1000, "laptop", LEASE_TTL_MS)
            .expect("first");
        let err = state
            .claim_lease("default", "sched1", 1000, "companion", LEASE_TTL_MS)
            .expect_err("second");
        assert_eq!(err, "laptop");
        let again = state
            .claim_lease("default", "sched1", 1000, "laptop", LEASE_TTL_MS)
            .expect("same claimer");
        assert_eq!(a, again);
    }

    #[test]
    fn outbox_persists() {
        let dir = std::env::temp_dir().join(format!("sw-node-ob-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let _guard = DropDir(dir.clone());
        let state = NodeState::open(dir.clone());
        state.push_outbox(OutboxItem {
            id: "a1".into(),
            profile_id: "default".into(),
            kind: "fire_ack".into(),
            schedule_id: Some("s".into()),
            ts_ms: 50,
            summary: "hello".into(),
            lease_id: None,
        });
        drop(state);
        let state2 = NodeState::open(dir);
        let items = state2.outbox_since(0, None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].summary, "hello");
    }

    #[test]
    fn companion_tick_fires_when_laptop_offline() {
        let dir = std::env::temp_dir().join(format!("sw-node-tick-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let _guard = DropDir(dir.clone());
        let state = NodeState::open(dir);
        let now = NodeState::now_ms();
        // no heartbeat → offline
        let entry = ScheduleEntry {
            id: "e1".into(),
            kind: ScheduleKind::Once,
            action: ScheduleActionKind::Notify,
            run_on: ScheduleRunOn::Companion,
            title: "t".into(),
            message: "m".into(),
            enabled: true,
            at_local: None,
            daily_time: None,
            cron: None,
            next_fire_ms: Some(now.saturating_sub(1_000)),
            last_fired_ms: None,
            created_ms: now,
            updated_ms: now,
        };
        let file = SchedulesFile {
            version: 1,
            timezone: softwake_tools::TIMEZONE_LOCAL.to_owned(),
            entries: vec![entry],
        };
        state.put_schedules("default", file);
        let n = state.tick_schedules();
        assert!(n >= 1, "expected fire");
        let items = state.outbox_since(0, None);
        assert!(!items.is_empty());
        assert!(items[0].summary.contains("timer"));
    }

    #[test]
    fn telegram_owner_follows_presence_and_token() {
        let dir = std::env::temp_dir().join(format!("sw-node-tg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let _guard = DropDir(dir.clone());
        let state = NodeState::open(dir);
        let now = NodeState::now_ms();
        // no token, offline → none
        assert_eq!(state.desired_telegram_owner(now), TelegramOwner::None);
        state.set_telegram_bot_token(Some("123:ABC"));
        assert!(state.has_telegram_token());
        // offline + token → companion
        assert_eq!(state.desired_telegram_owner(now), TelegramOwner::Companion);
        state.set_presence(PresenceState::Present, now);
        assert_eq!(state.desired_telegram_owner(now), TelegramOwner::Laptop);
        state.set_presence(PresenceState::Sleeping, now);
        assert_eq!(state.desired_telegram_owner(now), TelegramOwner::Companion);
        let claimed = state.claim_telegram_owner("companion", now).expect("claim");
        assert_eq!(claimed, TelegramOwner::Companion);
        let err = state
            .claim_telegram_owner("laptop", now)
            .expect_err("laptop blocked");
        assert_eq!(err.0, TelegramOwner::Companion);
        state.set_telegram_bot_token(None);
        assert!(!state.has_telegram_token());
    }

    #[test]
    fn messengers_mirror_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sw-node-msg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let _guard = DropDir(dir.clone());
        let state = NodeState::open(dir);
        let body = r#"{"version":1,"desktop":{"default":true,"receive_all":true,"voice":true},"telegram":{"enabled":true,"default":true,"receive_all":false,"voice":false,"chat_id":"42"}}"#;
        state.put_messengers("default", body).unwrap();
        let file = state.load_messengers("default");
        assert_eq!(file.telegram.chat_id.as_deref(), Some("42"));
        assert_eq!(state.find_profile_for_chat("42"), "default");
    }
}
