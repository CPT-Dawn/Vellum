use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use vellum_ipc::ScaleMode;

pub(crate) struct SwaybgController {
    wayland_session: bool,
    swaybg_available: bool,
    children: BTreeMap<String, Child>,
}

impl Default for SwaybgController {
    fn default() -> Self {
        let wayland_session = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let swaybg_available = command_exists("swaybg");
        Self {
            wayland_session,
            swaybg_available,
            children: BTreeMap::new(),
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

        if !self.wayland_session {
            return Ok(());
        }

        if !self.swaybg_available {
            bail!(
                "swaybg is required for visible wallpaper output in this build; install swaybg or run in a non-Wayland test environment"
            );
        }

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

    pub(crate) fn remove_output(&mut self, output: &str) {
        if let Some(mut child) = self.children.remove(output) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub(crate) fn clear_all(&mut self) {
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
