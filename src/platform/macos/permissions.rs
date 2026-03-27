use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub input_monitoring: bool,
}

pub fn preflight_permissions() -> PermissionStatus {
    PermissionStatus {
        accessibility: unsafe { AXIsProcessTrusted() != 0 },
        input_monitoring: unsafe { CGPreflightListenEventAccess() != 0 },
    }
}

pub fn missing_permission_names(status: PermissionStatus) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !status.accessibility {
        missing.push("Accessibility");
    }
    if !status.input_monitoring {
        missing.push("Input Monitoring");
    }
    missing
}

pub fn host_permission_hint() -> &'static str {
    "Grant access to the host app in System Settings -> Privacy & Security. If you run via cargo, the host app is usually Terminal or iTerm; if you run a packaged build, grant the packaged voice_bridge app itself."
}

pub fn permission_failure_message(status: PermissionStatus) -> String {
    let missing = missing_permission_names(status);
    if missing.is_empty() {
        format!(
            "Accessibility and Input Monitoring appear granted, but the event tap still failed. Re-add the host app in System Settings -> Privacy & Security, then restart the host app or log out and back in. {}",
            host_permission_hint()
        )
    } else {
        format!(
            "macOS permissions are missing: {}. {}",
            missing.join(", "),
            host_permission_hint()
        )
    }
}

pub fn ensure_runtime_permissions() -> Result<PermissionStatus> {
    let mut status = preflight_permissions();
    if !status.input_monitoring {
        let _ = unsafe { CGRequestListenEventAccess() };
        status = preflight_permissions();
    }

    if !missing_permission_names(status).is_empty() {
        bail!("{}", permission_failure_message(status));
    }

    Ok(status)
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> u8;
    fn CGRequestListenEventAccess() -> u8;
}
