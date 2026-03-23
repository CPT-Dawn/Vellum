mod backend;
mod command_queue;
mod image_pipeline;
mod layer_shell;
mod output_registry;
mod perf_checks;
mod shm_pool;
mod swaybg;

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::{info, warn};
use vellum_ipc::ScaleMode;

use backend::BackendState;
pub(crate) use command_queue::RenderCommand;
use command_queue::RenderCommandQueue;
use image_pipeline::ImagePipeline;
use layer_shell::LayerShellSession;
pub(crate) use output_registry::OutputRegistry;

#[derive(Default)]
pub(crate) struct RendererState {
    outputs: OutputRegistry,
    queue: RenderCommandQueue,
    backend: BackendState,
    image_pipeline: ImagePipeline,
    session: LayerShellSession,
}

impl RendererState {
    pub(crate) fn refresh_outputs(&mut self, output_names: Vec<String>) {
        self.outputs.update(output_names.clone());
        if let Err(err) = self.session.sync_outputs(output_names) {
            warn!(error = %err, "renderer session failed to refresh outputs");
        }
    }

    pub(crate) fn enqueue_apply(
        &mut self,
        monitor: Option<String>,
        path: PathBuf,
        mode: ScaleMode,
    ) {
        self.queue.push(RenderCommand::ApplyAssignment {
            monitor,
            path,
            mode,
        });
    }

    pub(crate) fn enqueue_clear(&mut self) {
        self.queue.push(RenderCommand::ClearAssignments);
    }

    pub(crate) fn apply_pending(&mut self) -> Result<()> {
        // This is a scaffold stage: commands are queued and logged while the
        // concrete SCTK/layer-shell renderer backend is being implemented.
        for command in self.queue.drain() {
            match &command {
                RenderCommand::ApplyAssignment {
                    monitor,
                    path,
                    mode,
                } => {
                    let preflight = self.image_pipeline.inspect(path);
                    if !preflight.exists {
                        warn!(path = %path.display(), "renderer preflight: image path does not exist");
                    } else if let Some((width, height)) = preflight.dimensions {
                        info!(path = %path.display(), width, height, ?mode, "renderer preflight decoded image");
                    } else {
                        warn!(
                            path = %path.display(),
                            error = ?preflight.decode_error,
                            "renderer preflight could not decode image"
                        );
                    }

                    if let Some(ref name) = monitor {
                        if !self.outputs.contains(name) {
                            info!(target = ?name, path = %path.display(), ?mode, "queued apply for currently unknown output");
                        }
                    }

                    self.session
                        .apply_assignment(monitor.as_deref(), path.as_path(), *mode)
                        .with_context(|| {
                            format!("renderer session apply failed for {}", path.display())
                        })?;

                    info!(target = ?monitor, path = %path.display(), ?mode, "renderer scaffold accepted apply command");
                }
                RenderCommand::ClearAssignments => {
                    self.session
                        .clear_assignments()
                        .context("renderer session clear failed")?;
                    info!("renderer scaffold accepted clear command");
                }
            }

            // First renderer milestone: keep an internal backend assignment state
            // synchronized with queued render commands.
            self.backend.apply_command(command);
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn backend_assignment_count(&self) -> usize {
        self.backend.assignment_count()
    }

    #[cfg(test)]
    pub(crate) fn backend_mode_for(&self, monitor: Option<&str>) -> Option<ScaleMode> {
        self.backend.assignment_mode_for(monitor)
    }

    #[cfg(test)]
    pub(crate) fn session_surface_count(&self) -> usize {
        self.session.known_output_count()
    }

    #[cfg(test)]
    pub(crate) fn session_buffer_count(&self) -> usize {
        self.session.pool_entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_then_clear_updates_backend_state() {
        let mut renderer = RendererState::default();

        renderer.enqueue_apply(
            Some("DP-1".to_string()),
            PathBuf::from("/tmp/wall.png"),
            ScaleMode::Fill,
        );
        renderer
            .apply_pending()
            .expect("renderer apply should succeed in test mode");

        assert_eq!(renderer.backend.assignment_count(), 1);
        assert_eq!(
            renderer.backend.assignment_mode_for(Some("DP-1")),
            Some(ScaleMode::Fill)
        );
        assert_eq!(
            renderer.backend.assignment_path_for(Some("DP-1")),
            Some(PathBuf::from("/tmp/wall.png"))
        );

        renderer.enqueue_clear();
        renderer
            .apply_pending()
            .expect("renderer clear should succeed in test mode");

        assert_eq!(renderer.backend.assignment_count(), 0);
    }
}
