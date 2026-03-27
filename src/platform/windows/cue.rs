use crate::platform::types::CueKind;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn Beep(dwFreq: u32, dwDuration: u32) -> i32;
}

pub fn play_cue(enabled: bool, kind: CueKind) {
    if !enabled {
        return;
    }
    let pattern: &[(u32, u32)] = match kind {
        CueKind::Listen => &[(880, 50)],
        CueKind::Recognized => &[(1175, 60), (1480, 70)],
        CueKind::Pasted => &[(1568, 45)],
        CueKind::Error => &[(392, 120)],
    };

    for (freq, duration_ms) in pattern {
        unsafe {
            let _ = Beep(*freq, *duration_ms);
        }
    }
}
