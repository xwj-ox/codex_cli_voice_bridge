pub mod factory;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod traits;
pub mod types;
#[cfg(target_os = "windows")]
pub mod windows;

pub use factory::{PlatformInitOptions, PlatformServices, create_platform_services};
pub use traits::{CuePlayer, PttController, TextInjector, WindowService};
pub use types::{CueKind, PttActivation, WindowInfo};
