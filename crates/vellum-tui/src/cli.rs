use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use vellum_ipc::ScaleMode;

#[derive(Debug, Parser)]
#[command(name = "vellum-tui", about = "Vellum terminal client")]
pub(crate) struct Args {
    #[arg(long, value_name = "PATH")]
    pub(crate) socket: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    pub(crate) images_dir: Option<PathBuf>,

    #[arg(long, value_name = "WIDTH")]
    pub(crate) monitor_width: Option<u32>,

    #[arg(long, value_name = "HEIGHT")]
    pub(crate) monitor_height: Option<u32>,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Ui,
    Ping,
    Set {
        #[arg(value_name = "PATH")]
        path: PathBuf,

        #[arg(long, value_name = "NAME")]
        monitor: Option<String>,

        #[arg(long, value_enum, default_value_t = CliScaleMode::Fit)]
        mode: CliScaleMode,
    },
    Monitors,
    Assignments,
    Clear,
    Kill,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum CliScaleMode {
    Fit,
    Fill,
    Crop,
}

impl From<CliScaleMode> for ScaleMode {
    fn from(value: CliScaleMode) -> Self {
        match value {
            CliScaleMode::Fit => ScaleMode::Fit,
            CliScaleMode::Fill => ScaleMode::Fill,
            CliScaleMode::Crop => ScaleMode::Crop,
        }
    }
}
