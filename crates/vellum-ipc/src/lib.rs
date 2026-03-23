use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const IPC_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum Request {
    Ping,
    SetWallpaper {
        path: String,
        monitor: Option<String>,
        mode: ScaleMode,
    },
    GetMonitors,
    GetAssignments,
    KillDaemon,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum Response {
    Pong,
    Ok,
    Monitors { names: Vec<String> },
    Assignments { entries: Vec<AssignmentEntry> },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssignmentEntry {
    pub monitor: Option<String>,
    pub path: String,
    pub mode: ScaleMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScaleMode {
    Fit,
    Fill,
    Crop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestEnvelope {
    pub version: u16,
    pub request: Request,
}

impl RequestEnvelope {
    pub fn new(request: Request) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn validate_version(&self) -> Result<(), IpcProtocolError> {
        if self.version == IPC_PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(IpcProtocolError::UnsupportedVersion(self.version))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseEnvelope {
    pub version: u16,
    pub response: Response,
}

impl ResponseEnvelope {
    pub fn new(response: Response) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION,
            response,
        }
    }

    pub fn validate_version(&self) -> Result<(), IpcProtocolError> {
        if self.version == IPC_PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(IpcProtocolError::UnsupportedVersion(self.version))
        }
    }
}

#[derive(Debug, Error)]
pub enum IpcProtocolError {
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u16),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_envelope_roundtrip_json() {
        let env = RequestEnvelope::new(Request::Ping);
        let json = serde_json::to_string(&env).expect("request envelope should serialize");
        let decoded: RequestEnvelope =
            serde_json::from_str(&json).expect("request envelope should deserialize");

        assert_eq!(decoded, env);
        assert!(decoded.validate_version().is_ok());
    }

    #[test]
    fn response_envelope_roundtrip_json() {
        let env = ResponseEnvelope::new(Response::Assignments {
            entries: vec![AssignmentEntry {
                monitor: Some("DP-1".to_string()),
                path: "/tmp/wallpapers/arch.png".to_string(),
                mode: ScaleMode::Fill,
            }],
        });
        let json = serde_json::to_string(&env).expect("response envelope should serialize");
        let decoded: ResponseEnvelope =
            serde_json::from_str(&json).expect("response envelope should deserialize");

        assert_eq!(decoded, env);
        assert!(decoded.validate_version().is_ok());
    }

    #[test]
    fn version_validation_fails_for_unsupported_version() {
        let env = RequestEnvelope {
            version: IPC_PROTOCOL_VERSION + 1,
            request: Request::Ping,
        };

        let err = env
            .validate_version()
            .expect_err("unsupported version should error");
        assert!(matches!(
            err,
            IpcProtocolError::UnsupportedVersion(version) if version == IPC_PROTOCOL_VERSION + 1
        ));
    }

    #[test]
    fn set_wallpaper_request_roundtrip_includes_mode() {
        let env = RequestEnvelope::new(Request::SetWallpaper {
            path: "/tmp/wallpapers/city.jpg".to_string(),
            monitor: Some("HDMI-A-1".to_string()),
            mode: ScaleMode::Crop,
        });

        let json = serde_json::to_string(&env).expect("set wallpaper request should serialize");
        let decoded: RequestEnvelope =
            serde_json::from_str(&json).expect("set wallpaper request should deserialize");

        assert_eq!(decoded, env);
    }

    #[test]
    fn scale_mode_serializes_lowercase() {
        let mode = ScaleMode::Fill;
        let json = serde_json::to_string(&mode).expect("scale mode should serialize");
        assert_eq!(json, "\"fill\"");
    }
}
