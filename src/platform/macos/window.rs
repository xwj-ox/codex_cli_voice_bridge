use std::env;
use std::process::Command;

use anyhow::{Context, Result};

use crate::platform::types::WindowInfo;

#[derive(Debug, Clone)]
pub struct MacosHostContext {
    host_process_names: Vec<String>,
}

impl MacosHostContext {
    pub fn detect() -> Self {
        let mut host_process_names = Vec::new();
        if let Some(window) = get_frontmost_window_info() {
            push_unique_name(&mut host_process_names, &window.process_name);
        }
        if let Ok(path) = env::current_exe() {
            if let Some(name) = path.file_stem().and_then(|value| value.to_str()) {
                push_unique_name(&mut host_process_names, name);
            }
            for ancestor in path.ancestors() {
                if let Some(component) = ancestor.file_name().and_then(|value| value.to_str())
                    && let Some(bundle_name) = component.strip_suffix(".app")
                {
                    push_unique_name(&mut host_process_names, bundle_name);
                    break;
                }
            }
        }

        Self { host_process_names }
    }
}

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
    forbid_host_window_target: bool,
    host_context: Option<&MacosHostContext>,
) -> bool {
    let Some(window) = window else {
        return false;
    };
    if window.process_name.trim().is_empty() {
        return false;
    }
    if forbid_host_window_target && let Some(host_context) = host_context {
        let process_name = normalize_process_name(&window.process_name);
        if host_context
            .host_process_names
            .iter()
            .any(|value| normalize_process_name(value) == process_name)
        {
            return false;
        }
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

fn normalize_process_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn push_unique_name(values: &mut Vec<String>, candidate: &str) {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return;
    }
    let normalized = normalize_process_name(candidate);
    if values
        .iter()
        .any(|existing| normalize_process_name(existing) == normalized)
    {
        return;
    }
    values.push(candidate.to_owned());
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
