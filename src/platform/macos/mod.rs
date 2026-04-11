pub mod cue;
pub mod paste;
pub mod permissions;
pub mod ptt;
pub mod window;

pub use cue::play_cue;
pub use paste::paste_text_to_window;
pub use permissions::{
    PermissionStatus, ensure_runtime_permissions, host_permission_hint, missing_permission_names,
    permission_failure_message, preflight_permissions,
};
pub use ptt::MacosPttController;
pub use window::{MacosHostContext, get_frontmost_window_info, is_usable_target_window};
