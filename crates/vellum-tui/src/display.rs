use ratatui::layout::Rect;
use serde_json::Value;
use std::process::Command as ProcessCommand;

#[derive(Clone)]
pub(crate) struct MonitorProfile {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) source: String,
}

impl MonitorProfile {
    pub(crate) fn resolve(width: Option<u32>, height: Option<u32>) -> Self {
        if let (Some(width), Some(height)) = (width, height) {
            if width > 0 && height > 0 {
                return Self {
                    width,
                    height,
                    source: "cli override".to_string(),
                };
            }
        }

        if let Some(profile) = detect_hyprland_monitor() {
            return profile;
        }

        if let Some(profile) = detect_wlr_randr_monitor() {
            return profile;
        }

        if let Ok((cols, rows)) = crossterm::terminal::size() {
            if cols > 0 && rows > 0 {
                return Self {
                    width: u32::from(cols),
                    height: u32::from(rows),
                    source: "terminal size fallback".to_string(),
                };
            }
        }

        Self {
            width: 1920,
            height: 1080,
            source: "default 1080p fallback".to_string(),
        }
    }

    pub(crate) fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

pub(crate) fn fit_aspect_rect(area: Rect, target_width: u32, target_height: u32) -> Rect {
    if area.width < 3 || area.height < 3 || target_width == 0 || target_height == 0 {
        return area;
    }

    let area_w = u32::from(area.width);
    let area_h = u32::from(area.height);

    let (width, height) =
        if area_w.saturating_mul(target_height) > area_h.saturating_mul(target_width) {
            let width = area_h.saturating_mul(target_width) / target_height;
            (width.max(1), area_h)
        } else {
            let height = area_w.saturating_mul(target_height) / target_width;
            (area_w, height.max(1))
        };

    let width_u16 = u16::try_from(width).unwrap_or(area.width);
    let height_u16 = u16::try_from(height).unwrap_or(area.height);
    let x = area.x + area.width.saturating_sub(width_u16) / 2;
    let y = area.y + area.height.saturating_sub(height_u16) / 2;

    Rect::new(x, y, width_u16, height_u16)
}

fn run_json_command(command: &str, args: &[&str]) -> Option<Value> {
    let output = ProcessCommand::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    serde_json::from_str::<Value>(&stdout).ok()
}

fn detect_hyprland_monitor() -> Option<MonitorProfile> {
    let value = run_json_command("hyprctl", &["monitors", "-j"])?;
    let monitors = value.as_array()?;

    let selected = monitors
        .iter()
        .find(|monitor| monitor.get("focused").and_then(Value::as_bool) == Some(true))
        .or_else(|| monitors.first())?;

    let width = selected.get("width").and_then(Value::as_u64)? as u32;
    let height = selected.get("height").and_then(Value::as_u64)? as u32;
    if width == 0 || height == 0 {
        return None;
    }

    let name = selected
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("hyprland")
        .to_string();

    Some(MonitorProfile {
        width,
        height,
        source: format!("hyprctl:{name}"),
    })
}

fn detect_wlr_randr_monitor() -> Option<MonitorProfile> {
    let value = run_json_command("wlr-randr", &["--json"])?;
    let (width, height) = find_resolution_pair(&value)?;
    if width == 0 || height == 0 {
        return None;
    }

    Some(MonitorProfile {
        width,
        height,
        source: "wlr-randr".to_string(),
    })
}

fn find_resolution_pair(value: &Value) -> Option<(u32, u32)> {
    match value {
        Value::Object(map) => {
            if let (Some(width), Some(height)) = (map.get("width"), map.get("height")) {
                let width = width.as_u64()? as u32;
                let height = height.as_u64()? as u32;
                if width > 0 && height > 0 {
                    return Some((width, height));
                }
            }

            for child in map.values() {
                if let Some(pair) = find_resolution_pair(child) {
                    return Some(pair);
                }
            }
            None
        }
        Value::Array(values) => {
            for child in values {
                if let Some(pair) = find_resolution_pair(child) {
                    return Some(pair);
                }
            }
            None
        }
        _ => None,
    }
}
