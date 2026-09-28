//! Durable companion state: presence, leases, outbox, mirrored schedules.

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
        Self {
            data_dir,
            presence: Mutex::new(presence),
            leases: Mutex::new(leases),
            outbox: Mutex::new(outbox),
            schedules: Mutex::new(schedules),
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

    fn tick_profile(&self, profile_id: &str, now: u64, laptop_present: bool) -> usize {
        let mut map = self.schedules.lock().expect("schedules");
        let Some(file) = map.get_mut(profile_id) else {
            return 0;
        };
        let mut changed = false;
        let mut fired = 0_usize;
        let mut events: Vec<(String, String, u64, String, String)> = Vec::new();
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
                        let summary = match entry.action {
                            ScheduleActionKind::Notify => fire_notify_line(entry),
                            ScheduleActionKind::AgentTask => {
                                format!(
                                    "agent_task recorded for '{}': {} (full companion LLM deferred)",
                                    entry.title,
                                    truncate(&entry.message, 160)
                                )
                            }
                        };
                        let kind = match entry.action {
                            ScheduleActionKind::Notify => "fire_ack",
                            ScheduleActionKind::AgentTask => "agent_task_result",
                        };
                        events.push((
                            entry.id.clone(),
                            lease_id,
                            fire_ms,
                            kind.to_owned(),
                            summary,
                        ));
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
        for (schedule_id, lease_id, _fire_ms, kind, summary) in events {
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
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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
}
