//! Turns consecutive printer statuses into alerts. Pure logic (no I/O) so it can be unit tested.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Spool level (percent) at or below which the spool in use triggers "Filament low".
const LOW_PCT: i64 = 15;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertPrefs {
    pub finished: bool,
    pub failed: bool,
    pub paused: bool,
    /// New printer warnings (HMS), including out of filament.
    pub error: bool,
    pub filament_low: bool,
    pub offline: bool,
    /// Open the monitor window for alerts that need you (error, out of filament, paused).
    pub open_window: bool,
}

impl Default for AlertPrefs {
    fn default() -> Self {
        AlertPrefs { finished: true, failed: true, paused: true, error: true, filament_low: true, offline: true, open_window: true }
    }
}

#[derive(Debug, PartialEq)]
pub struct Alert {
    pub title: String,
    pub body: String,
    /// The printer is waiting on you.
    pub needs_you: bool,
}

/// What we remember between polls.
#[derive(Default)]
pub struct Memory {
    seen_hms: HashMap<i64, HashSet<String>>,
    warned_low: HashSet<(i64, i64)>,
    connected: HashMap<i64, bool>,
}

fn is_active(s: &str) -> bool {
    matches!(s, "RUNNING" | "PREPARE" | "PAUSE" | "SLICING")
}

fn is_runout(desc: &str) -> bool {
    let d = desc.to_lowercase();
    d.contains("out of filament")
        || (d.contains("filament") && (d.contains("run out") || d.contains("ran out") || d.contains("runout") || d.contains("empty")))
}

fn slot_label(unit: i64, slot: i64) -> String {
    let letter = (b'A' + unit.clamp(0, 25) as u8) as char;
    format!("AMS {letter} slot {}", slot + 1)
}

/// `old` is the previous state of this printer (None on the first look after launch or a settings change,
/// when nothing is reported so a restart does not replay old problems).
pub fn detect(prefs: &AlertPrefs, pid: i64, old: Option<&str>, s: &Value, mem: &mut Memory) -> Vec<Alert> {
    let name = s["name"].as_str().unwrap_or("Printer");
    let new = s["state"].as_str().unwrap_or("");
    let job = s["subtask_name"].as_str().or_else(|| s["current_print"].as_str()).filter(|j| !j.is_empty());
    let with_job = |what: &str| match job {
        Some(j) => format!("{name}: {j} {what}"),
        None => format!("{name}: {what}"),
    };
    let first = old.is_none();
    let mut out = Vec::new();

    let connected = s["connected"].as_bool().unwrap_or(false);
    let was = mem.connected.insert(pid, connected);
    if !connected {
        if was == Some(true) && prefs.offline {
            out.push(Alert { title: "Printer offline".into(), body: format!("{name} has disconnected"), needs_you: false });
        }
        return out;
    }

    // New printer warnings. The first look only records them.
    let current: Vec<(String, String)> = s["hms_errors"]
        .as_array()
        .map(|v| {
            v.iter()
                .map(|e| {
                    let key = e["full_code"].as_str().or_else(|| e["code"].as_str()).unwrap_or("?").to_string();
                    let desc = e["description"].as_str().filter(|d| !d.is_empty()).unwrap_or(&key).to_string();
                    (key, desc)
                })
                .collect()
        })
        .unwrap_or_default();
    let seen = mem.seen_hms.entry(pid).or_default();
    let mut warned = false;
    for (key, desc) in &current {
        if seen.insert(key.clone()) && !first && prefs.error {
            let title = if is_runout(desc) { "Out of filament" } else { "Printer needs attention" };
            out.push(Alert { title: title.into(), body: format!("{name}: {desc}"), needs_you: true });
            warned = true;
        }
    }
    seen.retain(|k| current.iter().any(|(c, _)| c == k));

    let was_active = old.is_some_and(is_active);
    if !was_active && is_active(new) {
        mem.warned_low.retain(|(p, _)| *p != pid);
    }

    if was_active && old != Some(new) {
        match new {
            "FINISH" if prefs.finished => out.push(Alert { title: "Print finished".into(), body: with_job("is done"), needs_you: false }),
            "FAILED" if prefs.failed => out.push(Alert { title: "Print failed".into(), body: with_job("failed or was stopped"), needs_you: false }),
            "PAUSE" if prefs.paused && !warned => out.push(Alert { title: "Print paused".into(), body: with_job("is paused"), needs_you: true }),
            _ => {}
        }
    }

    // The spool in use is running low (tray_now = unit * 4 + slot for a regular AMS).
    if prefs.filament_low && is_active(new) {
        if let Some(t) = s["tray_now"].as_i64().filter(|t| (0..16).contains(t)) {
            let (unit, slot) = (t / 4, t % 4);
            let remain = s["ams"]
                .as_array()
                .and_then(|u| u.iter().find(|u| u["id"].as_i64() == Some(unit)))
                .and_then(|u| u["tray"].as_array())
                .and_then(|trays| trays.iter().find(|tr| tr["id"].as_i64() == Some(slot)))
                .and_then(|tr| tr["remain"].as_i64());
            if let Some(r) = remain.filter(|r| (0..=LOW_PCT).contains(r)) {
                if mem.warned_low.insert((pid, t)) && !first {
                    out.push(Alert {
                        title: "Filament low".into(),
                        body: format!("{name}: {} is at {r}%", slot_label(unit, slot)),
                        needs_you: false,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn st(state: &str) -> Value {
        json!({"name": "P1S", "state": state, "connected": true, "subtask_name": "Benchy", "hms_errors": [], "ams": [], "tray_now": 255})
    }

    fn run(old: Option<&str>, s: &Value, mem: &mut Memory) -> Vec<Alert> {
        detect(&AlertPrefs::default(), 1, old, s, mem)
    }

    #[test]
    fn finish_fail_pause() {
        let mut m = Memory::default();
        assert_eq!(run(Some("RUNNING"), &st("FINISH"), &mut m)[0].title, "Print finished");
        assert_eq!(run(Some("RUNNING"), &st("FAILED"), &mut m)[0].title, "Print failed");
        let p = run(Some("RUNNING"), &st("PAUSE"), &mut m);
        assert_eq!(p[0].title, "Print paused");
        assert!(p[0].needs_you);
        assert!(run(Some("PAUSE"), &st("PAUSE"), &mut m).is_empty());
        assert!(run(Some("IDLE"), &st("FINISH"), &mut m).is_empty());
    }

    #[test]
    fn first_look_is_silent() {
        let mut m = Memory::default();
        let mut s = st("RUNNING");
        s["hms_errors"] = json!([{"full_code": "A", "description": "Boom"}]);
        assert!(run(None, &s, &mut m).is_empty());
        // same error again: still silent
        assert!(run(Some("RUNNING"), &s, &mut m).is_empty());
    }

    #[test]
    fn runout_is_one_alert_not_two() {
        let mut m = Memory::default();
        run(Some("IDLE"), &st("RUNNING"), &mut m);
        let mut s = st("PAUSE");
        s["hms_errors"] = json!([{"full_code": "07008011", "description": "AMS A Slot 1 filament has run out."}]);
        let a = run(Some("RUNNING"), &s, &mut m);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].title, "Out of filament");
        assert!(a[0].needs_you);
        // error cleared then it comes back: alerts again
        run(Some("PAUSE"), &st("RUNNING"), &mut m);
        assert_eq!(run(Some("RUNNING"), &s, &mut m).len(), 1);
    }

    #[test]
    fn other_error() {
        let mut m = Memory::default();
        run(Some("RUNNING"), &st("RUNNING"), &mut m);
        let mut s = st("RUNNING");
        s["hms_errors"] = json!([{"code": "0300_0100", "description": "Nozzle temperature abnormal"}]);
        assert_eq!(run(Some("RUNNING"), &s, &mut m)[0].title, "Printer needs attention");
    }

    #[test]
    fn filament_low_once_per_print() {
        let mut m = Memory::default();
        let mut s = st("RUNNING");
        s["tray_now"] = json!(1);
        s["ams"] = json!([{"id": 0, "tray": [{"id": 0, "remain": 80}, {"id": 1, "remain": 12}]}]);
        let a = run(Some("RUNNING"), &s, &mut m);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].body, "P1S: AMS A slot 2 is at 12%");
        assert!(run(Some("RUNNING"), &s, &mut m).is_empty());
        // a new print re-arms it
        run(Some("FINISH"), &st("RUNNING"), &mut m);
        assert_eq!(run(Some("RUNNING"), &s, &mut m).len(), 1);
        // unknown level (-1) never alerts, and a healthy spool never alerts
        s["ams"] = json!([{"id": 0, "tray": [{"id": 1, "remain": -1}]}]);
        assert!(run(Some("RUNNING"), &s, &mut Memory::default()).is_empty());
    }

    #[test]
    fn offline_and_toggles() {
        let mut m = Memory::default();
        run(Some("RUNNING"), &st("RUNNING"), &mut m);
        let mut s = st("RUNNING");
        s["connected"] = json!(false);
        assert_eq!(run(Some("RUNNING"), &s, &mut m)[0].title, "Printer offline");
        assert!(run(Some("RUNNING"), &s, &mut m).is_empty());
        let off = AlertPrefs { finished: false, ..AlertPrefs::default() };
        assert!(detect(&off, 1, Some("RUNNING"), &st("FINISH"), &mut Memory::default()).is_empty());
    }
}
