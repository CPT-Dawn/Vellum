use anyhow::Result;
use serde_json::Value;
use std::process::Command as ProcessCommand;

pub(crate) fn detect_monitor_names() -> Result<Vec<String>> {
    if let Some(monitors) = detect_hyprland_monitors() {
        return Ok(monitors);
    }

    if let Some(monitors) = detect_wlr_randr_monitors() {
        return Ok(monitors);
    }

    anyhow::bail!("unable to detect monitors from hyprctl or wlr-randr")
}

fn detect_hyprland_monitors() -> Option<Vec<String>> {
    let value = run_json_command("hyprctl", &["monitors", "-j"])?;
    let items = value.as_array()?;
    let mut names = Vec::new();
    for item in items {
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            names.push(name.to_string());
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn detect_wlr_randr_monitors() -> Option<Vec<String>> {
    let value = run_json_command("wlr-randr", &["--json"])?;
    let items = value.as_array()?;
    let mut names = Vec::new();
    for item in items {
        if let Some(name) = item
            .get("name")
            .or_else(|| item.get("output"))
            .and_then(Value::as_str)
        {
            names.push(name.to_string());
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn run_json_command(command: &str, args: &[&str]) -> Option<Value> {
    let output = ProcessCommand::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    serde_json::from_str::<Value>(&stdout).ok()
}
