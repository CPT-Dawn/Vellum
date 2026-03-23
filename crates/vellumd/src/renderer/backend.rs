use std::collections::HashMap;
use std::path::PathBuf;
use vellum_ipc::ScaleMode;

use crate::renderer::command_queue::RenderCommand;

#[derive(Default)]
pub(crate) struct BackendState {
    assignments: HashMap<Option<String>, (PathBuf, ScaleMode)>,
}

impl BackendState {
    pub(crate) fn apply_command(&mut self, command: RenderCommand) {
        match command {
            RenderCommand::ApplyAssignment {
                monitor,
                path,
                mode,
            } => {
                self.assignments.insert(monitor, (path, mode));
            }
            RenderCommand::ClearAssignments => {
                self.assignments.clear();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn assignment_count(&self) -> usize {
        self.assignments.len()
    }

    #[cfg(test)]
    pub(crate) fn assignment_mode_for(&self, monitor: Option<&str>) -> Option<ScaleMode> {
        let key = monitor.map(str::to_string);
        self.assignments.get(&key).map(|(_, mode)| *mode)
    }

    #[cfg(test)]
    pub(crate) fn assignment_path_for(&self, monitor: Option<&str>) -> Option<PathBuf> {
        let key = monitor.map(str::to_string);
        self.assignments.get(&key).map(|(path, _)| path.clone())
    }
}
