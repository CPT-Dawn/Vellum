use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use tokio::sync::RwLock;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

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
    if let Some(monitors) = detect_wayland_registry_monitors() {
        return Ok(monitors);
    }

    if let Some(monitors) = detect_hyprland_monitors() {
        return Ok(monitors);
    }

    if let Some(monitors) = detect_wlr_randr_monitors() {
        return Ok(monitors);
    }

    anyhow::bail!("unable to detect monitors from wayland registry, hyprctl, or wlr-randr")
}

#[derive(Default)]
struct WaylandOutputCollector {
    outputs: BTreeMap<u32, OutputMetadata>,
}

#[derive(Default)]
struct OutputMetadata {
    name: Option<String>,
    make: Option<String>,
    model: Option<String>,
}

fn detect_wayland_registry_monitors() -> Option<Vec<String>> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut event_queue) =
        registry_queue_init::<WaylandOutputCollector>(&connection).ok()?;
    let queue_handle = event_queue.handle();

    let mut collector = WaylandOutputCollector::default();
    globals.contents().with_list(|list| {
        for global in list {
            if global.interface == wl_output::WlOutput::interface().name {
                let version = global.version.min(4);
                let _output: wl_output::WlOutput =
                    globals
                        .registry()
                        .bind(global.name, version, &queue_handle, global.name);
            }
        }
    });

    event_queue.roundtrip(&mut collector).ok()?;
    event_queue.roundtrip(&mut collector).ok()?;

    let mut names = Vec::new();
    for (id, metadata) in collector.outputs {
        names.push(extract_output_name(id, &metadata));
    }

    if names.is_empty() {
        None
    } else {
        Some(normalize_monitor_snapshot(names))
    }
}

fn extract_output_name(id: u32, metadata: &OutputMetadata) -> String {
    if let Some(name) = metadata.name.as_deref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    let make = metadata.make.as_deref().unwrap_or("").trim();
    let model = metadata.model.as_deref().unwrap_or("").trim();

    let combined = format!("{} {}", make, model).trim().to_string();
    if !combined.is_empty() {
        return combined;
    }

    format!("wl-output-{id}")
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandOutputCollector {
    fn event(
        state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::GlobalRemove { name } = event {
            state.outputs.remove(&name);
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for WaylandOutputCollector {
    fn event(
        state: &mut Self,
        _proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        data: &u32,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        let entry = state.outputs.entry(*data).or_default();
        match event {
            wl_output::Event::Geometry { make, model, .. } => {
                entry.make = Some(make);
                entry.model = Some(model);
            }
            wl_output::Event::Name { name } => {
                entry.name = Some(name);
            }
            _ => {}
        }
    }
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
    use super::{extract_output_name, normalize_monitor_snapshot, MonitorSnapshot, OutputMetadata};

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

    #[test]
    fn extract_output_name_prefers_protocol_name_then_make_model() {
        let named = OutputMetadata {
            name: Some("DP-1".to_string()),
            make: Some("Dell".to_string()),
            model: Some("U2720Q".to_string()),
        };
        assert_eq!(extract_output_name(4, &named), "DP-1".to_string());

        let make_model = OutputMetadata {
            name: Some(" ".to_string()),
            make: Some("Dell".to_string()),
            model: Some("U2720Q".to_string()),
        };
        assert_eq!(
            extract_output_name(5, &make_model),
            "Dell U2720Q".to_string()
        );

        let fallback = OutputMetadata::default();
        assert_eq!(extract_output_name(9, &fallback), "wl-output-9".to_string());
    }
}
