use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::platform::types::WindowInfo;

pub fn paste_text_to_window(
    target: &WindowInfo,
    text: &str,
    submit: bool,
    paste_delay_ms: u64,
) -> Result<()> {
    activate_target_application(target)?;
    write_clipboard(text)?;
    thread::sleep(Duration::from_millis(paste_delay_ms.max(20)));
    send_cmd_v()?;
    if submit {
        send_return()?;
    }
    Ok(())
}

fn activate_target_application(target: &WindowInfo) -> Result<()> {
    let app_name = target.process_name.trim();
    if app_name.is_empty() {
        bail!("macOS target application name is empty");
    }

    let escaped = app_name.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(r#"tell application "{escaped}" to activate"#);
    run_osascript(&script, "activate target application")?;
    thread::sleep(Duration::from_millis(120));
    Ok(())
}

fn write_clipboard(text: &str) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .context("Failed to start pbcopy")?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .context("Failed to write text to pbcopy")?;
    }
    let status = child.wait().context("Failed waiting for pbcopy")?;
    if !status.success() {
        bail!("pbcopy failed with status {status}");
    }
    Ok(())
}

fn send_cmd_v() -> Result<()> {
    run_osascript(
        r#"tell application "System Events" to keystroke "v" using command down"#,
        "Cmd+V",
    )
}

fn send_return() -> Result<()> {
    run_osascript(
        r#"tell application "System Events" to key code 36"#,
        "Return",
    )
}

fn run_osascript(script: &str, label: &str) -> Result<()> {
    let status = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .with_context(|| format!("Failed to run osascript for {label}"))?;
    if !status.success() {
        bail!("osascript failed while sending {label}");
    }
    Ok(())
}
