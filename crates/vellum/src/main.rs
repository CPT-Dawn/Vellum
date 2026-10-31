//! `vellum`: the CLI (and later the TUI and renderer) for the Vellum Daemon.

mod client;
mod decode;
mod frame;
mod monitors;
mod render;
mod set;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use vellum_ipc::{Direction, Transition, TransitionKind};

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
        #[command(flatten)]
        transition: TransitionArgs,
    },
    /// List connected monitors with their size and scale.
    Monitors,
}

/// Per-command Transition overrides.
#[derive(Args)]
struct TransitionArgs {
    /// The Transition to play [default: fade]
    #[arg(long, value_enum, value_name = "KIND")]
    transition: Option<KindArg>,
    /// Which way a Wipe or Slide moves [default: right]
    #[arg(long, value_enum)]
    direction: Option<DirectionArg>,
    /// Transition length in milliseconds [default: 600]
    #[arg(long, value_name = "MS")]
    duration: Option<u32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Fade,
    Wipe,
    Slide,
    None,
}

#[derive(Clone, Copy, ValueEnum)]
enum DirectionArg {
    Left,
    Right,
    Up,
    Down,
}

impl TransitionArgs {
    /// The defaults with this command's overrides applied.
    fn resolve(&self) -> Transition {
        let mut t = Transition::default();
        if let Some(kind) = self.transition {
            t.kind = match kind {
                KindArg::Fade => TransitionKind::Fade,
                KindArg::Wipe => TransitionKind::Wipe,
                KindArg::Slide => TransitionKind::Slide,
                KindArg::None => TransitionKind::None,
            };
        }
        if let Some(direction) = self.direction {
            t.direction = match direction {
                DirectionArg::Left => Direction::Left,
                DirectionArg::Right => Direction::Right,
                DirectionArg::Up => Direction::Up,
                DirectionArg::Down => Direction::Down,
            };
        }
        if let Some(ms) = self.duration {
            t.duration_ms = ms;
        }
        t
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Set {
            file,
            monitor,
            fit,
            transition,
        } => set::run(&file, monitor.as_deref(), fit, transition.resolve()),
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
