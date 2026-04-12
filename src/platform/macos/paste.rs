use std::ptr;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::platform::types::WindowInfo;

type CFAllocatorRef = *const core::ffi::c_void;
type CFDataRef = *const core::ffi::c_void;
type CFIndex = isize;
type CFStringRef = *const core::ffi::c_void;
type OSStatus = i32;
type OptionBits = u32;
type PasteboardFlavorFlags = u32;
type PasteboardItemID = *mut core::ffi::c_void;
type PasteboardRef = *mut core::ffi::c_void;
type Pid = i32;
type CGEventRef = *mut core::ffi::c_void;
type CGEventFlags = u64;

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const KCG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 1 << 20;
const KCG_EVENT_SOURCE_USER_DATA: i32 = 42;
const KSET_FRONT_PROCESS_FRONT_WINDOW_ONLY: OptionBits = 1 << 0;
const KSET_FRONT_PROCESS_CAUSED_BY_USER: OptionBits = 1 << 1;
const KCG_SESSION_EVENT_TAP: u32 = 1;
const SYNTHETIC_PASSTHROUGH_TAG: i64 = 0x0043_5642_5050_5454;
const MACOS_KEY_CODE_LEFT_COMMAND: u16 = 55;
const MACOS_KEY_CODE_V: u16 = 9;
const MACOS_KEY_CODE_RETURN: u16 = 36;
const ACTIVATION_TIMEOUT: Duration = Duration::from_millis(400);
const ACTIVATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PASTEBOARD_ITEM_ID_TEXT: PasteboardItemID = 1usize as PasteboardItemID;
const PASTEBOARD_NAME_CLIPBOARD: &str = "com.apple.pasteboard.clipboard";
const TEXT_FLAVOR_UTF8: &str = "public.utf8-plain-text";

pub fn paste_text_to_window(
    target: &WindowInfo,
    text: &str,
    submit: bool,
    paste_delay_ms: u64,
) -> Result<()> {
    let pid = target.hwnd as Pid;
    if pid <= 0 {
        bail!("macOS target pid is missing");
    }

    activate_target_application(pid)?;
    write_clipboard(text)?;
    thread::sleep(Duration::from_millis(paste_delay_ms.max(20)));
    send_cmd_v()?;
    if submit {
        send_return()?;
    }
    Ok(())
}

fn activate_target_application(pid: Pid) -> Result<()> {
    if current_frontmost_pid() == Some(pid) {
        return Ok(());
    }

    let mut psn = ProcessSerialNumber {
        high_long_of_psn: 0,
        low_long_of_psn: 0,
    };
    os_status(
        unsafe { GetProcessForPID(pid, &mut psn) },
        "GetProcessForPID",
    )?;
    let _ = unsafe { WakeUpProcess(&psn) };
    os_status(
        unsafe {
            SetFrontProcessWithOptions(
                &psn,
                KSET_FRONT_PROCESS_FRONT_WINDOW_ONLY | KSET_FRONT_PROCESS_CAUSED_BY_USER,
            )
        },
        "SetFrontProcessWithOptions",
    )?;

    let started = Instant::now();
    while started.elapsed() < ACTIVATION_TIMEOUT {
        if current_frontmost_pid() == Some(pid) {
            return Ok(());
        }
        thread::sleep(ACTIVATION_POLL_INTERVAL);
    }

    bail!("target application did not become frontmost")
}

fn current_frontmost_pid() -> Option<Pid> {
    unsafe {
        let mut psn = ProcessSerialNumber {
            high_long_of_psn: 0,
            low_long_of_psn: 0,
        };
        if GetFrontProcess(&mut psn) != 0 {
            return None;
        }
        let mut pid = 0;
        if GetProcessPID(&psn, &mut pid) != 0 || pid <= 0 {
            return None;
        }
        Some(pid)
    }
}

fn write_clipboard(text: &str) -> Result<()> {
    unsafe {
        let mut pasteboard: PasteboardRef = ptr::null_mut();
        let clipboard_name = create_cf_string(PASTEBOARD_NAME_CLIPBOARD)
            .context("Failed to create macOS clipboard pasteboard name")?;
        let create_result = PasteboardCreate(clipboard_name, &mut pasteboard);
        CFRelease(clipboard_name as *const core::ffi::c_void);
        os_status(create_result, "PasteboardCreate")?;

        let flavor = create_cf_string(TEXT_FLAVOR_UTF8)
            .context("Failed to create macOS pasteboard flavor string")?;
        let data = CFDataCreate(ptr::null(), text.as_ptr(), text.len() as CFIndex);
        if data.is_null() {
            CFRelease(flavor as *const core::ffi::c_void);
            CFRelease(pasteboard as *const core::ffi::c_void);
            bail!("CFDataCreate failed for pasteboard text");
        }

        let clear_result = PasteboardClear(pasteboard);
        let put_result =
            PasteboardPutItemFlavor(pasteboard, PASTEBOARD_ITEM_ID_TEXT, flavor, data, 0);

        CFRelease(data);
        CFRelease(flavor as *const core::ffi::c_void);
        CFRelease(pasteboard as *const core::ffi::c_void);

        os_status(clear_result, "PasteboardClear")?;
        os_status(put_result, "PasteboardPutItemFlavor")?;
    }

    Ok(())
}

fn send_cmd_v() -> Result<()> {
    send_key_sequence(
        &[
            (
                MACOS_KEY_CODE_LEFT_COMMAND,
                true,
                KCG_EVENT_FLAG_MASK_COMMAND,
            ),
            (MACOS_KEY_CODE_V, true, KCG_EVENT_FLAG_MASK_COMMAND),
            (MACOS_KEY_CODE_V, false, KCG_EVENT_FLAG_MASK_COMMAND),
            (MACOS_KEY_CODE_LEFT_COMMAND, false, 0),
        ],
        "Cmd+V",
    )
}

fn send_return() -> Result<()> {
    send_key_sequence(
        &[
            (MACOS_KEY_CODE_RETURN, true, 0),
            (MACOS_KEY_CODE_RETURN, false, 0),
        ],
        "Return",
    )
}

fn send_key_sequence(sequence: &[(u16, bool, CGEventFlags)], label: &str) -> Result<()> {
    for (key_code, key_down, flags) in sequence {
        let event = create_keyboard_event(*key_code, *key_down, *flags)
            .with_context(|| format!("Failed to create macOS event for {label}"))?;
        unsafe {
            CGEventPost(KCG_SESSION_EVENT_TAP, event);
            CFRelease(event as *const core::ffi::c_void);
        }
        thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}

fn create_keyboard_event(key_code: u16, key_down: bool, flags: CGEventFlags) -> Result<CGEventRef> {
    unsafe {
        let event =
            CGEventCreateKeyboardEvent(ptr::null_mut(), key_code, if key_down { 1 } else { 0 });
        if event.is_null() {
            bail!("CGEventCreateKeyboardEvent returned null");
        }
        if flags != 0 {
            CGEventSetFlags(event, flags);
        }
        CGEventSetIntegerValueField(event, KCG_EVENT_SOURCE_USER_DATA, SYNTHETIC_PASSTHROUGH_TAG);
        Ok(event)
    }
}

fn create_cf_string(value: &str) -> Result<CFStringRef> {
    let c_value = std::ffi::CString::new(value).context("CString conversion failed")?;
    let cf_string = unsafe {
        CFStringCreateWithCString(ptr::null(), c_value.as_ptr(), KCF_STRING_ENCODING_UTF8)
    };
    if cf_string.is_null() {
        bail!("CFStringCreateWithCString returned null");
    }
    Ok(cf_string)
}

fn os_status(status: OSStatus, label: &str) -> Result<()> {
    if status == 0 {
        return Ok(());
    }
    bail!("{label} failed with status {status}");
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> OSStatus;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut Pid) -> OSStatus;
    fn GetProcessForPID(pid: Pid, psn: *mut ProcessSerialNumber) -> OSStatus;
    fn SetFrontProcessWithOptions(psn: *const ProcessSerialNumber, options: OptionBits)
    -> OSStatus;
    fn WakeUpProcess(psn: *const ProcessSerialNumber) -> OSStatus;

    fn PasteboardCreate(name: CFStringRef, pasteboard: *mut PasteboardRef) -> OSStatus;
    fn PasteboardClear(pasteboard: PasteboardRef) -> OSStatus;
    fn PasteboardPutItemFlavor(
        pasteboard: PasteboardRef,
        item: PasteboardItemID,
        flavor_type: CFStringRef,
        data: CFDataRef,
        flags: PasteboardFlavorFlags,
    ) -> OSStatus;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(
        source: *mut core::ffi::c_void,
        virtual_key: u16,
        key_down: u8,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
    fn CGEventSetIntegerValueField(event: CGEventRef, field: i32, value: i64);
    fn CGEventPost(tap: u32, event: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(allocator: CFAllocatorRef, bytes: *const u8, length: CFIndex) -> CFDataRef;
    fn CFRelease(value: *const core::ffi::c_void);
    fn CFStringCreateWithCString(
        allocator: CFAllocatorRef,
        c_str: *const core::ffi::c_char,
        encoding: u32,
    ) -> CFStringRef;
}
