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
pub enum FocusRegion {
    Library,
    Preview,
    Playlist,
    Transitions,
}

impl FocusRegion {
    pub fn next(self) -> Self {
        match self {
            Self::Library => Self::Preview,
            Self::Preview => Self::Playlist,
            Self::Playlist => Self::Transitions,
            Self::Transitions => Self::Library,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Library => Self::Transitions,
            Self::Preview => Self::Library,
            Self::Playlist => Self::Transitions,
            Self::Transitions => Self::Playlist,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Preview => "Preview",
            Self::Playlist => "Playlist",
            Self::Transitions => "Transitions",
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
    Center,
}

impl ScaleMode {
    pub fn next(self) -> Self {
        match self {
            Self::Fit => Self::Fill,
            Self::Fill => Self::Stretch,
            Self::Stretch => Self::Center,
            Self::Center => Self::Fit,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Fit => Self::Center,
            Self::Fill => Self::Fit,
            Self::Stretch => Self::Fill,
            Self::Center => Self::Stretch,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Scale",
            Self::Fill => "Fill",
            Self::Stretch => "Stretch",
            Self::Center => "Center",
        }
    }

    pub fn backend_resize(self) -> &'static str {
        match self {
            Self::Fit | Self::Center => "fit",
            Self::Fill => "fill",
            Self::Stretch => "stretch",
        }
    }

    pub fn is_staged(self) -> bool {
        matches!(self, Self::Center)
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

    pub fn prev(self) -> Self {
        match self {
            Self::Deg0 => Self::Deg270,
            Self::Deg90 => Self::Deg0,
            Self::Deg180 => Self::Deg90,
            Self::Deg270 => Self::Deg180,
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Deg0 => "0deg",
            Self::Deg90 => "90deg",
            Self::Deg180 => "180deg",
            Self::Deg270 => "270deg",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserEntryKind {
    Parent,
    Directory,
    Image,
}

#[derive(Debug, Clone)]
pub struct BrowserEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: BrowserEntryKind,
}

impl BrowserEntry {
    pub fn is_dir(&self) -> bool {
        matches!(
            self.kind,
            BrowserEntryKind::Parent | BrowserEntryKind::Directory
        )
    }
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
                self.easing_idx = (self.easing_idx as i32 + step).rem_euclid(len) as usize;
            }
            3 => {
                let len = TRANSITION_EFFECTS.len() as i32;
                self.effect_idx = (self.effect_idx as i32 + step).rem_euclid(len) as usize;
            }
            _ => {}
        }
    }

    pub fn field_name(field_idx: usize) -> &'static str {
        match field_idx {
            0 => "duration",
            1 => "fps",
            2 => "easing",
            3 => "effect",
            _ => "unknown",
        }
    }

    pub fn field_value(&self, field_idx: usize) -> String {
        match field_idx {
            0 => format!("{} ms", self.duration_ms),
            1 => self.fps.to_string(),
            2 => EASING_PRESETS[self.easing_idx].to_string(),
            3 => TRANSITION_EFFECTS[self.effect_idx].to_string(),
            _ => String::new(),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "{}ms | {} fps | {} | {}",
            self.duration_ms,
            self.fps,
            EASING_PRESETS[self.easing_idx],
            TRANSITION_EFFECTS[self.effect_idx]
        )
    }
}

pub const EASING_PRESETS: [&str; 4] = ["linear", "ease-in", "ease-out", "ease-in-out"];
pub const TRANSITION_EFFECTS: [&str; 4] = ["simple", "fade", "wipe", "grow"];

#[derive(Debug, Clone)]
pub struct PlaylistEntry {
    pub path: PathBuf,
    pub transition: TransitionState,
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
            kind: BrowserEntryKind::Parent,
        });
    }

    let mut directories = Vec::new();
    let mut images = Vec::new();

    for entry in fs::read_dir(dir).with_context(|| format!("cannot read '{}'", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata()?;

        if meta.is_dir() {
            directories.push(BrowserEntry {
                name: format!("{name}/"),
                path,
                kind: BrowserEntryKind::Directory,
            });
        } else if meta.is_file() && is_supported_image_path(&path) {
            images.push(BrowserEntry {
                name,
                path,
                kind: BrowserEntryKind::Image,
            });
        }
    }

    directories.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());
    images.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());

    entries.extend(directories);
    entries.extend(images);

    Ok(entries)
}

pub fn fuzzy_filter(entries: &[BrowserEntry], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }

    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = NucleoMatcher::new(NucleoConfig::DEFAULT.match_paths());
    let mut utf32_buffer = Vec::new();

    let mut scored = entries
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            pattern
                .score(
                    Utf32Str::new(entry.name.as_str(), &mut utf32_buffer),
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
        .map(|monitor| MonitorEntry {
            name: monitor.name,
            width: monitor.width,
            height: monitor.height,
            focused: monitor.focused,
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

            Some(MonitorEntry {
                name,
                width,
                height,
                focused: false,
            })
        })
        .collect::<Vec<_>>();

    Ok(monitors)
}

fn extract_wlr_dimensions(object: &serde_json::Map<String, Value>) -> Option<(u32, u32)> {
    if let Some(mode) = object.get("current_mode").and_then(Value::as_object) {
        let width = mode
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())?;
        let height = mode
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())?;
        return Some((width, height));
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
            let width = mode
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())?;
            let height = mode
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())?;
            Some((width, height))
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
    let (monitor_width, monitor_height) = (monitor_dim.0.max(1), monitor_dim.1.max(1));
    let (image_width, image_height) = image.dimensions();
    let (image_width, image_height) = (image_width.max(1), image_height.max(1));

    match scale_mode {
        ScaleMode::Stretch => image.resize_exact(
            monitor_width,
            monitor_height,
            imageops::FilterType::Lanczos3,
        ),
        ScaleMode::Fill => {
            let scale = (monitor_width as f32 / image_width as f32)
                .max(monitor_height as f32 / image_height as f32);
            let resized_width = ((image_width as f32 * scale).round() as u32).max(1);
            let resized_height = ((image_height as f32 * scale).round() as u32).max(1);
            let resized = image.resize_exact(
                resized_width,
                resized_height,
                imageops::FilterType::Lanczos3,
            );
            let x = resized_width.saturating_sub(monitor_width) / 2;
            let y = resized_height.saturating_sub(monitor_height) / 2;
            resized.crop_imm(x, y, monitor_width, monitor_height)
        }
        ScaleMode::Fit => {
            let scale = (monitor_width as f32 / image_width as f32)
                .min(monitor_height as f32 / image_height as f32);
            let resized_width = ((image_width as f32 * scale).round() as u32).max(1);
            let resized_height = ((image_height as f32 * scale).round() as u32).max(1);
            let resized = image.resize_exact(
                resized_width,
                resized_height,
                imageops::FilterType::Lanczos3,
            );
            let mut canvas: RgbaImage =
                ImageBuffer::from_pixel(monitor_width, monitor_height, Rgba([0, 0, 0, 255]));
            let x = i64::from(monitor_width.saturating_sub(resized_width) / 2);
            let y = i64::from(monitor_height.saturating_sub(resized_height) / 2);
            imageops::overlay(&mut canvas, &resized.to_rgba8(), x, y);
            DynamicImage::ImageRgba8(canvas)
        }
        ScaleMode::Center => {
            let crop_width = image_width.min(monitor_width);
            let crop_height = image_height.min(monitor_height);
            let source_x = image_width.saturating_sub(crop_width) / 2;
            let source_y = image_height.saturating_sub(crop_height) / 2;
            let cropped = image.crop_imm(source_x, source_y, crop_width, crop_height);
            let mut canvas: RgbaImage =
                ImageBuffer::from_pixel(monitor_width, monitor_height, Rgba([0, 0, 0, 255]));
            let x = i64::from(monitor_width.saturating_sub(crop_width) / 2);
            let y = i64::from(monitor_height.saturating_sub(crop_height) / 2);
            imageops::overlay(&mut canvas, &cropped.to_rgba8(), x, y);
            DynamicImage::ImageRgba8(canvas)
        }
    }
}
