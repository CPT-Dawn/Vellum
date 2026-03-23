use std::collections::BTreeSet;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct OutputDelta {
    pub(crate) added: Vec<String>,
    pub(crate) removed: Vec<String>,
}

impl OutputDelta {
    pub(crate) fn changed(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct OutputRegistry {
    outputs: BTreeSet<String>,
}

impl OutputRegistry {
    pub(crate) fn reconcile<I>(&mut self, output_names: I) -> OutputDelta
    where
        I: IntoIterator<Item = String>,
    {
        let next: BTreeSet<String> = output_names.into_iter().collect();

        let removed: Vec<String> = self.outputs.difference(&next).cloned().collect();
        let added: Vec<String> = next.difference(&self.outputs).cloned().collect();

        self.outputs = next;

        OutputDelta { added, removed }
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.outputs.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::OutputRegistry;

    #[test]
    fn update_replaces_registry_contents() {
        let mut registry = OutputRegistry::default();
        let delta = registry.reconcile(vec!["DP-1".to_string(), "HDMI-A-1".to_string()]);
        assert!(delta.changed());
        assert_eq!(
            delta.added,
            vec!["DP-1".to_string(), "HDMI-A-1".to_string()]
        );
        assert!(delta.removed.is_empty());
        assert!(registry.contains("DP-1"));

        let delta = registry.reconcile(vec!["eDP-1".to_string()]);
        assert!(delta.changed());
        assert_eq!(delta.added, vec!["eDP-1".to_string()]);
        assert_eq!(
            delta.removed,
            vec!["DP-1".to_string(), "HDMI-A-1".to_string()]
        );
        assert!(!registry.contains("DP-1"));
        assert!(registry.contains("eDP-1"));
    }

    #[test]
    fn reconcile_reports_no_change_for_identical_snapshot() {
        let mut registry = OutputRegistry::default();
        let _ = registry.reconcile(vec!["DP-1".to_string(), "HDMI-A-1".to_string()]);

        let delta = registry.reconcile(vec!["HDMI-A-1".to_string(), "DP-1".to_string()]);
        assert!(!delta.changed());
        assert!(delta.added.is_empty());
        assert!(delta.removed.is_empty());
    }
}
