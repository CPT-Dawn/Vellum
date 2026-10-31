//! `vellumd`: the Vellum Daemon. Keeps wallpapers on every Monitor, holds no
//! image decoders and no pixels (ADR 0003).

mod daemon;
mod ipc;
mod wayland;

#[allow(warnings, clippy::all, clippy::pedantic)]
mod protocols {
    include!(concat!(env!("OUT_DIR"), "/protocols.rs"));
}

use std::process::ExitCode;

const USAGE: &str = "\
vellumd: the Vellum wallpaper Daemon

Usage: vellumd [--version | --help]

Keeps wallpapers on screen. Control it with `vellum`.";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("-V" | "--version") => {
            println!("vellumd {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("vellumd: unexpected argument '{other}'\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vellumd: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let socket = vellum_ipc::socket_path_from_env().map_err(|err| err.to_string())?;
    // Refuse early if another Daemon owns this display.
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        return Err(format!(
            "another vellumd is already running on this display ({})",
            socket.display()
        ));
    }
    let wayland = wayland::Wayland::connect()?;
    // Bind only once every Monitor is configured: a socket that answers
    // means the Daemon is ready to present.
    let listener = ipc::Listener::bind(&socket)?;
    daemon::Daemon::new(wayland, listener).run()
}
