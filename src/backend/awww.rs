//! Async command wrappers for `awww` and `awww-daemon`.

use std::{path::Path, process::Stdio};

use tokio::process::Command;

use crate::backend::BackendError;

/// Transition family supported by `awww`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    /// Cross-fade transition.
    Fade,
    /// Directional wipe transition.
    Wipe,
    /// Scale-in growth transition.
    Grow,
}

impl TransitionKind {
    /// Returns CLI-compatible transition token.
    #[must_use]
    pub fn as_cli_value(self) -> &'static str {
        match self {
            Self::Fade => "fade",
            Self::Wipe => "wipe",
            Self::Grow => "grow",
        }
    }
}

/// Transition parameters used when applying wallpapers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionSettings {
    /// Transition style.
    pub kind: TransitionKind,
    /// Step size accepted by `awww --transition-step`.
    pub step: u16,
    /// Frames per second for transition playback.
    pub fps: u16,
}

impl Default for TransitionSettings {
    /// Returns a balanced default transition profile.
    fn default() -> Self {
        Self {
            kind: TransitionKind::Fade,
            step: 16,
            fps: 60,
        }
    }
}

/// Typed request for setting wallpapers with optional monitor targeting.
#[derive(Debug)]
pub struct ApplyRequest<'a> {
    /// Absolute path to image file.
    pub image_path: &'a Path,
    /// Target monitor names; empty list means all monitors.
    pub outputs: &'a [String],
    /// Transition configuration for this apply action.
    pub transition: TransitionSettings,
}

/// Small async client wrapper around `awww` and `awww-daemon` commands.
#[derive(Debug, Clone)]
pub struct AwwwClient {
    /// Path or command name for `awww`.
    awww_program: &'static str,
    /// Path or command name for `awww-daemon`.
    daemon_program: &'static str,
}

impl Default for AwwwClient {
    /// Constructs a client using PATH-resolved binaries.
    fn default() -> Self {
        Self {
            awww_program: "awww",
            daemon_program: "awww-daemon",
        }
    }
}

impl AwwwClient {
    /// Starts `awww-daemon` and returns once startup exits successfully.
    pub async fn start_daemon(&self) -> Result<(), BackendError> {
        run_checked(self.daemon_program, std::iter::empty::<&str>()).await
    }

    /// Applies a wallpaper with transition and optional output targeting.
    pub async fn apply_wallpaper(&self, request: &ApplyRequest<'_>) -> Result<(), BackendError> {
        let image = request
            .image_path
            .to_str()
            .ok_or(BackendError::ParseError("image path must be valid utf-8"))?;

        let mut args = Vec::with_capacity(10 + request.outputs.len() * 2);
        args.push("img".to_owned());
        args.push(image.to_owned());
        args.push("--transition-type".to_owned());
        args.push(request.transition.kind.as_cli_value().to_owned());
        args.push("--transition-step".to_owned());
        args.push(request.transition.step.to_string());
        args.push("--transition-fps".to_owned());
        args.push(request.transition.fps.to_string());

        for output in request.outputs {
            args.push("--output".to_owned());
            args.push(output.clone());
        }

        run_checked(self.awww_program, args.iter().map(String::as_str)).await
    }

    /// Clears wallpaper assignment for one output or all outputs when `None`.
    pub async fn clear_wallpaper(&self, output: Option<&str>) -> Result<(), BackendError> {
        if let Some(output_name) = output {
            run_checked(self.awww_program, ["clear", "--output", output_name]).await
        } else {
            run_checked(self.awww_program, ["clear"]).await
        }
    }
}

/// Runs a command, capturing stderr and mapping non-zero status to `BackendError`.
async fn run_checked<I, S>(program: &'static str, args: I) -> Result<(), BackendError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut command = Command::new(program);
    for arg in args {
        command.arg(arg.as_ref());
    }

    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if output.status.success() {
        return Ok(());
    }

    Err(BackendError::CommandFailed {
        program,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}
