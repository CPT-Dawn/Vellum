use anyhow::Result;
use serde_json::Value;
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub(crate) struct MonitorSnapshot {
    names: Arc<RwLock<Vec<String>>>,
}

impl MonitorSnapshot {
    pub(crate) async fn get(&self) -> Vec<String> {
        self.names.read().await.clone()
    }

    pub(crate) async fn replace_if_changed(&self, next: Vec<String>) -> bool {
        let mut guard = self.names.write().await;
        if *guard == next {
            return false;
        }

        *guard = next;
        true
    }
}

pub(crate) fn normalize_monitor_snapshot(mut monitors: Vec<String>) -> Vec<String> {
    monitors.retain(|name| !name.is_empty());
    monitors.sort();
    monitors.dedup();
    monitors
}

pub(crate) fn detect_monitor_names() -> Result<Vec<String>> {
    if let Some(monitors) = detect_hyprland_monitors() {
        return Ok(monitors);
    }

    if let Some(monitors) = detect_wlr_randr_monitors() {
        return Ok(monitors);
    }

    anyhow::bail!("unable to detect monitors from hyprctl or wlr-randr")
}

fn detect_hyprland_monitors() -> Option<Vec<String>> {
    let value = run_json_command("hyprctl", &["monitors", "-j"])?;
    let items = value.as_array()?;
    let mut names = Vec::new();
    for item in items {
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            names.push(name.to_string());
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn detect_wlr_randr_monitors() -> Option<Vec<String>> {
    let value = run_json_command("wlr-randr", &["--json"])?;
    let items = value.as_array()?;
    let mut names = Vec::new();
    for item in items {
        if let Some(name) = item
            .get("name")
            .or_else(|| item.get("output"))
            .and_then(Value::as_str)
        {
            names.push(name.to_string());
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn run_json_command(command: &str, args: &[&str]) -> Option<Value> {
    let output = ProcessCommand::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    serde_json::from_str::<Value>(&stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::{normalize_monitor_snapshot, MonitorSnapshot};

    #[tokio::test]
    async fn snapshot_replace_if_changed_tracks_deltas() {
        let snapshot = MonitorSnapshot::default();
        assert!(snapshot.replace_if_changed(vec!["DP-1".to_string()]).await);
        assert!(!snapshot.replace_if_changed(vec!["DP-1".to_string()]).await);
        assert!(
            snapshot
                .replace_if_changed(vec!["DP-1".to_string(), "HDMI-A-1".to_string()])
                .await
        );
    }

    #[test]
    fn normalize_snapshot_sorts_and_dedups() {
        let normalized = normalize_monitor_snapshot(vec![
            "HDMI-A-1".to_string(),
            "".to_string(),
            "DP-1".to_string(),
            "DP-1".to_string(),
        ]);

        assert_eq!(normalized, vec!["DP-1".to_string(), "HDMI-A-1".to_string()]);
    }
}
