//! `vellum`: the CLI (and later the TUI and renderer) for the Vellum Daemon.

mod decode;
mod frame;
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
    /// Put a wallpaper on every monitor.
    Set {
        /// The image file (JPEG, PNG or WebP).
        file: PathBuf,
        /// How the wallpaper is sized to each monitor.
        #[arg(long, value_enum, default_value_t)]
        fit: FitMode,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Set { file, fit } => set::run(&file, fit),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vellum: {err}");
            ExitCode::FAILURE
        }
    }
}
