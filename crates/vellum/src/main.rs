//! `vellum`: the TUI, the CLI and the renderer for the Vellum Daemon.

mod atomic;
mod client;
mod config;
mod deck;
mod decode;
mod docs;
mod favorites;
mod frame;
mod hook;
mod library;
mod monitors;
mod render;
mod restore;
mod rotation;
mod set;
mod status;
mod sync;
mod tui;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{ArgGroup, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use vellum_ipc::{Direction, Transition, TransitionKind};

use crate::render::FitMode;

#[derive(Parser)]
#[command(
    name = "vellum",
    version,
    about = "Wallpapers, weightless.",
    long_about = "Wallpapers, weightless.\n\nRun `vellum` with no command to open the TUI."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Put a wallpaper on every monitor, or on one with -m.
    Set {
        /// The image file (JPEG, PNG or WebP), or a name in the Library.
        file: PathBuf,
        /// Only this monitor, by connector name (see `vellum monitors`).
        #[arg(short, long, value_name = "NAME")]
        monitor: Option<String>,
        /// How the wallpaper is sized to each monitor [default: from the
        /// Config, else fill]
        #[arg(long, value_enum)]
        fit: Option<FitMode>,
        #[command(flatten)]
        transition: TransitionArgs,
    },
    /// Show the next wallpaper by the Rotation rules.
    Next(PickArgs),
    /// Go back to the wallpaper shown before.
    Prev(PickArgs),
    /// Show a random wallpaper from the Rotation's source.
    Random(PickArgs),
    /// Start, stop or inspect Rotation.
    Rotation {
        #[command(subcommand)]
        action: RotationAction,
    },
    /// Add, remove or list Favorites.
    Fav {
        #[command(subcommand)]
        action: FavAction,
    },
    /// List connected monitors with their size and scale.
    Monitors,
    /// Show the Daemon, its memory use, and each monitor's wallpaper.
    Status,
    /// Re-read the Config now and report any error in it.
    Reload,
    /// Stop the Daemon.
    Kill,
    /// Print a shell completion script, e.g. `vellum completions zsh`.
    Completions {
        #[arg(value_enum)]
        shell: docs::Shell,
    },
    /// Write the man pages (vellum.1, vellum-set.1, ...) into DIR. Used by packaging.
    #[command(hide = true)]
    Man {
        #[arg(value_name = "DIR")]
        dir: PathBuf,
    },
    /// Render a frame or push the Config for the Daemon. Run by vellumd,
    /// not by hand.
    #[command(hide = true, group(ArgGroup::new("mode").required(true).args(["file", "restore", "sync", "rotate"])))]
    Render {
        /// Present this image file on the monitor, like `vellum set FILE
        /// -m NAME` with the Config's Fit mode and Transition: State
        /// records it, the on_change hook runs and the monitor's Rotation
        /// timer restarts. Unlike `set`, no Library lookup and no Daemon
        /// auto-start.
        #[arg(requires = "monitor")]
        file: Option<PathBuf>,
        /// Put the wallpaper State remembers back on the monitor.
        #[arg(long, requires = "monitor")]
        restore: bool,
        /// Read the Config and push its settings to the Daemon.
        #[arg(long)]
        sync: bool,
        /// A Rotation tick: put the next wallpaper on each monitor.
        #[arg(long)]
        rotate: bool,
        /// The monitor to render for; --rotate takes several.
        #[arg(short, long, value_name = "NAME")]
        monitor: Vec<String>,
    },
}

/// The target and Transition of `next`, `prev` and `random`.
#[derive(Args)]
struct PickArgs {
    /// Only this monitor, by connector name (see `vellum monitors`).
    #[arg(short, long, value_name = "NAME")]
    monitor: Option<String>,
    #[command(flatten)]
    transition: TransitionArgs,
}

#[derive(Subcommand)]
enum RotationAction {
    /// Start changing wallpapers on a timer; stays on across reboots.
    Start,
    /// Stop Rotation.
    Stop,
    /// Show each monitor's source, order, interval and next change.
    Status,
}

#[derive(Subcommand)]
enum FavAction {
    /// Add a wallpaper to Favorites (default: the one on screen).
    Add {
        /// The image file, or a name in the Library.
        file: Option<PathBuf>,
        /// Without a file: the wallpaper on this monitor.
        #[arg(short, long, value_name = "NAME")]
        monitor: Option<String>,
    },
    /// Remove a wallpaper from Favorites (default: the one on screen).
    Remove {
        /// The image file, or a name in the Library.
        file: Option<PathBuf>,
        /// Without a file: the wallpaper on this monitor.
        #[arg(short, long, value_name = "NAME")]
        monitor: Option<String>,
    },
    /// List Favorites, skipping files that have gone missing.
    List,
}

impl PickArgs {
    fn run(self, pick: rotation::Pick) -> Result<(), String> {
        let config = config::load();
        let transition = self.transition.resolve(config.transition);
        let monitors: Vec<String> = self.monitor.into_iter().collect();
        rotation::run(pick, &monitors, Some(transition), rotation::Trigger::Manual)
    }
}

/// Per-command Transition overrides.
#[derive(Args)]
struct TransitionArgs {
    /// The Transition to play [default: from the Config, else fade]
    #[arg(long, value_enum, value_name = "KIND")]
    transition: Option<KindArg>,
    /// Which way a Wipe or Slide moves [default: from the Config, else right]
    #[arg(long, value_enum)]
    direction: Option<DirectionArg>,
    /// Transition length in milliseconds [default: from the Config, else 600]
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
    /// `defaults` (from the Config) with this command's overrides applied.
    fn resolve(&self, defaults: Transition) -> Transition {
        let mut t = defaults;
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
    let Some(command) = cli.command else {
        return finish(tui::run(config::load()));
    };
    let result = match command {
        Command::Set {
            file,
            monitor,
            fit,
            transition,
        } => {
            let config = config::load();
            let fit = fit.unwrap_or(config.fit);
            let transition = transition.resolve(config.transition);
            set::run(&file, monitor.as_deref(), &config, fit, transition)
        }
        Command::Next(args) => args.run(rotation::Pick::Next),
        Command::Prev(args) => args.run(rotation::Pick::Prev),
        Command::Random(args) => args.run(rotation::Pick::Random),
        Command::Rotation { action } => match action {
            RotationAction::Start => rotation::control(true),
            RotationAction::Stop => rotation::control(false),
            RotationAction::Status => rotation::status(),
        },
        Command::Fav { action } => match action {
            FavAction::Add { file, monitor } => favorites::add(file.as_deref(), monitor.as_deref()),
            FavAction::Remove { file, monitor } => {
                favorites::remove(file.as_deref(), monitor.as_deref())
            }
            FavAction::List => favorites::list(),
        },
        Command::Monitors => monitors::run(),
        Command::Status => status::run(),
        Command::Reload => client::daemon().and_then(|socket| sync::run(&socket)),
        Command::Kill => client::kill(),
        Command::Completions { shell } => docs::completions(Cli::command(), shell),
        Command::Man { dir } => docs::man(Cli::command(), &dir),
        // Never auto-starts a Daemon: the Daemon is what runs these.
        Command::Render { sync: true, .. } => {
            client::socket().and_then(|socket| sync::run(&socket))
        }
        Command::Render {
            rotate: true,
            monitor,
            ..
        } => rotation::run(
            rotation::Pick::Next,
            &monitor,
            None,
            rotation::Trigger::Tick,
        ),
        Command::Render {
            file: Some(file),
            monitor,
            ..
        } => match &monitor[..] {
            [monitor] => set::explicit(&file, monitor, &config::load()),
            _ => Err("render FILE takes exactly one --monitor".into()),
        },
        Command::Render { monitor, .. } => restore::run(&monitor[0]),
    };
    finish(result)
}

fn finish(result: Result<(), String>) -> ExitCode {
    // Let queued on_change hooks finish; their failures are only reported.
    hook::finish();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vellum: {err}");
            ExitCode::FAILURE
        }
    }
}
