mod backend;
mod command_queue;
mod image_blit;
mod image_pipeline;
mod layer_shell;
mod native_commit;
mod output_registry;
mod perf_checks;
mod shm_pool;
mod swaybg;
mod wl_shm_bridge;

use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::path::PathBuf;
use tracing::{info, warn};
use vellum_ipc::ScaleMode;

use backend::BackendState;
pub(crate) use command_queue::RenderCommand;
use command_queue::RenderCommandQueue;
use image_pipeline::ImagePipeline;
use layer_shell::LayerShellSession;
pub(crate) use output_registry::OutputRegistry;
use wl_shm_bridge::WlShmBridge;

#[derive(Default)]
pub(crate) struct RendererState {
    outputs: OutputRegistry,
    queue: RenderCommandQueue,
    backend: BackendState,
    image_pipeline: ImagePipeline,
    session: LayerShellSession,
    shm_bridge: WlShmBridge,
}

impl RendererState {
    pub(crate) fn refresh_outputs(&mut self, output_names: Vec<String>) {
        let delta = self.outputs.reconcile(output_names.clone());
        if let Err(err) = self.session.sync_outputs(output_names) {
            warn!(error = %err, "renderer session failed to refresh outputs");
            return;
        }

        if delta.changed() {
            info!(
                added = ?delta.added,
                removed = ?delta.removed,
                "renderer output lifecycle changed"
            );
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
        let mut successfully_applied = Vec::new();
        let mut pending: VecDeque<RenderCommand> = self.queue.drain().into();

        while let Some(command) = pending.pop_front() {
            let command_result = match &command {
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
                    Ok(())
                }
                RenderCommand::ClearAssignments => {
                    self.session
                        .clear_assignments()
                        .context("renderer session clear failed")?;
                    info!("renderer scaffold accepted clear command");
                    Ok(())
                }
            };

            if let Err(err) = command_result {
                // Requeue current and remaining commands so work is not lost on
                // transient apply failures.
                let requeued = pending.len().saturating_add(1);
                let discarded_commits = self.session.drain_native_commit_plans().len();
                self.queue
                    .prepend(std::iter::once(command).chain(pending.into_iter()));
                warn!(
                    requeued,
                    discarded_commits,
                    error = %err,
                    "renderer command processing failed; commands requeued"
                );
                return Err(err);
            }

            successfully_applied.push(command);
        }

        // Bridge stage: forward native commit descriptors into wl_shm bridge.
        let commits = self.session.drain_native_commit_plans();
        for commit in &commits {
            info!(
                output = %commit.output,
                width = commit.width,
                height = commit.height,
                stride = commit.stride,
                buffer_id = commit.buffer_id,
                path = %commit.source_path.display(),
                ?commit.mode,
                "native commit plan prepared"
            );
        }
        self.shm_bridge.submit(commits);
        let committed = match self.shm_bridge.flush() {
            Ok(committed) => committed,
            Err(err) => {
                let requeued = successfully_applied.len();
                self.queue.prepend(successfully_applied);
                warn!(
                    requeued,
                    error = %err,
                    "wl_shm bridge flush failed; commands requeued"
                );
                return Err(err).context("wl_shm bridge flush failed");
            }
        };
        info!(
            committed,
            outputs = self.shm_bridge.committed_output_count(),
            "wl_shm bridge committed native plans"
        );

        // Keep daemon-facing assignment state aligned with successful native
        // commit execution only.
        for command in successfully_applied {
            self.backend.apply_command(command);
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn committed_output_count(&self) -> usize {
        self.shm_bridge.committed_output_count()
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

    #[cfg(test)]
    pub(crate) fn pending_command_count(&self) -> usize {
        self.queue.len()
    }

    #[cfg(test)]
    pub(crate) fn inject_invalid_commit_plan(&mut self, output: &str) {
        use crate::renderer::native_commit::NativeCommitPlan;

        self.shm_bridge.submit([NativeCommitPlan {
            output: output.to_string(),
            width: 1920,
            height: 1080,
            stride: 1,
            buffer_id: 999,
            source_path: PathBuf::from("/tmp/invalid.png"),
            mode: ScaleMode::Fit,
        }]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_then_clear_updates_backend_state() {
        let mut renderer = RendererState::default();
        renderer.refresh_outputs(vec!["DP-1".to_string()]);

        renderer.enqueue_apply(
            Some("DP-1".to_string()),
            PathBuf::from("/tmp/wall.png"),
            ScaleMode::Fill,
        );
        renderer
            .apply_pending()
            .expect("renderer apply should succeed in test mode");
        assert_eq!(renderer.committed_output_count(), 1);

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
        assert_eq!(renderer.committed_output_count(), 1);
    }

    #[test]
    fn flush_failure_requeues_processed_commands() {
        let mut renderer = RendererState::default();
        renderer.refresh_outputs(vec!["DP-1".to_string()]);
        renderer.enqueue_clear();
        renderer.inject_invalid_commit_plan("DP-1");

        let err = renderer
            .apply_pending()
            .expect_err("flush failure should bubble up");
        assert!(err.to_string().contains("wl_shm bridge flush failed"));
        assert_eq!(renderer.pending_command_count(), 1);
        assert_eq!(renderer.backend_assignment_count(), 0);
    }
}
