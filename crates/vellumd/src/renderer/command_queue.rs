use std::collections::VecDeque;
use std::path::PathBuf;
use vellum_ipc::ScaleMode;

#[derive(Debug, Clone)]
pub(crate) enum RenderCommand {
    ApplyAssignment {
        monitor: Option<String>,
        path: PathBuf,
        mode: ScaleMode,
    },
    ClearAssignments,
}

#[derive(Default)]
pub(crate) struct RenderCommandQueue {
    queue: VecDeque<RenderCommand>,
}

impl RenderCommandQueue {
    pub(crate) fn push(&mut self, command: RenderCommand) {
        self.queue.push_back(command);
    }

    pub(crate) fn drain(&mut self) -> Vec<RenderCommand> {
        self.queue.drain(..).collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderCommand, RenderCommandQueue};

    #[test]
    fn queue_push_and_drain() {
        let mut queue = RenderCommandQueue::default();
        queue.push(RenderCommand::ClearAssignments);
        assert_eq!(queue.len(), 1);

        let drained = queue.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(queue.len(), 0);
    }
}
