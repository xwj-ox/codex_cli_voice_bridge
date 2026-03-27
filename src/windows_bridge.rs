#![cfg(target_os = "windows")]

pub use crate::platform::windows::{
    KeyboardPttController, get_foreground_window_info, is_usable_target_window, parse_ptt_key,
    paste_text_to_window, play_cue,
};
pub use crate::platform::{CueKind, PttActivation, WindowInfo};
