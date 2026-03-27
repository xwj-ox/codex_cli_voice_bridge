use std::mem::size_of;
use std::ptr::copy_nonoverlapping;
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SW_RESTORE, SetForegroundWindow, ShowWindow,
};

use crate::platform::windows::window::get_foreground_window_info;

const CF_UNICODETEXT: u32 = 13;

pub fn paste_text_to_window(
    text: &str,
    expected_hwnd: isize,
    submit: bool,
    paste_delay_ms: u64,
) -> Result<()> {
    let current = get_foreground_window_info();
    if current.as_ref().map(|info| info.hwnd) != Some(expected_hwnd) {
        unsafe {
            let hwnd = HWND(expected_hwnd as *mut core::ffi::c_void);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            if !SetForegroundWindow(hwnd).as_bool() {
                bail!("SetForegroundWindow failed");
            }
        }
        thread::sleep(Duration::from_millis(120));
    }

    let restored = get_foreground_window_info();
    if restored.as_ref().map(|info| info.hwnd) != Some(expected_hwnd) {
        bail!("target window is not focused; refocus it and try again");
    }

    let original_clipboard = get_clipboard_text().ok().flatten();
    set_clipboard_text(text)?;
    send_ctrl_v()?;
    thread::sleep(Duration::from_millis(paste_delay_ms.max(20)));
    if submit {
        send_key_tap(VK_RETURN.0 as u32)?;
    }
    if let Some(original) = original_clipboard.as_deref() {
        let _ = set_clipboard_text(original);
    }
    Ok(())
}

pub(crate) fn send_key_tap(vk_code: u32) -> Result<()> {
    send_input_sequence(&[keyboard_input(vk_code, false), keyboard_input(vk_code, true)])
}

fn get_clipboard_text() -> Result<Option<String>> {
    unsafe {
        open_clipboard_with_retry()?;
        let handle = GetClipboardData(CF_UNICODETEXT)
            .map_err(|error| anyhow!("GetClipboardData failed: {error}"))?;
        if handle.0.is_null() {
            let _ = CloseClipboard();
            return Ok(None);
        }
        let hglobal = HGLOBAL(handle.0);
        let pointer = GlobalLock(hglobal);
        if pointer.is_null() {
            let _ = CloseClipboard();
            return Ok(None);
        }
        let mut len = 0usize;
        let mut cur = pointer as *const u16;
        while *cur != 0 {
            len += 1;
            cur = cur.add(1);
        }
        let slice = std::slice::from_raw_parts(pointer as *const u16, len);
        let text = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(hglobal);
        let _ = CloseClipboard();
        Ok(Some(text))
    }
}

fn set_clipboard_text(text: &str) -> Result<()> {
    unsafe {
        let wide = text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<_>>();
        let bytes = wide.len() * size_of::<u16>();
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
        if handle.0.is_null() {
            bail!("GlobalAlloc failed");
        }
        let pointer = GlobalLock(handle) as *mut u16;
        if pointer.is_null() {
            bail!("GlobalLock failed");
        }
        copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
        let _ = GlobalUnlock(handle);

        open_clipboard_with_retry()?;
        EmptyClipboard().map_err(|error| anyhow!("EmptyClipboard failed: {error}"))?;
        if let Err(error) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(handle.0))) {
            let _ = CloseClipboard();
            bail!("SetClipboardData failed: {error}");
        }
        let _ = CloseClipboard();
        Ok(())
    }
}

fn open_clipboard_with_retry() -> Result<()> {
    for _ in 0..8 {
        unsafe {
            if OpenClipboard(Some(HWND(std::ptr::null_mut()))).is_ok() {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(30));
    }
    bail!("OpenClipboard failed")
}

fn send_ctrl_v() -> Result<()> {
    send_input_sequence(&[
        keyboard_input(0x11, false),
        keyboard_input(0x56, false),
        keyboard_input(0x56, true),
        keyboard_input(0x11, true),
    ])
}

fn send_input_sequence(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        bail!("SendInput sent {sent} of {} events", inputs.len());
    }
    Ok(())
}

fn keyboard_input(vk_code: u32, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk_code as u16),
                wScan: 0,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
