use std::path::PathBuf;
use vellum_ipc::ScaleMode;

#[derive(Debug, Clone)]
pub(crate) struct NativeCommitPlan {
    pub(crate) output: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stride: usize,
    pub(crate) buffer_id: u64,
    pub(crate) source_path: PathBuf,
    pub(crate) mode: ScaleMode,
}
