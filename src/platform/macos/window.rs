use std::env;
use std::ffi::CString;
use std::ptr;

use anyhow::{Context, Result, bail};

use crate::platform::types::WindowInfo;

type AXError = i32;
type AXUIElementRef = *mut core::ffi::c_void;
type Boolean = u8;
type CFAllocatorRef = *const core::ffi::c_void;
type CFIndex = isize;
type CFStringRef = *const core::ffi::c_void;
type CFTypeRef = *const core::ffi::c_void;
type OSErr = i32;
type Pid = i32;

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const AX_ATTR_FOCUSED_WINDOW: &str = "AXFocusedWindow";
const AX_ATTR_TITLE: &str = "AXTitle";
const AX_MESSAGING_TIMEOUT_SECONDS: f32 = 0.08;

#[derive(Debug, Clone)]
pub struct MacosHostContext {
    host_process_names: Vec<String>,
}

impl MacosHostContext {
    pub fn detect() -> Self {
        let mut host_process_names = Vec::new();
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
        push_terminal_host_names(&mut host_process_names);

        Self { host_process_names }
    }
}

pub fn get_frontmost_window_info() -> Option<WindowInfo> {
    match read_frontmost_window() {
        Ok(Some((process_name, title, pid))) => Some(WindowInfo {
            hwnd: pid as isize,
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

fn push_terminal_host_names(values: &mut Vec<String>) {
    for key in ["TERM_PROGRAM", "TERM_PROGRAM_APP", "LC_TERMINAL"] {
        if let Ok(value) = env::var(key) {
            push_host_aliases(values, &value);
        }
    }
}

fn push_host_aliases(values: &mut Vec<String>, candidate: &str) {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return;
    }
    push_unique_name(values, candidate);
    if let Some(value) = candidate.strip_suffix(".app") {
        push_unique_name(values, value);
    }
    match normalize_process_name(candidate).as_str() {
        "apple_terminal" => push_unique_name(values, "Terminal"),
        "iterm.app" | "iterm" | "iterm2" => {
            push_unique_name(values, "iTerm");
            push_unique_name(values, "iTerm2");
        }
        "warpterminal" | "warp" => {
            push_unique_name(values, "Warp");
            push_unique_name(values, "WarpTerminal");
        }
        _ => {}
    }
}

fn read_frontmost_window() -> Result<Option<(String, String, Pid)>> {
    unsafe {
        let mut psn = ProcessSerialNumber {
            high_long_of_psn: 0,
            low_long_of_psn: 0,
        };
        os_status(GetFrontProcess(&mut psn), "GetFrontProcess")?;

        let mut pid = 0;
        os_status(GetProcessPID(&psn, &mut pid), "GetProcessPID")?;
        if pid <= 0 {
            return Ok(None);
        }

        let mut process_name_ref: CFStringRef = ptr::null();
        os_status(
            CopyProcessName(&psn, &mut process_name_ref),
            "CopyProcessName",
        )?;
        let process_name = cf_string_to_string(process_name_ref).unwrap_or_default();
        if !process_name_ref.is_null() {
            CFRelease(process_name_ref as *const core::ffi::c_void);
        }
        if process_name.trim().is_empty() {
            bail!("Frontmost macOS process name is empty");
        }

        let title = read_focused_window_title(pid).unwrap_or_default();
        Ok(Some((process_name, title, pid)))
    }
}

fn read_focused_window_title(pid: Pid) -> Result<String> {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            bail!("AXUIElementCreateApplication returned null");
        }
        let _ = AXUIElementSetMessagingTimeout(app, AX_MESSAGING_TIMEOUT_SECONDS);

        let focused_window_attr = create_cf_string(AX_ATTR_FOCUSED_WINDOW)?;
        let mut focused_window: CFTypeRef = ptr::null();
        let focused_window_result =
            AXUIElementCopyAttributeValue(app, focused_window_attr, &mut focused_window);
        CFRelease(focused_window_attr as *const core::ffi::c_void);
        if focused_window_result != 0 || focused_window.is_null() {
            CFRelease(app as *const core::ffi::c_void);
            return Ok(String::new());
        }

        let title_attr = create_cf_string(AX_ATTR_TITLE)?;
        let mut title_value: CFTypeRef = ptr::null();
        let title_result = AXUIElementCopyAttributeValue(
            focused_window as AXUIElementRef,
            title_attr,
            &mut title_value,
        );
        CFRelease(title_attr as *const core::ffi::c_void);
        CFRelease(focused_window);
        CFRelease(app as *const core::ffi::c_void);

        if title_result != 0 || title_value.is_null() {
            return Ok(String::new());
        }

        let title = cf_string_to_string(title_value as CFStringRef).unwrap_or_default();
        CFRelease(title_value);
        Ok(title)
    }
}

fn create_cf_string(value: &str) -> Result<CFStringRef> {
    let c_value = CString::new(value).context("CString conversion failed")?;
    let cf_string = unsafe {
        CFStringCreateWithCString(ptr::null(), c_value.as_ptr(), KCF_STRING_ENCODING_UTF8)
    };
    if cf_string.is_null() {
        bail!("CFStringCreateWithCString returned null");
    }
    Ok(cf_string)
}

fn cf_string_to_string(value: CFStringRef) -> Result<String> {
    if value.is_null() {
        return Ok(String::new());
    }

    unsafe {
        let length = CFStringGetLength(value);
        let max_size = CFStringGetMaximumSizeForEncoding(length, KCF_STRING_ENCODING_UTF8);
        let mut buffer = vec![0u8; max_size as usize + 1];
        let ok = CFStringGetCString(
            value,
            buffer.as_mut_ptr().cast(),
            buffer.len() as CFIndex,
            KCF_STRING_ENCODING_UTF8,
        );
        if ok == 0 {
            bail!("CFStringGetCString failed");
        }
        let nul = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf8_lossy(&buffer[..nul]).into_owned())
    }
}

fn os_status(status: OSErr, label: &str) -> Result<()> {
    if status == 0 {
        return Ok(());
    }
    bail!("{label} failed with status {status}");
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> OSErr;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut Pid) -> OSErr;
    fn CopyProcessName(psn: *const ProcessSerialNumber, name: *mut CFStringRef) -> OSErr;

    fn AXUIElementCreateApplication(pid: Pid) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout_in_seconds: f32) -> AXError;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const core::ffi::c_void);
    fn CFStringCreateWithCString(
        allocator: CFAllocatorRef,
        c_str: *const core::ffi::c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetCString(
        the_string: CFStringRef,
        buffer: *mut core::ffi::c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> Boolean;
    fn CFStringGetLength(the_string: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
}
