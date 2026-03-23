use anyhow::{Context, Result};
use ratatui::style::Color;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::path::PathBuf;
use vellum_ipc::{AssignmentEntry, Request, Response, ScaleMode};

use crate::daemon_client::send_request_blocking;
use crate::display::MonitorProfile;
use crate::images::{is_supported_image_path, load_image};

pub(crate) struct App {
    pub(crate) files: Vec<PathBuf>,
    pub(crate) selected: usize,
    pub(crate) image_root: PathBuf,
    pub(crate) picker: Picker,
    pub(crate) image_state: Option<StatefulProtocol>,
    pub(crate) preview_info: String,
    pub(crate) monitor_profile: MonitorProfile,
    pub(crate) status: String,
    pub(crate) pending_g: bool,
    pub(crate) show_help: bool,
    pub(crate) theme: UiTheme,
    pub(crate) socket_path: Option<PathBuf>,
    pub(crate) monitor_targets: Vec<String>,
    pub(crate) target_index: usize,
    pub(crate) assignments: Vec<AssignmentEntry>,
    pub(crate) scale_mode: ScaleMode,
}

#[derive(Clone)]
pub(crate) struct UiTheme {
    pub(crate) chrome: Color,
    pub(crate) panel: Color,
    pub(crate) accent: Color,
    pub(crate) accent_alt: Color,
    pub(crate) text: Color,
    pub(crate) muted: Color,
    pub(crate) warn: Color,
}

impl App {
    pub(crate) fn discover_files(
        image_root: PathBuf,
        monitor_profile: MonitorProfile,
        socket_path: Option<PathBuf>,
    ) -> Result<Self> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&image_root)
            .with_context(|| format!("failed to read image directory {}", image_root.display()))?
        {
            let entry = entry.context("failed to read directory entry")?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if is_supported_image_path(&path) {
                files.push(path);
            }
        }

        files.sort();
        let mut app = Self {
            files,
            selected: 0,
            image_root,
            picker: Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((16, 8))),
            image_state: None,
            preview_info: "No image selected".to_string(),
            monitor_profile,
            status: "Normal | j/k move | gg/G edges | ? help | q quit".to_string(),
            pending_g: false,
            show_help: false,
            theme: UiTheme::arch_punk(),
            socket_path,
            monitor_targets: Vec::new(),
            target_index: 0,
            assignments: Vec::new(),
            scale_mode: ScaleMode::Fit,
        };
        app.refresh_preview();
        app.refresh_monitor_targets();
        app.refresh_assignments_cache_silent();
        Ok(app)
    }

    pub(crate) fn select_next(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = (self.selected + 1) % self.files.len();
        self.status = format!("Normal | moved to {}", self.selected);
        self.refresh_preview();
    }

    pub(crate) fn select_previous(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = if self.selected == 0 {
            self.files.len().saturating_sub(1)
        } else {
            self.selected.saturating_sub(1)
        };
        self.status = format!("Normal | moved to {}", self.selected);
        self.refresh_preview();
    }

    pub(crate) fn select_first(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = 0;
        self.status = "Normal | jumped to first image".to_string();
        self.refresh_preview();
    }

    pub(crate) fn select_last(&mut self) {
        if self.files.is_empty() {
            return;
        }

        self.selected = self.files.len().saturating_sub(1);
        self.status = "Normal | jumped to last image".to_string();
        self.refresh_preview();
    }

    pub(crate) fn select_page_down(&mut self, amount: usize) {
        if self.files.is_empty() {
            return;
        }

        self.selected = (self.selected + amount).min(self.files.len().saturating_sub(1));
        self.status = format!("Normal | page down {}", amount);
        self.refresh_preview();
    }

    pub(crate) fn select_page_up(&mut self, amount: usize) {
        if self.files.is_empty() {
            return;
        }

        self.selected = self.selected.saturating_sub(amount);
        self.status = format!("Normal | page up {}", amount);
        self.refresh_preview();
    }

    pub(crate) fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        self.status = if self.show_help {
            "Help overlay enabled".to_string()
        } else {
            "Help overlay disabled".to_string()
        };
    }

    pub(crate) fn daemon_status(&self) -> &'static str {
        if self.socket_path.is_some() {
            "online-configured"
        } else {
            "offline"
        }
    }

    pub(crate) fn selected_file_name(&self) -> Option<&str> {
        self.files
            .get(self.selected)
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
    }

    pub(crate) fn refresh_preview(&mut self) {
        let Some(path) = self.files.get(self.selected) else {
            self.image_state = None;
            self.preview_info = "No image found in directory".to_string();
            return;
        };

        match load_image(path) {
            Ok(image) => {
                let dimensions = (image.width(), image.height());
                self.image_state = Some(self.picker.new_resize_protocol(image));
                self.preview_info = format!("Loaded {}x{}", dimensions.0, dimensions.1);
            }
            Err(err) => {
                self.image_state = None;
                self.preview_info = format!("Failed to load: {err}");
            }
        }
    }

    pub(crate) fn reload_files(&mut self) {
        let mut files = Vec::new();
        let entries = std::fs::read_dir(&self.image_root);
        let Ok(entries) = entries else {
            self.status = format!(
                "Failed to reload: cannot read {}",
                self.image_root.display()
            );
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_supported_image_path(&path) {
                files.push(path);
            }
        }

        files.sort();
        self.files = files;
        if self.selected >= self.files.len() {
            self.selected = self.files.len().saturating_sub(1);
        }
        self.status = format!("Reloaded {} images", self.files.len());
        self.refresh_preview();
    }

    pub(crate) fn apply_selected_wallpaper(&mut self) {
        let Some(path) = self.files.get(self.selected).cloned() else {
            self.status = "No selected image to apply".to_string();
            return;
        };

        let monitor = self.current_target_name();

        let response = self.send_daemon_request(Request::SetWallpaper {
            path: path.display().to_string(),
            monitor,
            mode: self.scale_mode,
        });

        match response {
            Ok(Response::Ok) => {
                self.status = format!("Applied wallpaper: {}", path.display());
                self.refresh_assignments_cache_silent();
            }
            Ok(Response::Error { message }) => {
                self.status = format!("Daemon rejected wallpaper: {message}");
            }
            Ok(other) => {
                self.status = format!("Unexpected daemon response: {other:?}");
            }
            Err(err) => {
                self.status = format!("Apply failed: {err:#}");
            }
        }
    }

    pub(crate) fn fetch_monitors(&mut self) {
        let response = self.send_daemon_request(Request::GetMonitors);
        match response {
            Ok(Response::Monitors { names }) => {
                if names.is_empty() {
                    self.status = "Daemon reported no monitors".to_string();
                } else {
                    self.monitor_targets = names.clone();
                    if self.target_index > self.monitor_targets.len() {
                        self.target_index = 0;
                    }
                    self.status = format!("Monitors: {}", names.join(", "));
                }
            }
            Ok(Response::Error { message }) => {
                self.status = format!("Monitor query failed: {message}");
            }
            Ok(other) => {
                self.status = format!("Unexpected monitor response: {other:?}");
            }
            Err(err) => {
                self.status = format!("Monitor query failed: {err:#}");
            }
        }
    }

    pub(crate) fn fetch_assignments(&mut self) {
        let response = self.send_daemon_request(Request::GetAssignments);
        match response {
            Ok(Response::Assignments { entries }) => {
                self.assignments = entries.clone();
                if entries.is_empty() {
                    self.status = "No wallpaper assignments recorded yet".to_string();
                } else {
                    let compact = entries
                        .iter()
                        .map(|entry| {
                            let monitor =
                                entry.monitor.clone().unwrap_or_else(|| "all".to_string());
                            let file = PathBuf::from(&entry.path)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("<path>")
                                .to_string();
                            format!("{monitor}:{file}({})", scale_mode_label(entry.mode))
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    self.status = format!("Assignments {compact}");
                }
            }
            Ok(Response::Error { message }) => {
                self.status = format!("Assignments query failed: {message}");
            }
            Ok(other) => {
                self.status = format!("Unexpected assignments response: {other:?}");
            }
            Err(err) => {
                self.status = format!("Assignments query failed: {err:#}");
            }
        }
    }

    pub(crate) fn clear_assignments(&mut self) {
        let response = self.send_daemon_request(Request::ClearAssignments);
        match response {
            Ok(Response::Ok) => {
                self.assignments.clear();
                self.status = "Cleared all daemon assignments".to_string();
            }
            Ok(Response::Error { message }) => {
                self.status = format!("Clear failed: {message}");
            }
            Ok(other) => {
                self.status = format!("Unexpected clear response: {other:?}");
            }
            Err(err) => {
                self.status = format!("Clear failed: {err:#}");
            }
        }
    }

    pub(crate) fn refresh_monitor_targets(&mut self) {
        let response = self.send_daemon_request(Request::GetMonitors);
        if let Ok(Response::Monitors { names }) = response {
            self.monitor_targets = names;
            if self.target_index > self.monitor_targets.len() {
                self.target_index = 0;
            }
        }
    }

    pub(crate) fn cycle_monitor_target(&mut self) {
        if self.monitor_targets.is_empty() {
            self.refresh_monitor_targets();
        }

        let total_slots = self.monitor_targets.len() + 1;
        if total_slots == 0 {
            self.target_index = 0;
            self.status = "Target output: all".to_string();
            return;
        }

        self.target_index = (self.target_index + 1) % total_slots;
        self.status = format!("Target output: {}", self.current_target_label());
    }

    pub(crate) fn cycle_scale_mode(&mut self) {
        self.scale_mode = match self.scale_mode {
            ScaleMode::Fit => ScaleMode::Fill,
            ScaleMode::Fill => ScaleMode::Crop,
            ScaleMode::Crop => ScaleMode::Fit,
        };
        self.status = format!("Scale mode: {}", self.scale_mode_label());
    }

    pub(crate) fn current_target_name(&self) -> Option<String> {
        if self.target_index == 0 {
            None
        } else {
            self.monitor_targets.get(self.target_index - 1).cloned()
        }
    }

    pub(crate) fn current_target_label(&self) -> String {
        self.current_target_name()
            .unwrap_or_else(|| "all outputs".to_string())
    }

    pub(crate) fn scale_mode_label(&self) -> &'static str {
        scale_mode_label(self.scale_mode)
    }

    pub(crate) fn send_daemon_request(&self, request: Request) -> Result<Response> {
        let socket_path = self
            .socket_path
            .as_ref()
            .context("daemon socket is not configured in this environment")?;

        send_request_blocking(socket_path, request)
    }

    pub(crate) fn refresh_assignments_cache_silent(&mut self) {
        let response = self.send_daemon_request(Request::GetAssignments);
        if let Ok(Response::Assignments { entries }) = response {
            self.assignments = entries;
        }
    }

    pub(crate) fn assignments_overview(&self) -> String {
        if self.assignments.is_empty() {
            return "none".to_string();
        }

        let shown = self
            .assignments
            .iter()
            .take(3)
            .map(|entry| {
                let monitor = entry.monitor.as_deref().unwrap_or("all");
                let file = PathBuf::from(&entry.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<path>")
                    .to_string();
                format!("{monitor}:{file}({})", scale_mode_label(entry.mode))
            })
            .collect::<Vec<_>>()
            .join(" | ");

        if self.assignments.len() > 3 {
            format!("{shown} | +{} more", self.assignments.len() - 3)
        } else {
            shown
        }
    }
}

impl UiTheme {
    fn arch_punk() -> Self {
        Self {
            chrome: Color::Rgb(14, 17, 23),
            panel: Color::Rgb(58, 74, 112),
            accent: Color::Rgb(129, 207, 255),
            accent_alt: Color::Rgb(255, 92, 143),
            text: Color::Rgb(196, 206, 240),
            muted: Color::Rgb(156, 168, 198),
            warn: Color::Rgb(255, 158, 100),
        }
    }
}

pub(crate) fn print_assignment_entries(entries: &[AssignmentEntry]) {
    if entries.is_empty() {
        println!("no assignments recorded");
        return;
    }

    for entry in entries {
        let monitor = entry.monitor.as_deref().unwrap_or("all");
        println!(
            "{monitor} -> {} ({})",
            entry.path,
            scale_mode_label(entry.mode)
        );
    }
}

fn scale_mode_label(mode: ScaleMode) -> &'static str {
    match mode {
        ScaleMode::Fit => "fit",
        ScaleMode::Fill => "fill",
        ScaleMode::Crop => "crop",
    }
}
