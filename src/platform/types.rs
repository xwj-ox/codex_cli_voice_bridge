use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub process_name: String,
}

#[derive(Debug, Clone, Copy)]
pub enum CueKind {
    Listen,
    Recognized,
    Pasted,
    Error,
}

#[derive(Debug)]
pub struct PttActivation {
    pub target_window: WindowInfo,
    pub stop_signal: Arc<AtomicBool>,
}
