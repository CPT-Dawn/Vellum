use serde::{Deserialize, Serialize};

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
    ClearAssignments,
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
