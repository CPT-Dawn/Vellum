use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use tracing::warn;
use vellum_ipc::ScaleMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresenterBackend {
    Hyprpaper,
    None,
    Swaybg,
    Swww,
}

pub(crate) struct SwaybgController {
    disabled: bool,
    wayland_session: bool,
    backend: PresenterBackend,
    children: BTreeMap<String, Child>,
    warned_missing_backend: bool,
}

impl Default for SwaybgController {
    fn default() -> Self {
        let disabled = std::env::var("VELLUM_DISABLE_SWAYBG")
            .ok()
            .as_deref()
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let wayland_session = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let hyprland_session = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
        let backend = if disabled || !wayland_session {
            PresenterBackend::None
        } else if hyprland_session && command_exists("hyprctl") {
            PresenterBackend::Hyprpaper
        } else if command_exists("swaybg") {
            PresenterBackend::Swaybg
        } else if command_exists("swww") {
            PresenterBackend::Swww
        } else {
            PresenterBackend::None
        };

        Self {
            disabled,
            wayland_session,
            backend,
            children: BTreeMap::new(),
            warned_missing_backend: false,
        }
    }
}

impl SwaybgController {
    pub(crate) fn apply_to_output(
        &mut self,
        output: &str,
        path: &Path,
        mode: ScaleMode,
    ) -> Result<()> {
        if cfg!(test) {
            return Ok(());
        }

        if self.disabled {
            return Ok(());
        }

        match self.backend {
            PresenterBackend::None => {
                if self.wayland_session {
                    bail!(
                        "no wallpaper presenter backend available in Wayland session (tried hyprpaper/swaybg/swww)"
                    );
                }

                Ok(())
            }
            PresenterBackend::Hyprpaper => {
                if let Err(err) = self.apply_with_hyprpaper(output, path) {
                    if !self.warned_missing_backend {
                        warn!(error = %err, "hyprpaper apply failed, attempting fallback backends");
                        self.warned_missing_backend = true;
                    }

                    if command_exists("swaybg") {
                        self.backend = PresenterBackend::Swaybg;
                        return self.apply_with_swaybg(output, path, mode);
                    }
                    if command_exists("swww") {
                        self.backend = PresenterBackend::Swww;
                        return self.apply_with_swww(path);
                    }

                    return Err(err.context(
                        "no fallback presenter backend available; install and run hyprpaper, or install swaybg/swww",
                    ));
                }

                Ok(())
            }
            PresenterBackend::Swaybg => self.apply_with_swaybg(output, path, mode),
            PresenterBackend::Swww => self.apply_with_swww(path),
        }
    }

    fn apply_with_hyprpaper(&mut self, output: &str, path: &Path) -> Result<()> {
        if let Err(first_err) = self.run_hyprpaper_sequence(output, path) {
            if command_exists("hyprpaper") {
                let _ = Command::new("hyprpaper")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                thread::sleep(Duration::from_millis(300));

                if let Ok(()) = self.run_hyprpaper_sequence(output, path) {
                    return Ok(());
                }
            }

            return Err(first_err);
        }

        Ok(())
    }

    fn run_hyprpaper_sequence(&self, output: &str, path: &Path) -> Result<()> {
        run_status(
            Command::new("hyprctl")
                .arg("hyprpaper")
                .arg("preload")
                .arg(path),
            "failed to preload wallpaper in hyprpaper",
        )?;

        let wallpaper_arg = format!("{output},{}", path.display());
        run_status(
            Command::new("hyprctl")
                .arg("hyprpaper")
                .arg("wallpaper")
                .arg(&wallpaper_arg),
            "failed to set hyprpaper wallpaper",
        )
    }

    fn apply_with_swaybg(&mut self, output: &str, path: &Path, mode: ScaleMode) -> Result<()> {
        self.remove_output(output);

        let child = Command::new("swaybg")
            .arg("-o")
            .arg(output)
            .arg("-i")
            .arg(path)
            .arg("-m")
            .arg(scale_mode_to_swaybg(mode))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start swaybg for output {output}"))?;

        self.children.insert(output.to_string(), child);
        Ok(())
    }

    fn apply_with_swww(&mut self, path: &Path) -> Result<()> {
        let try_status = Command::new("swww")
            .arg("img")
            .arg(path)
            .arg("--transition-type")
            .arg("none")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if matches!(try_status, Ok(status) if status.success()) {
            return Ok(());
        }

        if command_exists("swww-daemon") {
            let _ = Command::new("swww-daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            let retry = Command::new("swww")
                .arg("img")
                .arg(path)
                .arg("--transition-type")
                .arg("none")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            if matches!(retry, Ok(status) if status.success()) {
                return Ok(());
            }
        }

        bail!("failed to set wallpaper via swww backend")
    }

    pub(crate) fn remove_output(&mut self, output: &str) {
        if let Some(mut child) = self.children.remove(output) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub(crate) fn clear_all(&mut self) {
        if matches!(self.backend, PresenterBackend::Hyprpaper) {
            let _ = run_status(
                Command::new("hyprctl")
                    .arg("hyprpaper")
                    .arg("unload")
                    .arg("all"),
                "failed to clear hyprpaper wallpapers",
            );
            return;
        }

        if matches!(self.backend, PresenterBackend::Swww) {
            let _ = Command::new("swww")
                .arg("clear")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            return;
        }

        let outputs = self.children.keys().cloned().collect::<Vec<_>>();
        for output in outputs {
            self.remove_output(&output);
        }
    }

    #[cfg(test)]
    pub(crate) fn running_count(&self) -> usize {
        self.children.len()
    }
}

impl Drop for SwaybgController {
    fn drop(&mut self) {
        self.clear_all();
    }
}

fn command_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn run_status(command: &mut Command, context: &str) -> Result<()> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| context.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stderr.is_empty() && stdout.is_empty() {
        bail!("{context}");
    }

    if !stderr.is_empty() {
        bail!("{context}: {stderr}");
    }

    bail!("{context}: {stdout}")
}

fn scale_mode_to_swaybg(mode: ScaleMode) -> &'static str {
    match mode {
        ScaleMode::Fit => "fit",
        ScaleMode::Fill => "stretch",
        ScaleMode::Crop => "fill",
    }
}

#[cfg(test)]
mod tests {
    use super::scale_mode_to_swaybg;
    use vellum_ipc::ScaleMode;

    #[test]
    fn maps_scale_modes_to_swaybg_modes() {
        assert_eq!(scale_mode_to_swaybg(ScaleMode::Fit), "fit");
        assert_eq!(scale_mode_to_swaybg(ScaleMode::Fill), "stretch");
        assert_eq!(scale_mode_to_swaybg(ScaleMode::Crop), "fill");
    }
}
