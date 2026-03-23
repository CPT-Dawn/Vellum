use std::collections::BTreeSet;

#[derive(Default)]
pub(crate) struct OutputRegistry {
    outputs: BTreeSet<String>,
}

impl OutputRegistry {
    pub(crate) fn update<I>(&mut self, output_names: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.outputs.clear();
        self.outputs.extend(output_names);
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
        registry.update(vec!["DP-1".to_string(), "HDMI-A-1".to_string()]);
        assert!(registry.contains("DP-1"));

        registry.update(vec!["eDP-1".to_string()]);
        assert!(!registry.contains("DP-1"));
        assert!(registry.contains("eDP-1"));
    }
}
