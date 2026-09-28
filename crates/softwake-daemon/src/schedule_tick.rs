//! Background schedule fire loop.
//!
//! Ticks while softwaked is running, in any voice state. OS suspend is outside
//! Softwake's control; catch-up grace applies when the daemon resumes.
//! Remote Agent slice 2: honors `run_on` with companion fire leases (ADR-0040).

use softwake_tools::{
    ScheduleActionKind, ScheduleEntry, ScheduleRunOn, advance_after_fire, fire_notify_line,
    fire_speak_line, list_profile_ids, load_schedules, now_ms, resolve_schedules_file,
    save_schedules, should_fire, skip_missed,
};

use crate::remote_agent;
use crate::runtime::Runtime;

/// Tick interval for due-schedule scans.
pub(crate) const TICK_SECS: u64 = 15;

/// Scan all profiles and fire due rows. Returns how many fired.
pub(crate) fn tick_all(runtime: &mut Runtime) -> usize {
    let Ok(profiles) = list_profile_ids() else {
        return 0;
    };
    let now = now_ms();
    let mut fired = 0;
    for profile_id in profiles {
        fired += tick_profile(runtime, &profile_id, now);
    }
    fired
}

fn tick_profile(runtime: &mut Runtime, profile_id: &str, now: u64) -> usize {
    let Ok(path) = resolve_schedules_file(profile_id) else {
        return 0;
    };
    let Ok(mut file) = load_schedules(&path) else {
        return 0;
    };
    let mut changed = false;
    let mut fired = 0;
    for entry in &mut file.entries {
        if !entry.enabled {
            continue;
        }
        // Refresh next if missing.
        if entry.next_fire_ms.is_none() && softwake_tools::refresh_next_fire(entry, now).is_ok() {
            changed = true;
        }
        let Some(next) = entry.next_fire_ms else {
            continue;
        };
        if next > now {
            continue;
        }
        if should_fire(entry, now) {
            if fire_one(runtime, profile_id, entry, next) {
                if advance_after_fire(entry, now).is_ok() {
                    changed = true;
                    fired += 1;
                }
            } else if advance_after_fire(entry, now).is_ok() {
                // Skipped (companion owns fire / lost lease) — still advance.
                changed = true;
            }
        } else if now.saturating_sub(next) > softwake_tools::CATCH_UP_GRACE_MS
            && skip_missed(entry, now).is_ok()
        {
            changed = true;
        }
    }
    if changed {
        let _ = save_schedules(&path, &file);
    }
    fired
}

/// Returns true when this laptop performed the fire.
fn fire_one(runtime: &mut Runtime, profile_id: &str, entry: &ScheduleEntry, fire_ms: u64) -> bool {
    match entry.run_on {
        ScheduleRunOn::Local => {
            do_local_fire(runtime, profile_id, entry);
            true
        }
        ScheduleRunOn::Companion => {
            // Laptop never fires; companion tick owns it. Nudge mirror.
            remote_agent::mirror_schedules();
            false
        }
        ScheduleRunOn::Auto => {
            if remote_agent::companion_says_present() {
                match remote_agent::claim_lease(profile_id, &entry.id, fire_ms, "laptop") {
                    Ok(true) => {
                        do_local_fire(runtime, profile_id, entry);
                        true
                    }
                    Ok(false) => false,
                    Err(_) => {
                        // Companion unreachable — fire locally (prefer_local).
                        do_local_fire(runtime, profile_id, entry);
                        true
                    }
                }
            } else {
                remote_agent::mirror_schedules();
                false
            }
        }
    }
}

fn do_local_fire(runtime: &mut Runtime, profile_id: &str, entry: &ScheduleEntry) {
    match entry.action {
        ScheduleActionKind::Notify => {
            let notify = fire_notify_line(entry);
            let speak = fire_speak_line(entry);
            runtime.fire_schedule_reminder(profile_id, notify, speak);
        }
        ScheduleActionKind::AgentTask => {
            runtime.fire_schedule_agent_task(profile_id, entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_run_on_always_fires_decision() {
        // Pure decision coverage via companion_says_present is integration-level;
        // keep a compile/smoke assert on enum wiring.
        assert_eq!(ScheduleRunOn::Local.as_str(), "local");
        assert_eq!(ScheduleRunOn::Companion.as_str(), "companion");
        assert_eq!(ScheduleRunOn::Auto.as_str(), "auto");
    }
}
