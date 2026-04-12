use std::thread;
use std::time::Duration;

use crate::platform::types::CueKind;

pub fn play_cue(enabled: bool, kind: CueKind) {
    if !enabled {
        return;
    }

    let repeat_count = match kind {
        CueKind::Listen => 1,
        CueKind::Recognized => 2,
        CueKind::Pasted => 1,
        CueKind::Error => 3,
    };

    thread::spawn(move || {
        for index in 0..repeat_count {
            unsafe { NSBeep() };
            if index + 1 < repeat_count {
                thread::sleep(Duration::from_millis(110));
            }
        }
    });
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSBeep();
}
