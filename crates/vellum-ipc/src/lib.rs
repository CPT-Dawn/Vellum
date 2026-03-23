mod envelope;
mod protocol;

pub use envelope::{IpcProtocolError, RequestEnvelope, ResponseEnvelope, IPC_PROTOCOL_VERSION};
pub use protocol::{AssignmentEntry, Request, Response, ScaleMode};

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
    fn clear_assignments_request_roundtrip() {
        let env = RequestEnvelope::new(Request::ClearAssignments);
        let json = serde_json::to_string(&env).expect("clear request should serialize");
        let decoded: RequestEnvelope =
            serde_json::from_str(&json).expect("clear request should deserialize");

        assert_eq!(decoded, env);
    }

    #[test]
    fn scale_mode_serializes_lowercase() {
        let mode = ScaleMode::Fill;
        let json = serde_json::to_string(&mode).expect("scale mode should serialize");
        assert_eq!(json, "\"fill\"");
    }
}
