use anyhow::Result;

use crate::platform::traits::{CuePlayer, PttController, TextInjector, WindowService};

pub struct PlatformInitOptions {
    pub ptt_key: String,
    pub ptt_hold_ms: u64,
    pub ptt_short_press_passthrough: bool,
    pub forbid_host_window_target: bool,
}

pub struct PlatformServices {
    pub ptt: Box<dyn PttController>,
    pub window_service: Box<dyn WindowService>,
    pub text_injector: Box<dyn TextInjector>,
    pub cue_player: Box<dyn CuePlayer>,
}

#[cfg(target_os = "windows")]
pub fn create_platform_services(options: &PlatformInitOptions) -> Result<PlatformServices> {
    use crate::platform::traits::PttActivationFuture;
    use crate::platform::types::{CueKind, WindowInfo};
    use crate::platform::windows::{
        KeyboardPttController, get_foreground_window_info, is_usable_target_window, parse_ptt_key,
        paste_text_to_window, play_cue,
    };

    struct WindowsPttController {
        inner: KeyboardPttController,
    }

    impl PttController for WindowsPttController {
        fn recv_activation(&mut self) -> PttActivationFuture<'_> {
            Box::pin(async move { self.inner.recv().await })
        }
    }

    struct WindowsWindowService {
        host_hwnd: Option<isize>,
    }

    impl WindowService for WindowsWindowService {
        fn current_target(&self) -> Option<WindowInfo> {
            get_foreground_window_info()
        }

        fn is_usable_target(
            &self,
            target: Option<&WindowInfo>,
            require_title: &str,
            forbid_host_window_target: bool,
        ) -> bool {
            let bridge_host_hwnd = if forbid_host_window_target {
                self.host_hwnd
            } else {
                None
            };
            is_usable_target_window(target, require_title, bridge_host_hwnd)
        }
    }

    struct WindowsTextInjector;

    impl TextInjector for WindowsTextInjector {
        fn paste_text(
            &self,
            target: &WindowInfo,
            text: &str,
            submit: bool,
            paste_delay_ms: u64,
        ) -> Result<()> {
            paste_text_to_window(text, target.hwnd, submit, paste_delay_ms)
        }
    }

    struct WindowsCuePlayer;

    impl CuePlayer for WindowsCuePlayer {
        fn play(&self, cue: CueKind) {
            play_cue(true, cue);
        }
    }

    let vk_code = parse_ptt_key(&options.ptt_key)?;
    let ptt = KeyboardPttController::new(
        vk_code,
        options.ptt_hold_ms,
        options.ptt_short_press_passthrough,
    )?;
    let host_hwnd = options
        .forbid_host_window_target
        .then(|| get_foreground_window_info().map(|window| window.hwnd))
        .flatten();

    Ok(PlatformServices {
        ptt: Box::new(WindowsPttController { inner: ptt }),
        window_service: Box::new(WindowsWindowService { host_hwnd }),
        text_injector: Box::new(WindowsTextInjector),
        cue_player: Box::new(WindowsCuePlayer),
    })
}

#[cfg(target_os = "macos")]
pub fn create_platform_services(_options: &PlatformInitOptions) -> Result<PlatformServices> {
    use crate::platform::macos::{
        MacosHostContext, MacosPttController, ensure_runtime_permissions,
        get_frontmost_window_info, is_usable_target_window, paste_text_to_window, play_cue,
    };
    use crate::platform::traits::PttActivationFuture;
    use crate::platform::types::{CueKind, WindowInfo};

    struct MacosPttWrapper {
        inner: MacosPttController,
    }

    impl PttController for MacosPttWrapper {
        fn recv_activation(&mut self) -> PttActivationFuture<'_> {
            Box::pin(async move { self.inner.recv().await })
        }
    }

    struct MacosWindowService {
        host_context: Option<MacosHostContext>,
    }

    impl WindowService for MacosWindowService {
        fn current_target(&self) -> Option<WindowInfo> {
            get_frontmost_window_info()
        }

        fn is_usable_target(
            &self,
            target: Option<&WindowInfo>,
            require_title: &str,
            forbid_host_window_target: bool,
        ) -> bool {
            is_usable_target_window(
                target,
                require_title,
                forbid_host_window_target,
                self.host_context.as_ref(),
            )
        }
    }

    struct MacosTextInjector;

    impl TextInjector for MacosTextInjector {
        fn paste_text(
            &self,
            target: &WindowInfo,
            text: &str,
            submit: bool,
            paste_delay_ms: u64,
        ) -> Result<()> {
            paste_text_to_window(target, text, submit, paste_delay_ms)
        }
    }

    struct MacosCuePlayer;

    impl CuePlayer for MacosCuePlayer {
        fn play(&self, cue: CueKind) {
            play_cue(true, cue);
        }
    }

    let _permission_status = ensure_runtime_permissions()?;
    let host_context = _options
        .forbid_host_window_target
        .then(MacosHostContext::detect);
    let ptt = MacosPttController::new(
        &_options.ptt_key,
        _options.ptt_hold_ms,
        _options.ptt_short_press_passthrough,
    )?;

    Ok(PlatformServices {
        ptt: Box::new(MacosPttWrapper { inner: ptt }),
        window_service: Box::new(MacosWindowService { host_context }),
        text_injector: Box::new(MacosTextInjector),
        cue_player: Box::new(MacosCuePlayer),
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn create_platform_services(_options: &PlatformInitOptions) -> Result<PlatformServices> {
    bail!("voice_bridge platform services are not implemented for this OS yet")
}
