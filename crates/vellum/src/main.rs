//! `vellum`: the CLI (and later the TUI and renderer) for the Vellum Daemon.

mod client;
mod decode;
mod frame;
mod monitors;
mod render;
mod restore;
mod set;
mod status;

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
    /// Show the Daemon, its memory use, and each monitor's wallpaper.
    Status,
    /// Stop the Daemon.
    Kill,
    /// Render a frame for the Daemon. Run by vellumd, not by hand.
    #[command(hide = true)]
    Render {
        /// Put the wallpaper State remembers back on the monitor.
        #[arg(long, required = true)]
        restore: bool,
        /// The monitor to render for.
        #[arg(short, long, value_name = "NAME")]
        monitor: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Set { file, monitor, fit } => set::run(&file, monitor.as_deref(), fit),
        Command::Monitors => monitors::run(),
        Command::Status => status::run(),
        Command::Kill => client::kill(),
        Command::Render { restore: _, monitor } => restore::run(&monitor),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vellum: {err}");
            ExitCode::FAILURE
        }
    }
}
