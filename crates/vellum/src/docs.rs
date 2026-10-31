//! Shell completions and the man page, both generated from the clap
//! definition so they never drift from the real CLI.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use clap::{Command, ValueEnum};
use clap_complete::Shell as Generator;

/// The shells `vellum completions` writes scripts for.
#[derive(Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

/// Writes the completion script for `shell` to stdout.
pub fn completions(cmd: Command, shell: Shell) -> Result<(), String> {
    let generator = match shell {
        Shell::Bash => Generator::Bash,
        Shell::Zsh => Generator::Zsh,
        Shell::Fish => Generator::Fish,
    };
    let mut out = Vec::new();
    clap_complete::generate(generator, &mut without_hidden(cmd), "vellum", &mut out);
    write_stdout(&out)
}

/// `cmd` minus its hidden subcommands. The bash and zsh generators would
/// otherwise offer internals like `render --restore` on Tab.
fn without_hidden(cmd: Command) -> Command {
    let mut out = Command::new("vellum")
        .version(env!("CARGO_PKG_VERSION"))
        .args(cmd.get_arguments().cloned())
        .subcommands(cmd.get_subcommands().filter(|s| !s.is_hide_set()).cloned());
    if let Some(about) = cmd.get_about() {
        out = out.about(about.clone());
    }
    out
}

/// Writes `vellum.1` plus one page per visible subcommand into `dir`.
pub fn man(cmd: Command, dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    clap_mangen::generate_to(cmd, dir)
        .map_err(|e| format!("cannot write man pages to {}: {e}", dir.display()))
}

fn write_stdout(bytes: &[u8]) -> Result<(), String> {
    io::stdout()
        .lock()
        .write_all(bytes)
        .map_err(|e| format!("cannot write to stdout: {e}"))
}
