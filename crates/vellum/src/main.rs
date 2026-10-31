//! `vellum`: the CLI (and later the TUI and renderer) for the Vellum Daemon.

mod client;
mod decode;
mod frame;
mod monitors;
mod render;
mod set;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::render::FitMode;

#[derive(Parser)]
#[command(
    version,
    about = "Wallpapers, weightless.",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Put a wallpaper on every monitor, or on one with -m.
    Set {
        /// The image file (JPEG, PNG or WebP).
        file: PathBuf,
        /// Only this monitor, by connector name (see `vellum monitors`).
        #[arg(short, long, value_name = "NAME")]
        monitor: Option<String>,
        /// How the wallpaper is sized to each monitor.
        #[arg(long, value_enum, default_value_t)]
        fit: FitMode,
    },
    /// List connected monitors with their size and scale.
    Monitors,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Set { file, monitor, fit } => set::run(&file, monitor.as_deref(), fit),
        Command::Monitors => monitors::run(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vellum: {err}");
            ExitCode::FAILURE
        }
    }
}
