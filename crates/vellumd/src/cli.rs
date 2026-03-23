use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "vellumd", about = "Vellum wallpaper daemon")]
pub(crate) struct Args {
    #[arg(long, value_name = "PATH")]
    pub(crate) socket: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    pub(crate) state_file: Option<PathBuf>,
}
