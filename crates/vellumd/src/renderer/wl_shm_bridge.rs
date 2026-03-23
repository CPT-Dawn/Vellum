use std::collections::{BTreeMap, VecDeque};

use anyhow::{bail, Result};

use crate::renderer::native_commit::NativeCommitPlan;

trait WlShmCommitExecutor {
    fn commit(&mut self, commit: &NativeCommitPlan) -> Result<()>;
}

#[derive(Debug, Default)]
struct SimulatedCommitExecutor {
    serial: u64,
    last_applied_serial: BTreeMap<String, u64>,
}

impl WlShmCommitExecutor for SimulatedCommitExecutor {
    fn commit(&mut self, commit: &NativeCommitPlan) -> Result<()> {
        if commit.width == 0 || commit.height == 0 {
            bail!(
                "invalid native commit dimensions for output {}: {}x{}",
                commit.output,
                commit.width,
                commit.height
            );
        }

        let min_stride = commit.width as usize * 4;
        if commit.stride < min_stride {
            bail!(
                "invalid native commit stride for output {}: got {}, need at least {}",
                commit.output,
                commit.stride,
                min_stride
            );
        }

        self.serial = self.serial.saturating_add(1);
        self.last_applied_serial
            .insert(commit.output.clone(), self.serial);
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(crate) struct WlShmBridge {
    pending: VecDeque<NativeCommitPlan>,
    last_committed: BTreeMap<String, NativeCommitPlan>,
    executor: SimulatedCommitExecutor,
}

impl WlShmBridge {
    pub(crate) fn submit<I>(&mut self, commits: I)
    where
        I: IntoIterator<Item = NativeCommitPlan>,
    {
        self.pending.extend(commits);
    }

    pub(crate) fn flush(&mut self) -> Result<usize> {
        let mut committed = 0usize;
        while let Some(commit) = self.pending.front().cloned() {
            self.executor.commit(&commit)?;
            self.pending.pop_front();
            self.last_committed.insert(commit.output.clone(), commit);
            committed = committed.saturating_add(1);
        }

        Ok(committed)
    }

    pub(crate) fn committed_output_count(&self) -> usize {
        self.last_committed.len()
    }

    #[cfg(test)]
    pub(crate) fn last_applied_serial_for(&self, output: &str) -> Option<u64> {
        self.executor.last_applied_serial.get(output).copied()
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
        let committed = bridge.flush().expect("flush should succeed");

        assert_eq!(committed, 3);
        assert_eq!(bridge.pending_count(), 0);
        assert_eq!(bridge.committed_output_count(), 2);
        assert_eq!(bridge.last_buffer_for("DP-1"), Some(3));
        assert_eq!(bridge.last_buffer_for("HDMI-A-1"), Some(4));
        assert_eq!(bridge.last_applied_serial_for("DP-1"), Some(2));
        assert_eq!(bridge.last_applied_serial_for("HDMI-A-1"), Some(3));
    }

    #[test]
    fn flush_keeps_invalid_commit_pending_and_returns_error() {
        let mut bridge = WlShmBridge::default();
        bridge.submit([
            NativeCommitPlan {
                output: "DP-1".to_string(),
                width: 1920,
                height: 1080,
                stride: 7680,
                buffer_id: 10,
                source_path: PathBuf::from("/tmp/ok.png"),
                mode: ScaleMode::Fit,
            },
            NativeCommitPlan {
                output: "DP-1".to_string(),
                width: 1920,
                height: 1080,
                stride: 64,
                buffer_id: 11,
                source_path: PathBuf::from("/tmp/bad.png"),
                mode: ScaleMode::Fill,
            },
        ]);

        let err = bridge
            .flush()
            .expect_err("flush should fail on invalid stride");
        assert!(err.to_string().contains("invalid native commit stride"));
        assert_eq!(bridge.pending_count(), 1);
        assert_eq!(bridge.committed_output_count(), 1);
        assert_eq!(bridge.last_buffer_for("DP-1"), Some(10));
    }
}
