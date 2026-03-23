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

    pub(crate) fn prepend<I>(&mut self, commands: I)
    where
        I: IntoIterator<Item = RenderCommand>,
    {
        let buffered: Vec<RenderCommand> = commands.into_iter().collect();
        for command in buffered.into_iter().rev() {
            self.queue.push_front(command);
        }
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

    #[test]
    fn queue_prepend_preserves_order() {
        let mut queue = RenderCommandQueue::default();
        queue.push(RenderCommand::ClearAssignments);
        queue.prepend([
            RenderCommand::ApplyAssignment {
                monitor: Some("DP-1".to_string()),
                path: "/tmp/a.png".into(),
                mode: vellum_ipc::ScaleMode::Fit,
            },
            RenderCommand::ApplyAssignment {
                monitor: Some("HDMI-A-1".to_string()),
                path: "/tmp/b.png".into(),
                mode: vellum_ipc::ScaleMode::Fill,
            },
        ]);

        let drained = queue.drain();
        assert_eq!(drained.len(), 3);

        match &drained[0] {
            RenderCommand::ApplyAssignment { monitor, .. } => {
                assert_eq!(monitor.as_deref(), Some("DP-1"));
            }
            _ => panic!("first command should be apply"),
        }
        match &drained[1] {
            RenderCommand::ApplyAssignment { monitor, .. } => {
                assert_eq!(monitor.as_deref(), Some("HDMI-A-1"));
            }
            _ => panic!("second command should be apply"),
        }
        assert!(matches!(drained[2], RenderCommand::ClearAssignments));
    }
}
