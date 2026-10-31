//! The Rotation timer. The Daemon owns only the schedule: whether Rotation
//! runs and when each Monitor changes next. `vellum render --rotate` picks
//! the wallpapers (ADR 0003).
//!
//! Deadlines are wall-clock milliseconds since the Unix epoch, so they
//! survive a restart (saved in State) and a suspend: a deadline that passed
//! while the machine slept fires right away. The Daemon arms one
//! `CLOCK_REALTIME` timerfd at the earliest deadline, so it makes no
//! wakeups between ticks.
//!
//! With `sync = true`, the Monitors following the global Rotation form one
//! sync group: they share a deadline, tick together and reset together.

use std::collections::BTreeMap;

use vellum_ipc::RotationSettings;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rotation {
    pub running: bool,
    /// Monitor name to its next change, in Unix milliseconds.
    pub deadlines: BTreeMap<String, u64>,
}

impl Rotation {
    /// Starts Rotation: every connected Monitor changes one interval from
    /// now. Returns whether anything changed (already running: no).
    pub fn start(&mut self, now: u64, connected: &[String], settings: &RotationSettings) -> bool {
        if self.running {
            return false;
        }
        self.running = true;
        self.deadlines.clear();
        self.ensure(now, connected, settings);
        true
    }

    /// Stops Rotation. Returns whether it was running.
    pub fn stop(&mut self) -> bool {
        let was = self.running;
        self.running = false;
        self.deadlines.clear();
        was
    }

    /// Gives every connected Monitor without a deadline one: its sync
    /// group's, or one interval from now. Returns whether any was added.
    pub fn ensure(&mut self, now: u64, connected: &[String], settings: &RotationSettings) -> bool {
        if !self.running {
            return false;
        }
        let mut added = false;
        for monitor in connected {
            if self.deadlines.contains_key(monitor) {
                continue;
            }
            let deadline = self
                .group_deadline(monitor, connected, settings)
                .unwrap_or_else(|| now + interval(settings, monitor));
            self.deadlines.insert(monitor.clone(), deadline);
            added = true;
        }
        added
    }

    /// New settings: no deadline may lie further out than one (new)
    /// interval, so a shortened interval takes effect at once. Returns
    /// whether anything changed.
    pub fn settings_changed(&mut self, now: u64, settings: &RotationSettings) -> bool {
        let mut changed = false;
        for (monitor, deadline) in &mut self.deadlines {
            let latest = now + interval(settings, monitor);
            if *deadline > latest {
                *deadline = latest;
                changed = true;
            }
        }
        if settings.sync {
            // Re-align the sync group on its earliest member.
            let group: Vec<String> = self
                .deadlines
                .keys()
                .filter(|m| settings.in_sync_group(m))
                .cloned()
                .collect();
            if let Some(&earliest) = group.iter().filter_map(|m| self.deadlines.get(m)).min() {
                for m in &group {
                    let d = self.deadlines.get_mut(m).expect("listed above");
                    if *d != earliest {
                        *d = earliest;
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// A manual apply on `monitor`: its timer (or its sync group's)
    /// restarts. Returns whether anything changed.
    pub fn reset(
        &mut self,
        monitor: &str,
        now: u64,
        connected: &[String],
        settings: &RotationSettings,
    ) -> bool {
        if !self.running {
            return false;
        }
        let deadline = now + interval(settings, monitor);
        for m in self.members(monitor, connected, settings) {
            self.deadlines.insert(m, deadline);
        }
        true
    }

    /// The connected Monitors whose deadline has passed, each moved one
    /// interval past `now`. A due sync group comes back whole.
    pub fn take_due(
        &mut self,
        now: u64,
        connected: &[String],
        settings: &RotationSettings,
    ) -> Vec<String> {
        if !self.running {
            return Vec::new();
        }
        let mut due: Vec<String> = Vec::new();
        for monitor in connected {
            if due.contains(monitor) {
                continue;
            }
            if self.deadlines.get(monitor).is_some_and(|&d| d <= now) {
                for m in self.members(monitor, connected, settings) {
                    if !due.contains(&m) {
                        due.push(m);
                    }
                }
            }
        }
        for monitor in &due {
            self.deadlines
                .insert(monitor.clone(), now + interval(settings, monitor));
        }
        due
    }

    /// The earliest deadline among connected Monitors: when to wake.
    pub fn next_deadline(&self, connected: &[String]) -> Option<u64> {
        if !self.running {
            return None;
        }
        connected
            .iter()
            .filter_map(|m| self.deadlines.get(m))
            .min()
            .copied()
    }

    /// Milliseconds until `monitor` changes, if Rotation runs.
    pub fn remaining(&self, monitor: &str, now: u64) -> Option<u64> {
        if !self.running {
            return None;
        }
        self.deadlines.get(monitor).map(|&d| d.saturating_sub(now))
    }

    /// `monitor` plus, with sync, the rest of its connected sync group.
    fn members(
        &self,
        monitor: &str,
        connected: &[String],
        settings: &RotationSettings,
    ) -> Vec<String> {
        if !settings.in_sync_group(monitor) {
            return vec![monitor.to_owned()];
        }
        let mut members: Vec<String> = connected
            .iter()
            .filter(|m| settings.in_sync_group(m))
            .cloned()
            .collect();
        if !members.iter().any(|m| m == monitor) {
            members.push(monitor.to_owned());
        }
        members
    }

    fn group_deadline(
        &self,
        monitor: &str,
        connected: &[String],
        settings: &RotationSettings,
    ) -> Option<u64> {
        if !settings.in_sync_group(monitor) {
            return None;
        }
        connected
            .iter()
            .filter(|m| m.as_str() != monitor && settings.in_sync_group(m))
            .filter_map(|m| self.deadlines.get(m))
            .min()
            .copied()
    }
}

/// `monitor`'s interval in milliseconds, at least 1.
fn interval(settings: &RotationSettings, monitor: &str) -> u64 {
    settings.schedule_for(monitor).interval_ms.max(1)
}

/// The current wall-clock time in Unix milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_ipc::{Override, Schedule};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn settings(interval_ms: u64, sync: bool) -> RotationSettings {
        RotationSettings {
            schedule: Schedule {
                interval_ms,
                ..Schedule::default()
            },
            sync,
            overrides: vec![Override {
                monitor: "C".into(),
                schedule: Schedule {
                    interval_ms: 50,
                    ..Schedule::default()
                },
            }],
        }
    }

    #[test]
    fn a_stopped_rotation_never_wakes() {
        let mut r = Rotation::default();
        let all = names(&["A"]);
        assert_eq!(r.next_deadline(&all), None);
        assert!(r.take_due(u64::MAX, &all, &settings(10, false)).is_empty());
        assert_eq!(r.remaining("A", 0), None);
    }

    #[test]
    fn each_monitor_ticks_on_its_own_schedule() {
        let s = settings(100, false);
        let all = names(&["A", "B", "C"]);
        let mut r = Rotation::default();
        assert!(r.start(1000, &all, &s));
        assert_eq!(r.next_deadline(&all), Some(1050)); // C's Override
        assert_eq!(r.take_due(1050, &all, &s), ["C"]);
        assert_eq!(r.take_due(1100, &all, &s), ["A", "B", "C"]);
        assert_eq!(r.remaining("A", 1150), Some(50));
    }

    #[test]
    fn a_manual_apply_restarts_only_that_monitors_timer() {
        let s = settings(100, false);
        let all = names(&["A", "B"]);
        let mut r = Rotation::default();
        r.start(0, &all, &s);
        r.reset("A", 60, &all, &s);
        assert_eq!(r.take_due(100, &all, &s), ["B"]);
        assert_eq!(r.take_due(160, &all, &s), ["A"]);
    }

    #[test]
    fn a_sync_group_ticks_and_resets_together_but_overrides_do_not_join() {
        let s = settings(100, true);
        let all = names(&["A", "B", "C"]);
        let mut r = Rotation::default();
        r.start(0, &all, &s);
        r.reset("A", 60, &all, &s);
        assert_eq!(r.remaining("B", 60), Some(100));
        assert_eq!(r.take_due(50, &all, &s), ["C"]);
        assert_eq!(r.take_due(160, &all, &s), ["A", "B", "C"]);
        // A late-plugged Monitor joins its group's deadline.
        let more = names(&["A", "B", "C", "D"]);
        r.ensure(200, &more, &s);
        assert_eq!(r.deadlines["D"], r.deadlines["A"]);
    }

    #[test]
    fn a_shorter_interval_pulls_deadlines_in() {
        let all = names(&["A"]);
        let mut r = Rotation::default();
        r.start(0, &all, &settings(1_000_000, false));
        assert!(r.settings_changed(10, &settings(100, false)));
        assert_eq!(r.next_deadline(&all), Some(110));
        // A longer one leaves the current deadline alone.
        assert!(!r.settings_changed(20, &settings(5000, false)));
    }

    #[test]
    fn unplugged_monitors_do_not_wake_the_daemon() {
        let s = settings(100, false);
        let mut r = Rotation::default();
        r.start(0, &names(&["A", "B"]), &s);
        r.reset("A", 500, &names(&["A", "B"]), &s);
        assert_eq!(r.next_deadline(&names(&["A"])), Some(600));
    }

    #[test]
    fn starting_twice_keeps_the_running_schedule() {
        let s = settings(100, false);
        let all = names(&["A"]);
        let mut r = Rotation::default();
        r.start(0, &all, &s);
        assert!(!r.start(50, &all, &s));
        assert_eq!(r.next_deadline(&all), Some(100));
        assert!(r.stop());
        assert_eq!(r.next_deadline(&all), None);
    }
}
