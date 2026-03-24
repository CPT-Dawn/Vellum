use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage, imageops};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as NucleoConfig, Matcher as NucleoMatcher, Utf32Str};
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Library,
    Monitor,
    Playback,
}

impl Panel {
    pub fn next(self) -> Self {
        match self {
            Self::Library => Self::Monitor,
            Self::Monitor => Self::Playback,
            Self::Playback => Self::Library,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Library => Self::Playback,
            Self::Monitor => Self::Library,
            Self::Playback => Self::Monitor,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Monitor => "Monitor",
            Self::Playback => "Playback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleMode {
    Fit,
    Fill,
    Stretch,
}

impl ScaleMode {
    pub fn next(self) -> Self {
        match self {
            Self::Fit => Self::Fill,
            Self::Fill => Self::Stretch,
            Self::Stretch => Self::Fit,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fit => "Fit",
            Self::Fill => "Fill",
            Self::Stretch => "Stretch",
        }
    }

    pub fn as_resize(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::Stretch => "stretch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl Rotation {
    pub fn next(self) -> Self {
        match self {
            Self::Deg0 => Self::Deg90,
            Self::Deg90 => Self::Deg180,
            Self::Deg180 => Self::Deg270,
            Self::Deg270 => Self::Deg0,
        }
    }

    pub fn degrees(self) -> u16 {
        match self {
            Self::Deg0 => 0,
            Self::Deg90 => 90,
            Self::Deg180 => 180,
            Self::Deg270 => 270,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum NotificationLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub text: String,
    pub level: NotificationLevel,
    pub created_at: std::time::Instant,
}

#[derive(Debug, Clone)]
pub struct BrowserEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_parent: bool,
}

#[derive(Debug, Clone)]
pub struct ImagePreview {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct MonitorEntry {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub focused: bool,
}

#[derive(Debug, Clone)]
pub struct TransitionState {
    pub duration_ms: u32,
    pub fps: u16,
    pub easing_idx: usize,
    pub effect_idx: usize,
    pub selected_field: usize,
}

impl Default for TransitionState {
    fn default() -> Self {
        Self {
            duration_ms: 700,
            fps: 60,
            easing_idx: 3,
            effect_idx: 1,
            selected_field: 0,
        }
    }
}

impl TransitionState {
    pub fn change_selected(&mut self, step: i32) {
        match self.selected_field {
            0 => {
                let next = self.duration_ms as i32 + step * 25;
                self.duration_ms = next.clamp(50, 20_000) as u32;
            }
            1 => {
                let next = self.fps as i32 + step * 5;
                self.fps = next.clamp(10, 240) as u16;
            }
            2 => {
                let len = EASING_PRESETS.len() as i32;
                let next = (self.easing_idx as i32 + step).rem_euclid(len);
                self.easing_idx = next as usize;
            }
            3 => {
                let len = TRANSITION_EFFECTS.len() as i32;
                let next = (self.effect_idx as i32 + step).rem_euclid(len);
                self.effect_idx = next as usize;
            }
            _ => {}
        }
    }
}

pub const EASING_PRESETS: [&str; 4] = ["linear", "ease-in", "ease-out", "ease-in-out"];
pub const TRANSITION_EFFECTS: [&str; 4] = ["simple", "fade", "wipe", "grow"];

#[derive(Debug, Clone, Copy)]
pub struct AspectSimulation {
    pub target_width: u32,
    pub target_height: u32,
    pub bars_x: u32,
    pub bars_y: u32,
    pub crop_x: u32,
    pub crop_y: u32,
}

pub fn preferred_initial_browser_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let pictures = PathBuf::from(home).join("Pictures");
        if pictures.is_dir() {
            return pictures;
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn is_supported_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp"
            )
        })
        .unwrap_or(false)
}

pub fn load_browser_entries(dir: &Path) -> Result<Vec<BrowserEntry>> {
    let mut entries = Vec::new();

    if let Some(parent) = dir.parent() {
        entries.push(BrowserEntry {
            name: String::from(".."),
            path: parent.to_path_buf(),
            is_dir: true,
            is_parent: true,
        });
    }

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in fs::read_dir(dir).with_context(|| format!("cannot read '{}'", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata()?;

        if meta.is_dir() {
            dirs.push(BrowserEntry {
                name: format!("{file_name}/"),
                path,
                is_dir: true,
                is_parent: false,
            });
            continue;
        }

        if meta.is_file() && is_supported_image_path(&path) {
            files.push(BrowserEntry {
                name: file_name,
                path,
                is_dir: false,
                is_parent: false,
            });
        }
    }

    dirs.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());
    files.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());

    entries.extend(dirs);
    entries.extend(files);

    Ok(entries)
}

pub fn fuzzy_filter(entries: &[BrowserEntry], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }

    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = NucleoMatcher::new(NucleoConfig::DEFAULT.match_paths());
    let mut utf32_buf = Vec::new();

    let mut scored = entries
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            pattern
                .score(
                    Utf32Str::new(entry.name.as_str(), &mut utf32_buf),
                    &mut matcher,
                )
                .map(|score| (score, idx))
        })
        .collect::<Vec<_>>();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, idx)| idx).collect()
}

#[derive(Debug, Deserialize)]
struct HyprMonitor {
    name: String,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    #[serde(default)]
    focused: bool,
}

pub async fn discover_monitors() -> Result<Vec<MonitorEntry>> {
    match probe_hyprctl_monitors().await {
        Ok(monitors) if !monitors.is_empty() => return Ok(monitors),
        Ok(_) => {}
        Err(_) => {}
    }

    probe_wlr_randr_monitors().await
}

async fn probe_hyprctl_monitors() -> Result<Vec<MonitorEntry>> {
    let json = command_json("hyprctl", &["monitors", "-j"]).await?;
    let monitors: Vec<HyprMonitor> =
        serde_json::from_value(json).context("invalid hyprctl monitors JSON")?;

    Ok(monitors
        .into_iter()
        .map(|m| MonitorEntry {
            name: m.name,
            width: m.width,
            height: m.height,
            x: m.x,
            y: m.y,
            focused: m.focused,
        })
        .collect())
}

async fn probe_wlr_randr_monitors() -> Result<Vec<MonitorEntry>> {
    let json = command_json("wlr-randr", &["--json"]).await?;
    parse_wlr_randr_monitors(json)
}

async fn command_json(binary: &str, args: &[&str]) -> Result<Value> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to execute {binary}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{binary} returned {}: {}", output.status, stderr.trim());
    }

    serde_json::from_slice(&output.stdout).with_context(|| format!("invalid JSON from {binary}"))
}

fn parse_wlr_randr_monitors(payload: Value) -> Result<Vec<MonitorEntry>> {
    let Value::Array(outputs) = payload else {
        bail!("wlr-randr JSON root must be an array");
    };

    let monitors = outputs
        .into_iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let name = obj.get("name")?.as_str()?.to_owned();
            let (width, height) = extract_wlr_dimensions(obj)?;
            let x = obj
                .get("x")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or(0);
            let y = obj
                .get("y")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or(0);

            Some(MonitorEntry {
                name,
                width,
                height,
                x,
                y,
                focused: false,
            })
        })
        .collect::<Vec<_>>();

    Ok(monitors)
}

fn extract_wlr_dimensions(object: &serde_json::Map<String, Value>) -> Option<(u32, u32)> {
    if let Some(mode) = object.get("current_mode").and_then(Value::as_object) {
        let w = mode
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())?;
        let h = mode
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())?;
        return Some((w, h));
    }

    object
        .get("modes")
        .and_then(Value::as_array)
        .and_then(|modes| {
            modes
                .iter()
                .find(|mode| mode.get("current").and_then(Value::as_bool) == Some(true))
        })
        .and_then(Value::as_object)
        .and_then(|mode| {
            let w = mode
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())?;
            let h = mode
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())?;
            Some((w, h))
        })
}

pub fn apply_rotation(image: DynamicImage, rotation: Rotation) -> DynamicImage {
    match rotation {
        Rotation::Deg0 => image,
        Rotation::Deg90 => image.rotate90(),
        Rotation::Deg180 => image.rotate180(),
        Rotation::Deg270 => image.rotate270(),
    }
}

pub fn render_to_monitor_canvas(
    image: DynamicImage,
    monitor_dim: (u32, u32),
    scale_mode: ScaleMode,
) -> DynamicImage {
    let (mw, mh) = (monitor_dim.0.max(1), monitor_dim.1.max(1));
    let (iw, ih) = image.dimensions();
    let (iw, ih) = (iw.max(1), ih.max(1));

    match scale_mode {
        ScaleMode::Stretch => image.resize_exact(mw, mh, imageops::FilterType::Lanczos3),
        ScaleMode::Fill => {
            let scale = (mw as f32 / iw as f32).max(mh as f32 / ih as f32);
            let nw = ((iw as f32 * scale).round() as u32).max(1);
            let nh = ((ih as f32 * scale).round() as u32).max(1);
            let resized = image.resize_exact(nw, nh, imageops::FilterType::Lanczos3);
            let x = nw.saturating_sub(mw) / 2;
            let y = nh.saturating_sub(mh) / 2;
            resized.crop_imm(x, y, mw, mh)
        }
        ScaleMode::Fit => {
            let scale = (mw as f32 / iw as f32).min(mh as f32 / ih as f32);
            let nw = ((iw as f32 * scale).round() as u32).max(1);
            let nh = ((ih as f32 * scale).round() as u32).max(1);
            let resized = image.resize_exact(nw, nh, imageops::FilterType::Lanczos3);
            let mut canvas: RgbaImage = ImageBuffer::from_pixel(mw, mh, Rgba([0, 0, 0, 255]));
            let x = i64::from(mw.saturating_sub(nw) / 2);
            let y = i64::from(mh.saturating_sub(nh) / 2);
            imageops::overlay(&mut canvas, &resized.to_rgba8(), x, y);
            DynamicImage::ImageRgba8(canvas)
        }
    }
}

pub fn simulate_aspect(
    image: (u32, u32),
    monitor: (u32, u32),
    scale_mode: ScaleMode,
    rotation: Rotation,
) -> AspectSimulation {
    let (mut iw, mut ih) = (image.0.max(1), image.1.max(1));
    if matches!(rotation, Rotation::Deg90 | Rotation::Deg270) {
        std::mem::swap(&mut iw, &mut ih);
    }

    let (iw, ih) = (iw as f64, ih as f64);
    let (mw, mh) = (monitor.0.max(1) as f64, monitor.1.max(1) as f64);

    let scale = match scale_mode {
        ScaleMode::Fit => (mw / iw).min(mh / ih),
        ScaleMode::Fill => (mw / iw).max(mh / ih),
        ScaleMode::Stretch => 0.0,
    };

    let (target_width, target_height) = if matches!(scale_mode, ScaleMode::Stretch) {
        (monitor.0.max(1), monitor.1.max(1))
    } else {
        (
            (iw * scale).round().max(1.0) as u32,
            (ih * scale).round().max(1.0) as u32,
        )
    };

    AspectSimulation {
        target_width,
        target_height,
        bars_x: monitor.0.saturating_sub(target_width) / 2,
        bars_y: monitor.1.saturating_sub(target_height) / 2,
        crop_x: target_width.saturating_sub(monitor.0) / 2,
        crop_y: target_height.saturating_sub(monitor.1) / 2,
    }
}

pub fn make_monitor_layout_ascii(
    monitors: &[MonitorEntry],
    selected: usize,
    cols: usize,
    rows: usize,
) -> Vec<String> {
    if monitors.is_empty() || cols < 4 || rows < 3 {
        return vec![String::from("No monitor data")];
    }

    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    let max_x = monitors
        .iter()
        .map(|m| m.x + i32::try_from(m.width).unwrap_or(i32::MAX / 4))
        .max()
        .unwrap_or(1);
    let max_y = monitors
        .iter()
        .map(|m| m.y + i32::try_from(m.height).unwrap_or(i32::MAX / 4))
        .max()
        .unwrap_or(1);

    let span_x = (max_x - min_x).max(1) as f32;
    let span_y = (max_y - min_y).max(1) as f32;

    let mut grid = vec![vec![' '; cols]; rows];

    for (idx, mon) in monitors.iter().enumerate() {
        let x0 = (((mon.x - min_x) as f32 / span_x) * (cols as f32 - 1.0)).round() as usize;
        let y0 = (((mon.y - min_y) as f32 / span_y) * (rows as f32 - 1.0)).round() as usize;
        let x1 = ((((mon.x - min_x) as f32 + mon.width as f32) / span_x) * (cols as f32 - 1.0))
            .round() as usize;
        let y1 = ((((mon.y - min_y) as f32 + mon.height as f32) / span_y) * (rows as f32 - 1.0))
            .round() as usize;

        let fill = if idx == selected { '▓' } else { '▒' };

        for y in y0.min(rows - 1)..=y1.min(rows - 1) {
            for x in x0.min(cols - 1)..=x1.min(cols - 1) {
                let is_border = y == y0.min(rows - 1)
                    || y == y1.min(rows - 1)
                    || x == x0.min(cols - 1)
                    || x == x1.min(cols - 1);
                grid[y][x] = if is_border { '█' } else { fill };
            }
        }
    }

    grid.into_iter()
        .map(|line| line.into_iter().collect::<String>())
        .collect()
}
