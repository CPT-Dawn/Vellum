use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use tokio::sync::RwLock;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorLayout {
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_factor: u32,
}

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

pub(crate) fn detect_monitor_layouts() -> Result<Vec<MonitorLayout>> {
    if let Some(layouts) = detect_wayland_registry_layouts() {
        return Ok(layouts);
    }

    if let Some(layouts) = detect_hyprland_layouts() {
        return Ok(layouts);
    }

    if let Some(layouts) = detect_wlr_randr_layouts() {
        return Ok(layouts);
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
    width: Option<u32>,
    height: Option<u32>,
    scale_factor: Option<u32>,
}

fn detect_wayland_registry_layouts() -> Option<Vec<MonitorLayout>> {
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

    let mut layouts = Vec::new();
    for (id, metadata) in collector.outputs {
        layouts.push(extract_output_layout(id, &metadata));
    }

    if layouts.is_empty() {
        None
    } else {
        layouts.sort_by(|a, b| a.name.cmp(&b.name));
        Some(layouts)
    }
}

fn extract_output_layout(id: u32, metadata: &OutputMetadata) -> MonitorLayout {
    MonitorLayout {
        name: extract_output_name(id, metadata),
        width: metadata.width.unwrap_or(1920).max(1),
        height: metadata.height.unwrap_or(1080).max(1),
        scale_factor: metadata.scale_factor.unwrap_or(1).max(1),
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
            wl_output::Event::Mode { width, height, .. } => {
                if width > 0 && height > 0 {
                    entry.width = Some(width as u32);
                    entry.height = Some(height as u32);
                }
            }
            wl_output::Event::Scale { factor } => {
                if factor > 0 {
                    entry.scale_factor = Some(factor as u32);
                }
            }
            wl_output::Event::Name { name } => {
                entry.name = Some(name);
            }
            _ => {}
        }
    }
}

fn detect_hyprland_layouts() -> Option<Vec<MonitorLayout>> {
    let value = run_json_command("hyprctl", &["monitors", "-j"])?;
    let items = value.as_array()?;
    let mut layouts = Vec::new();

    for item in items {
        let name = item.get("name").and_then(Value::as_str)?.to_string();
        let width = item
            .get("width")
            .and_then(Value::as_u64)
            .unwrap_or(1920)
            .max(1) as u32;
        let height = item
            .get("height")
            .and_then(Value::as_u64)
            .unwrap_or(1080)
            .max(1) as u32;
        let scale_factor = item
            .get("scale")
            .and_then(Value::as_f64)
            .map(|v| v.max(1.0).round() as u32)
            .unwrap_or(1);

        layouts.push(MonitorLayout {
            name,
            width,
            height,
            scale_factor,
        });
    }

    if layouts.is_empty() {
        None
    } else {
        layouts.sort_by(|a, b| a.name.cmp(&b.name));
        Some(layouts)
    }
}

fn detect_wlr_randr_layouts() -> Option<Vec<MonitorLayout>> {
    let value = run_json_command("wlr-randr", &["--json"])?;
    let items = value.as_array()?;
    let mut layouts = Vec::new();

    for item in items {
        let name = item
            .get("name")
            .or_else(|| item.get("output"))
            .and_then(Value::as_str)?
            .to_string();

        let width = item
            .get("width")
            .and_then(Value::as_u64)
            .or_else(|| {
                item.get("current_mode")
                    .and_then(|mode| mode.get("width"))
                    .and_then(Value::as_u64)
            })
            .unwrap_or(1920)
            .max(1) as u32;
        let height = item
            .get("height")
            .and_then(Value::as_u64)
            .or_else(|| {
                item.get("current_mode")
                    .and_then(|mode| mode.get("height"))
                    .and_then(Value::as_u64)
            })
            .unwrap_or(1080)
            .max(1) as u32;
        let scale_factor = item
            .get("scale")
            .and_then(Value::as_f64)
            .map(|v| v.max(1.0).round() as u32)
            .unwrap_or(1);

        layouts.push(MonitorLayout {
            name,
            width,
            height,
            scale_factor,
        });
    }

    if layouts.is_empty() {
        None
    } else {
        layouts.sort_by(|a, b| a.name.cmp(&b.name));
        Some(layouts)
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
    use super::{
        extract_output_layout, extract_output_name, normalize_monitor_snapshot, MonitorLayout,
        MonitorSnapshot, OutputMetadata,
    };

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
            ..Default::default()
        };
        assert_eq!(extract_output_name(4, &named), "DP-1".to_string());

        let make_model = OutputMetadata {
            name: Some(" ".to_string()),
            make: Some("Dell".to_string()),
            model: Some("U2720Q".to_string()),
            ..Default::default()
        };
        assert_eq!(
            extract_output_name(5, &make_model),
            "Dell U2720Q".to_string()
        );

        let fallback = OutputMetadata::default();
        assert_eq!(extract_output_name(9, &fallback), "wl-output-9".to_string());
    }

    #[test]
    fn extract_output_layout_applies_dimension_defaults() {
        let unknown = OutputMetadata::default();
        let fallback = extract_output_layout(4, &unknown);
        assert_eq!(
            fallback,
            MonitorLayout {
                name: "wl-output-4".to_string(),
                width: 1920,
                height: 1080,
                scale_factor: 1,
            }
        );

        let exact = OutputMetadata {
            name: Some("DP-1".to_string()),
            make: None,
            model: None,
            width: Some(3440),
            height: Some(1440),
            scale_factor: Some(2),
        };
        let layout = extract_output_layout(5, &exact);
        assert_eq!(layout.name, "DP-1");
        assert_eq!(layout.width, 3440);
        assert_eq!(layout.height, 1440);
        assert_eq!(layout.scale_factor, 2);
    }
}
