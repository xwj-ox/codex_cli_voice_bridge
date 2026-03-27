use anyhow::Result;

#[cfg(target_os = "macos")]
use codex_cli_voice_bridge_rust::platform::macos::{
    host_permission_hint, missing_permission_names, preflight_permissions,
};

fn main() -> Result<()> {
    println!("voice_bridge_doctor");
    println!("platform: {}", std::env::consts::OS);
    println!("arch: {}", std::env::consts::ARCH);

    #[cfg(target_os = "windows")]
    {
        println!("runtime checks:");
        println!("- microphone permission: not preflighted in this command");
        println!("- accessibility permission: not required on Windows");
        println!("- input monitoring permission: not required on Windows");
        println!();
        println!("notes:");
        println!("- text injection into elevated windows still requires running voice_bridge with matching privileges");
        println!("- no additional runtime diagnostics are implemented for Windows yet");
    }

    #[cfg(target_os = "macos")]
    {
        let permissions = preflight_permissions();
        let missing = missing_permission_names(permissions);
        println!("runtime checks:");
        println!(
            "- accessibility permission: {}",
            if permissions.accessibility {
                "granted"
            } else {
                "missing"
            }
        );
        println!(
            "- input monitoring permission: {}",
            if permissions.input_monitoring {
                "granted"
            } else {
                "missing"
            }
        );
        println!("- microphone permission: not preflighted in this command");
        println!();
        println!("notes:");
        println!("- missing Accessibility or Input Monitoring will prevent global PTT and text injection");
        println!("- {}", host_permission_hint());
        if missing.is_empty() {
            println!("- if permissions appear granted but PTT still does not start, remove and re-add the host app in Privacy & Security, then restart the host app or log out and back in");
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        println!("runtime checks:");
        println!("- no platform-specific diagnostics implemented for this OS");
    }

    Ok(())
}
