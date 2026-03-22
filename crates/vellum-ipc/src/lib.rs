use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const IPC_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum Request {
    Ping,
    SetWallpaper { path: String },
    GetMonitors,
    KillDaemon,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum Response {
    Pong,
    Ok,
    Monitors { names: Vec<String> },
    Error { message: String },
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
        let env = ResponseEnvelope::new(Response::Monitors {
            names: vec!["DP-1".to_string(), "HDMI-A-1".to_string()],
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
}
