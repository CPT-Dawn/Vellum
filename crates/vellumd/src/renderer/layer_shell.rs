use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::info;
use vellum_ipc::ScaleMode;

use crate::renderer::shm_pool::ShmPool;
use crate::renderer::swaybg::SwaybgController;

#[derive(Debug, Clone)]
struct OutputSurface {
    width: u32,
    height: u32,
    scale_factor: u32,
    current_buffer_id: Option<u64>,
    current_path: Option<PathBuf>,
    current_mode: Option<ScaleMode>,
}

impl Default for OutputSurface {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            scale_factor: 1,
            current_buffer_id: None,
            current_path: None,
            current_mode: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct LayerShellSession {
    surfaces: BTreeMap<String, OutputSurface>,
    shm_pool: ShmPool,
    presenter: SwaybgController,
}

impl LayerShellSession {
    pub(crate) fn sync_outputs<I>(&mut self, outputs: I) -> Result<()>
    where
        I: IntoIterator<Item = String>,
    {
        let incoming: BTreeMap<String, ()> = outputs.into_iter().map(|name| (name, ())).collect();

        let removed: Vec<String> = self
            .surfaces
            .keys()
            .filter(|name| !incoming.contains_key(*name))
            .cloned()
            .collect();

        for output in removed {
            if let Some(mut surface) = self.surfaces.remove(&output) {
                if let Some(buffer_id) = surface.current_buffer_id.take() {
                    self.shm_pool.release(buffer_id);
                }
            }
            self.presenter.remove_output(&output);
        }

        for output in incoming.keys() {
            self.surfaces.entry(output.clone()).or_default();
        }

        self.shm_pool.reclaim_unused(1);
        info!(
            count = self.surfaces.len(),
            "renderer layer-shell output snapshot updated"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reconfigure_output(
        &mut self,
        output: &str,
        width: u32,
        height: u32,
        scale_factor: u32,
    ) -> Result<()> {
        if let Some(surface) = self.surfaces.get_mut(output) {
            surface.width = width.max(1);
            surface.height = height.max(1);
            surface.scale_factor = scale_factor.max(1);
        }
        Ok(())
    }

    pub(crate) fn apply_assignment(
        &mut self,
        monitor: Option<&str>,
        path: &Path,
        mode: ScaleMode,
    ) -> Result<()> {
        match monitor {
            Some(target) => {
                if let Some(surface) = self.surfaces.get_mut(target) {
                    Self::render_to_surface(surface, &mut self.shm_pool, path, mode)?;
                    self.presenter.apply_to_output(target, path, mode)?;
                }
            }
            None => {
                for (output, surface) in &mut self.surfaces {
                    Self::render_to_surface(surface, &mut self.shm_pool, path, mode)?;
                    self.presenter.apply_to_output(output, path, mode)?;
                }
            }
        }

        self.shm_pool.reclaim_unused(6);
        info!(
            monitor,
            path = %path.display(),
            ?mode,
            surfaces = self.surfaces.len(),
            leased_buffers = self.shm_pool.leased_count(),
            "renderer layer-shell apply requested"
        );
        Ok(())
    }

    pub(crate) fn clear_assignments(&mut self) -> Result<()> {
        for surface in self.surfaces.values_mut() {
            if let Some(buffer_id) = surface.current_buffer_id.take() {
                self.shm_pool.release(buffer_id);
            }
            surface.current_path = None;
            surface.current_mode = None;
        }
        self.presenter.clear_all();

        self.shm_pool.reclaim_unused(0);
        info!(
            surfaces = self.surfaces.len(),
            pool_entries = self.shm_pool.entry_count(),
            "renderer layer-shell clear requested"
        );
        Ok(())
    }

    fn render_to_surface(
        surface: &mut OutputSurface,
        pool: &mut ShmPool,
        path: &Path,
        mode: ScaleMode,
    ) -> Result<()> {
        let logical_pixels = (surface.width as usize)
            .saturating_mul(surface.height as usize)
            .saturating_mul(surface.scale_factor as usize);
        let required_bytes = logical_pixels.saturating_mul(4).max(4);

        let next_buffer = pool.acquire(required_bytes)?;
        if let Some(previous) = surface.current_buffer_id.replace(next_buffer) {
            pool.release(previous);
        }

        surface.current_path = Some(path.to_path_buf());
        surface.current_mode = Some(mode);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn known_output_count(&self) -> usize {
        self.surfaces.len()
    }

    #[cfg(test)]
    pub(crate) fn leased_buffer_count(&self) -> usize {
        self.shm_pool.leased_count()
    }

    #[cfg(test)]
    pub(crate) fn pool_entry_count(&self) -> usize {
        self.shm_pool.entry_count()
    }

    #[cfg(test)]
    pub(crate) fn pool_total_bytes(&self) -> usize {
        self.shm_pool.total_bytes()
    }

    #[cfg(test)]
    pub(crate) fn has_assignment_for(&self, output: &str) -> bool {
        self.surfaces
            .get(output)
            .and_then(|surface| surface.current_path.as_ref())
            .is_some()
    }

    #[cfg(test)]
    pub(crate) fn presenter_process_count(&self) -> usize {
        self.presenter.running_count()
    }
}

#[cfg(test)]
mod tests {
    use super::LayerShellSession;
    use std::path::Path;
    use vellum_ipc::ScaleMode;

    #[test]
    fn sync_outputs_replaces_known_output_snapshot() {
        let mut session = LayerShellSession::default();
        assert!(session
            .sync_outputs(vec!["DP-1".to_string(), "HDMI-A-1".to_string()])
            .is_ok());
        assert_eq!(session.known_output_count(), 2);

        assert!(session.sync_outputs(vec!["eDP-1".to_string()]).is_ok());
        assert_eq!(session.known_output_count(), 1);
    }

    #[test]
    fn apply_and_clear_manage_buffers_and_assignments() {
        let mut session = LayerShellSession::default();
        assert!(session
            .sync_outputs(vec!["DP-1".to_string(), "HDMI-A-1".to_string()])
            .is_ok());

        assert!(session
            .apply_assignment(None, Path::new("/tmp/wall.png"), ScaleMode::Fill)
            .is_ok());
        assert_eq!(session.leased_buffer_count(), 2);
        assert!(session.has_assignment_for("DP-1"));
        assert!(session.has_assignment_for("HDMI-A-1"));

        assert!(session.clear_assignments().is_ok());
        assert_eq!(session.leased_buffer_count(), 0);
        assert_eq!(session.pool_entry_count(), 0);
        assert_eq!(session.presenter_process_count(), 0);
    }

    #[test]
    fn reconfigure_output_increases_buffer_size_on_next_apply() {
        let mut session = LayerShellSession::default();
        assert!(session.sync_outputs(vec!["DP-1".to_string()]).is_ok());
        assert!(session
            .apply_assignment(Some("DP-1"), Path::new("/tmp/base.png"), ScaleMode::Fit)
            .is_ok());
        let baseline = session.pool_total_bytes();

        assert!(session.reconfigure_output("DP-1", 3840, 2160, 1).is_ok());
        assert!(session
            .apply_assignment(Some("DP-1"), Path::new("/tmp/next.png"), ScaleMode::Crop)
            .is_ok());
        assert!(session.pool_total_bytes() >= baseline);
    }
}
