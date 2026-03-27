use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
};

use crate::platform::types::WindowInfo;

pub fn get_foreground_window_info() -> Option<WindowInfo> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let title = get_window_text(hwnd);
        let process_name = get_process_name(hwnd).unwrap_or_else(|_| "<unknown>".to_owned());
        Some(WindowInfo {
            hwnd: hwnd.0 as isize,
            title,
            process_name,
        })
    }
}

pub fn is_usable_target_window(
    window: Option<&WindowInfo>,
    require_title: &str,
    bridge_host_hwnd: Option<isize>,
) -> bool {
    let Some(window) = window else {
        return false;
    };
    if window.hwnd == 0 {
        return false;
    }
    if let Some(host_hwnd) = bridge_host_hwnd {
        if window.hwnd == host_hwnd {
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

fn get_window_text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length == 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; (length + 1) as usize];
        let copied = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

fn get_process_name(hwnd: HWND) -> Result<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            bail!("Missing process id");
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .context("OpenProcess failed")?;
        let mut buffer = vec![0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(process);
        if ok.is_err() {
            bail!("QueryFullProcessImageNameW failed");
        }
        Ok(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}
