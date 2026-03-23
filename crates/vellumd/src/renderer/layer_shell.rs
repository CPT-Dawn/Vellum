use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;
use tracing::info;
use vellum_ipc::ScaleMode;

#[derive(Debug, Default)]
pub(crate) struct LayerShellSession {
    known_outputs: BTreeSet<String>,
}

impl LayerShellSession {
    pub(crate) fn sync_outputs<I>(&mut self, outputs: I) -> Result<()>
    where
        I: IntoIterator<Item = String>,
    {
        self.known_outputs.clear();
        self.known_outputs.extend(outputs);
        info!(
            count = self.known_outputs.len(),
            "renderer layer-shell output snapshot updated"
        );
        Ok(())
    }

    pub(crate) fn apply_assignment(
        &mut self,
        monitor: Option<&str>,
        path: &Path,
        mode: ScaleMode,
    ) -> Result<()> {
        info!(
            monitor,
            path = %path.display(),
            ?mode,
            "renderer layer-shell scaffold apply requested"
        );
        Ok(())
    }

    pub(crate) fn clear_assignments(&mut self) -> Result<()> {
        info!("renderer layer-shell scaffold clear requested");
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn known_output_count(&self) -> usize {
        self.known_outputs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::LayerShellSession;

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
}
