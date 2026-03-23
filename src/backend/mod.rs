//! Backend integrations for Wayland monitor discovery and wallpaper execution.

pub mod awww;
pub mod monitors;

use std::{error::Error, fmt, io};

/// Backend-level error type used by monitor and command wrapper modules.
#[derive(Debug)]
pub enum BackendError {
    /// Process execution failed at the OS layer.
    Io(io::Error),
    /// JSON deserialization failed for monitor data.
    Json(serde_json::Error),
    /// External command failed with a non-success exit status.
    CommandFailed {
        /// Command name that failed.
        program: &'static str,
        /// Human-readable stderr payload returned by the process.
        stderr: String,
    },
    /// No monitor backend succeeded on the host.
    MonitorBackendUnavailable,
    /// A parser could not interpret monitor output.
    ParseError(&'static str),
}

impl fmt::Display for BackendError {
    /// Formats an operator-friendly error string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Json(err) => write!(f, "JSON parse error: {err}"),
            Self::CommandFailed { program, stderr } => {
                write!(f, "command `{program}` failed: {stderr}")
            }
            Self::MonitorBackendUnavailable => {
                write!(
                    f,
                    "no supported monitor backend (hyprctl/wlr-randr) is available"
                )
            }
            Self::ParseError(message) => write!(f, "monitor parse error: {message}"),
        }
    }
}

impl Error for BackendError {
    /// Returns source errors for wrapped variants.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::CommandFailed { .. } | Self::MonitorBackendUnavailable | Self::ParseError(_) => {
                None
            }
        }
    }
}

impl From<io::Error> for BackendError {
    /// Converts an I/O error into a backend error.
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for BackendError {
    /// Converts a JSON parse error into a backend error.
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
