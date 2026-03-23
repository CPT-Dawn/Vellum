use std::collections::{BTreeMap, VecDeque};

use crate::renderer::native_commit::NativeCommitPlan;

#[derive(Debug, Default)]
pub(crate) struct WlShmBridge {
    pending: VecDeque<NativeCommitPlan>,
    last_committed: BTreeMap<String, NativeCommitPlan>,
}

impl WlShmBridge {
    pub(crate) fn submit<I>(&mut self, commits: I)
    where
        I: IntoIterator<Item = NativeCommitPlan>,
    {
        self.pending.extend(commits);
    }

    pub(crate) fn flush(&mut self) {
        while let Some(commit) = self.pending.pop_front() {
            self.last_committed.insert(commit.output.clone(), commit);
        }
    }

    #[cfg(test)]
    pub(crate) fn committed_output_count(&self) -> usize {
        self.last_committed.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub(crate) fn last_buffer_for(&self, output: &str) -> Option<u64> {
        self.last_committed
            .get(output)
            .map(|commit| commit.buffer_id)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use vellum_ipc::ScaleMode;

    use crate::renderer::native_commit::NativeCommitPlan;

    use super::WlShmBridge;

    #[test]
    fn submit_and_flush_tracks_latest_commit_per_output() {
        let mut bridge = WlShmBridge::default();
        bridge.submit([
            NativeCommitPlan {
                output: "DP-1".to_string(),
                width: 1920,
                height: 1080,
                stride: 7680,
                buffer_id: 2,
                source_path: PathBuf::from("/tmp/a.png"),
                mode: ScaleMode::Fit,
            },
            NativeCommitPlan {
                output: "DP-1".to_string(),
                width: 1920,
                height: 1080,
                stride: 7680,
                buffer_id: 3,
                source_path: PathBuf::from("/tmp/b.png"),
                mode: ScaleMode::Fill,
            },
            NativeCommitPlan {
                output: "HDMI-A-1".to_string(),
                width: 2560,
                height: 1440,
                stride: 10240,
                buffer_id: 4,
                source_path: PathBuf::from("/tmp/c.png"),
                mode: ScaleMode::Crop,
            },
        ]);

        assert_eq!(bridge.pending_count(), 3);
        bridge.flush();

        assert_eq!(bridge.pending_count(), 0);
        assert_eq!(bridge.committed_output_count(), 2);
        assert_eq!(bridge.last_buffer_for("DP-1"), Some(3));
        assert_eq!(bridge.last_buffer_for("HDMI-A-1"), Some(4));
    }
}
