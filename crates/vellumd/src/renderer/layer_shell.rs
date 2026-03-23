use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use vellum_ipc::ScaleMode;

use crate::renderer::image_blit::{render_frame, NativeFrame};
use crate::renderer::native_commit::NativeCommitPlan;
use crate::renderer::shm_pool::ShmPool;
use crate::renderer::swaybg::SwaybgController;
use crate::renderer::OutputLayout;

#[derive(Debug, Clone)]
struct OutputSurface {
    width: u32,
    height: u32,
    scale_factor: u32,
    current_stride: usize,
    current_buffer_id: Option<u64>,
    current_path: Option<PathBuf>,
    current_mode: Option<ScaleMode>,
    current_frame: Option<NativeFrame>,
}

impl Default for OutputSurface {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            scale_factor: 1,
            current_stride: 1920 * 4,
            current_buffer_id: None,
            current_path: None,
            current_mode: None,
            current_frame: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct LayerShellSession {
    surfaces: BTreeMap<String, OutputSurface>,
    shm_pool: ShmPool,
    presenter: SwaybgController,
    pending_commits: Vec<NativeCommitPlan>,
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
            self.pending_commits
                .retain(|commit| commit.output != output);
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

    pub(crate) fn sync_output_layouts<I>(&mut self, layouts: I) -> Result<()>
    where
        I: IntoIterator<Item = OutputLayout>,
    {
        for layout in layouts {
            if let Some(surface) = self.surfaces.get_mut(&layout.name) {
                surface.width = layout.width.max(1);
                surface.height = layout.height.max(1);
                surface.scale_factor = layout.scale_factor.max(1);
            }
        }

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
                    Self::render_to_surface(
                        target,
                        surface,
                        &mut self.shm_pool,
                        &mut self.pending_commits,
                        path,
                        mode,
                    )?;
                    self.presenter.apply_to_output(target, path, mode)?;
                }
            }
            None => {
                for (output, surface) in &mut self.surfaces {
                    Self::render_to_surface(
                        output,
                        surface,
                        &mut self.shm_pool,
                        &mut self.pending_commits,
                        path,
                        mode,
                    )?;
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
            surface.current_frame = None;
        }
        self.pending_commits.clear();
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
        output: &str,
        surface: &mut OutputSurface,
        pool: &mut ShmPool,
        pending_commits: &mut Vec<NativeCommitPlan>,
        path: &Path,
        mode: ScaleMode,
    ) -> Result<()> {
        let target_width = surface.width.saturating_mul(surface.scale_factor).max(1);
        let target_height = surface.height.saturating_mul(surface.scale_factor).max(1);

        let frame = match render_frame(path, target_width, target_height, mode) {
            Ok(frame) => frame,
            Err(err) => {
                warn!(
                    error = %err,
                    path = %path.display(),
                    width = target_width,
                    height = target_height,
                    "native frame rendering failed, using solid fallback frame"
                );
                NativeFrame::solid_black(target_width, target_height)
            }
        };

        let required_bytes = frame.pixels.len().max(4);

        let next_buffer = pool.acquire(required_bytes)?;
        pool.upload(next_buffer, &frame.pixels)?;
        if let Some(previous) = surface.current_buffer_id.replace(next_buffer) {
            pool.release(previous);
        }

        surface.current_stride = frame.stride;
        surface.current_path = Some(path.to_path_buf());
        surface.current_mode = Some(mode);
        surface.current_frame = Some(frame);

        pending_commits.push(NativeCommitPlan {
            output: output.to_string(),
            width: target_width,
            height: target_height,
            stride: surface.current_stride,
            buffer_id: next_buffer,
            source_path: path.to_path_buf(),
            mode,
        });
        Ok(())
    }

    pub(crate) fn drain_native_commit_plans(&mut self) -> Vec<NativeCommitPlan> {
        self.pending_commits.drain(..).collect()
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

    #[cfg(test)]
    pub(crate) fn frame_byte_len_for(&self, output: &str) -> Option<usize> {
        self.surfaces
            .get(output)
            .and_then(|surface| surface.current_frame.as_ref())
            .map(|frame| frame.pixels.len())
    }

    #[cfg(test)]
    pub(crate) fn pending_commit_count(&self) -> usize {
        self.pending_commits.len()
    }
}

#[cfg(test)]
mod tests {
    use super::LayerShellSession;
    use crate::renderer::OutputLayout;
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
        assert_eq!(session.frame_byte_len_for("DP-1"), Some(1920 * 1080 * 4));
        assert_eq!(session.pending_commit_count(), 2);

        let commits = session.drain_native_commit_plans();
        assert_eq!(commits.len(), 2);

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

    #[test]
    fn sync_output_layouts_updates_surface_dimensions() {
        let mut session = LayerShellSession::default();
        assert!(session.sync_outputs(vec!["DP-1".to_string()]).is_ok());
        assert!(session
            .sync_output_layouts(vec![OutputLayout {
                name: "DP-1".to_string(),
                width: 4,
                height: 3,
                scale_factor: 2,
            }])
            .is_ok());

        assert!(session
            .apply_assignment(Some("DP-1"), Path::new("/tmp/layout.png"), ScaleMode::Fit)
            .is_ok());
        assert_eq!(session.frame_byte_len_for("DP-1"), Some(8 * 6 * 4));
    }
}
