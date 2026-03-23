use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Request, Response};

pub const IPC_PROTOCOL_VERSION: u16 = 1;

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
