mod command_queue;
mod output_registry;

use std::path::PathBuf;
use tracing::info;
use vellum_ipc::ScaleMode;

pub(crate) use command_queue::RenderCommand;
use command_queue::RenderCommandQueue;
pub(crate) use output_registry::OutputRegistry;

#[derive(Default)]
pub(crate) struct RendererState {
    outputs: OutputRegistry,
    queue: RenderCommandQueue,
}

impl RendererState {
    pub(crate) fn refresh_outputs(&mut self, output_names: Vec<String>) {
        self.outputs.update(output_names);
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

    pub(crate) fn apply_pending(&mut self) {
        // This is a scaffold stage: commands are queued and logged while the
        // concrete SCTK/layer-shell renderer backend is being implemented.
        for command in self.queue.drain() {
            match command {
                RenderCommand::ApplyAssignment {
                    monitor,
                    path,
                    mode,
                } => {
                    if let Some(ref name) = monitor {
                        if !self.outputs.contains(name) {
                            info!(target = ?name, path = %path.display(), ?mode, "queued apply for currently unknown output");
                        }
                    }
                    info!(target = ?monitor, path = %path.display(), ?mode, "renderer scaffold accepted apply command");
                }
                RenderCommand::ClearAssignments => {
                    info!("renderer scaffold accepted clear command");
                }
            }
        }
    }

    pub(crate) fn output_names(&self) -> Vec<String> {
        self.outputs.names()
    }
}
