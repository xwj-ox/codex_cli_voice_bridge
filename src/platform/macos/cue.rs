use std::process::Command;

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

    let script = format!("repeat {repeat_count} times\nbeep\nend repeat");
    let _ = Command::new("osascript").arg("-e").arg(script).status();
}
