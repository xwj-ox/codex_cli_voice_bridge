use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::platform::types::{CueKind, PttActivation, WindowInfo};

pub type PttActivationFuture<'a> =
    Pin<Box<dyn Future<Output = Option<PttActivation>> + Send + 'a>>;

pub trait PttController: Send {
    fn recv_activation(&mut self) -> PttActivationFuture<'_>;
}

pub trait WindowService: Send + Sync {
    fn current_target(&self) -> Option<WindowInfo>;
    fn is_usable_target(
        &self,
        target: Option<&WindowInfo>,
        require_title: &str,
        forbid_host_window_target: bool,
    ) -> bool;
}

pub trait TextInjector: Send + Sync {
    fn paste_text(
        &self,
        target: &WindowInfo,
        text: &str,
        submit: bool,
        paste_delay_ms: u64,
    ) -> Result<()>;
}

pub trait CuePlayer: Send + Sync {
    fn play(&self, cue: CueKind);
}
