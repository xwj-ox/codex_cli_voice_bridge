use std::process::Command;

use anyhow::{Context, Result};

use crate::platform::types::WindowInfo;

pub fn get_frontmost_window_info() -> Option<WindowInfo> {
    match read_frontmost_window() {
        Ok(Some((process_name, title))) => Some(WindowInfo {
            hwnd: 1,
            title,
            process_name,
        }),
        Ok(None) => None,
        Err(_) => None,
    }
}

pub fn is_usable_target_window(
    window: Option<&WindowInfo>,
    require_title: &str,
    _forbid_host_window_target: bool,
) -> bool {
    let Some(window) = window else {
        return false;
    };
    if window.process_name.trim().is_empty() {
        return false;
    }
    if !require_title.trim().is_empty()
        && !window
            .title
            .to_ascii_lowercase()
            .contains(&require_title.trim().to_ascii_lowercase())
    {
        return false;
    }
    true
}

fn read_frontmost_window() -> Result<Option<(String, String)>> {
    let script = r#"
tell application "System Events"
    set frontApp to first application process whose frontmost is true
    set appName to name of frontApp
    set windowTitle to ""
    try
        set windowTitle to name of front window of frontApp
    end try
    return appName & linefeed & windowTitle
end tell
"#;
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .context("Failed to run osascript for frontmost window detection")?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let mut lines = text.lines();
    let app_name = lines.next().unwrap_or_default().trim().to_owned();
    let title = lines.next().unwrap_or_default().trim().to_owned();
    if app_name.is_empty() {
        return Ok(None);
    }
    Ok(Some((app_name, title)))
}
