//! `vellum`: the CLI (and later the TUI and renderer) for the Vellum Daemon.

mod frame;
mod render;
mod set;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use vellum_ipc::{Direction, Transition, TransitionKind};

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
        #[command(flatten)]
        transition: TransitionArgs,
    },
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
        Command::Set { file, transition } => set::run(&file, transition.resolve()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vellum: {err}");
            ExitCode::FAILURE
        }
    }
}
