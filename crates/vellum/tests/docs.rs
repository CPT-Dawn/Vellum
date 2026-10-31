//! Shell completions and the man page, generated from the CLI definition.
//! No Daemon or compositor needed.

use std::process::{Command, Output};

fn vellum(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vellum"))
        .args(args)
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap()
}

fn stdout_of(args: &[&str]) -> String {
    let out = vellum(args);
    assert!(
        out.status.success(),
        "vellum {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Visible subcommands every completion script and the man page must know.
const SUBCOMMANDS: &[&str] = &["set", "monitors", "status", "kill", "completions"];

#[test]
fn completions_cover_bash_zsh_and_fish() {
    for (shell, marker, fit) in [
        ("bash", "complete -F", "--fit"),
        ("zsh", "#compdef vellum", "--fit"),
        ("fish", "complete -c vellum", "-l fit"),
    ] {
        let script = stdout_of(&["completions", shell]);
        assert!(script.contains(marker), "{shell} script lacks {marker:?}");
        for sub in SUBCOMMANDS {
            assert!(script.contains(sub), "{shell} script lacks {sub}");
        }
        assert!(script.contains(fit), "{shell} script lacks {fit:?}");
        assert!(
            !script.contains("restore"),
            "{shell} script leaks the hidden render command"
        );
    }
}

#[test]
fn bash_and_zsh_completions_parse() {
    for shell in ["bash", "zsh"] {
        let script = stdout_of(&["completions", shell]);
        let Ok(out) = Command::new(shell).args(["-n", "-c", &script]).output() else {
            continue; // shell not installed
        };
        assert!(
            out.status.success(),
            "{shell} -n rejects the script: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn unknown_shell_is_rejected() {
    let out = vellum(&["completions", "tcsh"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bash") && err.contains("zsh") && err.contains("fish"));
}

#[test]
fn man_pages_cover_every_visible_subcommand() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("man1");
    stdout_of(&["man", out.to_str().unwrap()]);

    let mut names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    let mut want: Vec<String> = SUBCOMMANDS
        .iter()
        .map(|s| format!("vellum-{s}.1"))
        .chain(["vellum.1".to_owned()])
        .collect();
    want.sort();
    assert_eq!(names, want, "hidden commands must not get a page");

    let page = std::fs::read_to_string(out.join("vellum.1")).unwrap();
    assert!(page.contains(".TH vellum 1"), "not roff");
    assert!(page.contains("Wallpapers, weightless."));
    for sub in SUBCOMMANDS {
        assert!(
            page.contains(&format!("vellum\\-{sub}(1)")),
            "vellum.1 lacks {sub}"
        );
    }
    let set = std::fs::read_to_string(out.join("vellum-set.1")).unwrap();
    assert!(set.contains("fit"), "vellum-set.1 lacks --fit");
}

#[test]
fn man_is_hidden_from_help() {
    let help = stdout_of(&["--help"]);
    assert!(help.contains("completions"));
    assert!(!help.lines().any(|l| l.trim_start().starts_with("man ")));
}
