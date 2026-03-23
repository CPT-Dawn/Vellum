//! Monitor discovery and parsing backends for Wayland compositors.

use serde::Deserialize;
use tokio::process::Command;

use crate::backend::BackendError;

/// Display monitor metadata used by the TUI layout and wallpaper assignment logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Stable monitor identifier (for example `DP-1` or `eDP-1`).
    pub name: String,
    /// Top-left X coordinate in compositor layout space.
    pub x: i32,
    /// Top-left Y coordinate in compositor layout space.
    pub y: i32,
    /// Pixel width of the monitor.
    pub width: u32,
    /// Pixel height of the monitor.
    pub height: u32,
    /// Refresh rate in millihertz when reported by backend.
    pub refresh_millihz: Option<u32>,
}

/// Fetches monitor data using `hyprctl -j monitors` or `wlr-randr` fallback.
pub async fn query_monitors() -> Result<Vec<MonitorInfo>, BackendError> {
    if let Ok(monitors) = query_hyprctl_monitors().await {
        return Ok(monitors);
    }

    if let Ok(monitors) = query_wlr_randr_monitors().await {
        return Ok(monitors);
    }

    Err(BackendError::MonitorBackendUnavailable)
}

/// Executes `hyprctl -j monitors` and parses the JSON response.
pub async fn query_hyprctl_monitors() -> Result<Vec<MonitorInfo>, BackendError> {
    let output = Command::new("hyprctl")
        .args(["-j", "monitors"])
        .output()
        .await?;

    if !output.status.success() {
        return Err(BackendError::CommandFailed {
            program: "hyprctl",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    parse_hyprctl_monitors_json(&output.stdout)
}

/// Executes `wlr-randr` and parses the plain-text response.
pub async fn query_wlr_randr_monitors() -> Result<Vec<MonitorInfo>, BackendError> {
    let output = Command::new("wlr-randr").output().await?;

    if !output.status.success() {
        return Err(BackendError::CommandFailed {
            program: "wlr-randr",
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    parse_wlr_randr_output(&output.stdout)
}

/// Parses hyprctl JSON monitor payload into normalized monitor data.
pub fn parse_hyprctl_monitors_json(raw: &[u8]) -> Result<Vec<MonitorInfo>, BackendError> {
    let parsed: Vec<HyprctlMonitor> = serde_json::from_slice(raw)?;

    let mut monitors = Vec::with_capacity(parsed.len());
    for monitor in parsed {
        let refresh_millihz = monitor.refresh_rate.and_then(refresh_rate_to_millihz);

        monitors.push(MonitorInfo {
            name: monitor.name,
            x: monitor.x,
            y: monitor.y,
            width: monitor.width,
            height: monitor.height,
            refresh_millihz,
        });
    }

    Ok(monitors)
}

/// Parses wlr-randr text output into normalized monitor data.
pub fn parse_wlr_randr_output(raw: &[u8]) -> Result<Vec<MonitorInfo>, BackendError> {
    let text = std::str::from_utf8(raw).map_err(|_| BackendError::ParseError("utf8 decode"))?;

    let mut monitors = Vec::new();
    let mut active_monitor: Option<MonitorInfo> = None;

    for line in text.lines() {
        if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
            if let Some(current) = active_monitor.as_mut() {
                parse_wlr_detail_line(current, line);
            }
            continue;
        }

        if let Some(current) = active_monitor.take() {
            if current.width > 0 && current.height > 0 {
                monitors.push(current);
            }
        }

        active_monitor = Some(MonitorInfo {
            name: line.trim().to_owned(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            refresh_millihz: None,
        });
    }

    if let Some(current) = active_monitor {
        if current.width > 0 && current.height > 0 {
            monitors.push(current);
        }
    }

    if monitors.is_empty() {
        return Err(BackendError::ParseError("no monitors found"));
    }

    Ok(monitors)
}

/// Parses indented per-monitor detail lines from `wlr-randr`.
fn parse_wlr_detail_line(monitor: &mut MonitorInfo, line: &str) {
    let trimmed = line.trim_start();

    if let Some((x, y)) = parse_position(trimmed) {
        monitor.x = x;
        monitor.y = y;
        return;
    }

    if let Some((width, height, refresh_millihz)) = parse_current_mode(trimmed) {
        monitor.width = width;
        monitor.height = height;
        monitor.refresh_millihz = Some(refresh_millihz);
    }
}

/// Parses a `Position: X,Y` entry.
fn parse_position(line: &str) -> Option<(i32, i32)> {
    let value = line.strip_prefix("Position:")?.trim();
    let (x_raw, y_raw) = value.split_once(',')?;
    let x = x_raw.trim().parse::<i32>().ok()?;
    let y = y_raw.trim().parse::<i32>().ok()?;
    Some((x, y))
}

/// Parses a `current WIDTHxHEIGHT @ REFRESH Hz` entry.
fn parse_current_mode(line: &str) -> Option<(u32, u32, u32)> {
    if !line.starts_with("current ") {
        return None;
    }

    let mut parts = line.split_ascii_whitespace();
    let _current = parts.next()?;
    let resolution = parts.next()?;
    let _at = parts.next()?;
    let hz_raw = parts.next()?;

    let (width_raw, height_raw) = resolution.split_once('x')?;
    let width = width_raw.parse::<u32>().ok()?;
    let height = height_raw.parse::<u32>().ok()?;

    let hz = hz_raw.parse::<f64>().ok()?;
    let refresh_millihz = refresh_rate_to_millihz(hz)?;

    Some((width, height, refresh_millihz))
}

/// Converts a floating-point hertz value to rounded millihertz.
fn refresh_rate_to_millihz(value_hz: f64) -> Option<u32> {
    if !value_hz.is_finite() || value_hz.is_sign_negative() {
        return None;
    }

    let milli = (value_hz * 1000.0).round();
    if milli > f64::from(u32::MAX) {
        return None;
    }

    Some(milli as u32)
}

/// Shape of `hyprctl -j monitors` entries.
#[derive(Debug, Deserialize)]
struct HyprctlMonitor {
    /// Monitor name.
    name: String,
    /// Layout X coordinate.
    x: i32,
    /// Layout Y coordinate.
    y: i32,
    /// Pixel width.
    width: u32,
    /// Pixel height.
    height: u32,
    /// Refresh rate in hertz.
    #[serde(rename = "refreshRate")]
    refresh_rate: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::{parse_hyprctl_monitors_json, parse_wlr_randr_output};

    /// Validates hyprctl JSON monitor parsing.
    #[test]
    fn parse_hyprctl_payload() {
        let raw = br#"[
            {
                "name": "DP-1",
                "x": 1920,
                "y": 0,
                "width": 2560,
                "height": 1440,
                "refreshRate": 143.998
            },
            {
                "name": "eDP-1",
                "x": 0,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "refreshRate": 60.0
            }
        ]"#;

        let monitors = parse_hyprctl_monitors_json(raw).expect("must parse");
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].name, "DP-1");
        assert_eq!(monitors[0].width, 2560);
        assert_eq!(monitors[0].refresh_millihz, Some(143_998));
    }

    /// Validates wlr-randr text monitor parsing.
    #[test]
    fn parse_wlr_randr_payload() {
        let raw = b"DP-1\n  current 2560x1440 @ 143.997 Hz\n  Position: 1920,0\neDP-1\n  current 1920x1080 @ 60.000 Hz\n  Position: 0,0\n";

        let monitors = parse_wlr_randr_output(raw).expect("must parse");
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[1].name, "eDP-1");
        assert_eq!(monitors[1].x, 0);
        assert_eq!(monitors[1].y, 0);
        assert_eq!(monitors[1].refresh_millihz, Some(60_000));
    }
}
