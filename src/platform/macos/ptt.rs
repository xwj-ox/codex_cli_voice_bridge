use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::platform::macos::permissions::{permission_failure_message, preflight_permissions};
use crate::platform::macos::window::get_frontmost_window_info;
use crate::platform::types::{PttActivation, WindowInfo};

type CGEventTapProxy = *mut core::ffi::c_void;
type CGEventRef = *mut core::ffi::c_void;
type CGEventMask = u64;
type CFMachPortRef = *mut core::ffi::c_void;
type CFRunLoopRef = *mut core::ffi::c_void;
type CFRunLoopSourceRef = *mut core::ffi::c_void;
type CFAllocatorRef = *const core::ffi::c_void;
type CFIndex = isize;
type CFStringRef = *const core::ffi::c_void;
type CFMutableDictionaryRef = *mut core::ffi::c_void;
type CFNumberRef = *const core::ffi::c_void;
type IOHIDManagerRef = *mut core::ffi::c_void;
type IOHIDValueRef = *mut core::ffi::c_void;
type IOHIDElementRef = *mut core::ffi::c_void;
type IOReturn = i32;
type CGEventType = u32;
type CGEventFlags = u64;

const KCG_EVENT_KEY_DOWN: CGEventType = 10;
const KCG_EVENT_KEY_UP: CGEventType = 11;
const KCG_EVENT_FLAGS_CHANGED: CGEventType = 12;
const KCG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFF_FFFE;
const KCG_EVENT_TAP_DISABLED_BY_USER_INPUT: CGEventType = 0xFFFF_FFFF;
const KCG_EVENT_TAP_SESSION: u32 = 1;
const KCG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const KCG_EVENT_TAP_DEFAULT: u32 = 0;
const KCG_SESSION_EVENT_TAP: u32 = 1;
const KCG_KEYBOARD_EVENT_KEYCODE: i32 = 9;
const MACOS_KEY_CODE_CAPS_LOCK: i64 = 57;
const KCG_EVENT_FLAG_MASK_SHIFT: CGEventFlags = 1 << 17;
const KCG_EVENT_FLAG_MASK_CONTROL: CGEventFlags = 1 << 18;
const KCG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 1 << 20;
const KCG_EVENT_FLAG_MASK_SECONDARY_FN: CGEventFlags = 1 << 23;
const KCG_EVENT_SOURCE_USER_DATA: i32 = 42;
const SYNTHETIC_PASSTHROUGH_TAG: i64 = 0x0043_5642_5050_5454;
const KIOHID_OPTIONS_TYPE_NONE: u32 = 0;
const KIO_RETURN_SUCCESS: IOReturn = 0;
const KCF_NUMBER_INT_TYPE: i32 = 9;
const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const HID_PAGE_GENERIC_DESKTOP: i32 = 0x01;
const HID_USAGE_GENERIC_DESKTOP_KEYBOARD: i32 = 0x06;
const HID_PAGE_KEYBOARD_OR_KEYPAD: u32 = 0x07;
const HID_USAGE_KEYBOARD_CAPS_LOCK: u32 = 0x39;

#[repr(C)]
struct CFDictionaryKeyCallBacks {
    version: CFIndex,
    retain: *const core::ffi::c_void,
    release: *const core::ffi::c_void,
    copy_description: *const core::ffi::c_void,
    equal: *const core::ffi::c_void,
    hash: *const core::ffi::c_void,
}

#[repr(C)]
struct CFDictionaryValueCallBacks {
    version: CFIndex,
    retain: *const core::ffi::c_void,
    release: *const core::ffi::c_void,
    copy_description: *const core::ffi::c_void,
    equal: *const core::ffi::c_void,
}

#[derive(Debug)]
enum RawKeyEvent {
    Press,
    Release,
}

#[derive(Debug, Clone, Copy)]
enum MacosPttKey {
    CapsLock {
        key_code: i64,
    },
    Fn {
        flag_mask: CGEventFlags,
    },
    Regular {
        key_code: u16,
    },
    Modifier {
        key_code: i64,
        flag_mask: CGEventFlags,
    },
}

#[derive(Debug)]
struct HookState {
    key: MacosPttKey,
    suppress: bool,
    modifier_pressed: bool,
    tap: EventTapHandle,
    sender: mpsc::Sender<RawKeyEvent>,
}

#[derive(Debug, Clone, Copy)]
struct EventTapHandle(usize);

impl EventTapHandle {
    fn from_ptr(value: CFMachPortRef) -> Self {
        Self(value as usize)
    }

    fn as_ptr(self) -> CFMachPortRef {
        self.0 as CFMachPortRef
    }
}

#[derive(Debug, Clone, Copy)]
struct RunLoopHandle(usize);

impl RunLoopHandle {
    fn null() -> Self {
        Self(0)
    }

    fn from_ptr(value: CFRunLoopRef) -> Self {
        Self(value as usize)
    }

    fn as_ptr(self) -> CFRunLoopRef {
        self.0 as CFRunLoopRef
    }

    fn is_null(self) -> bool {
        self.0 == 0
    }
}

fn global_hook_state() -> &'static Mutex<Option<HookState>> {
    static STATE: OnceLock<Mutex<Option<HookState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

unsafe extern "C" fn keyboard_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    _user_info: *mut core::ffi::c_void,
) -> CGEventRef {
    if event_type == KCG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || event_type == KCG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        if let Ok(guard) = global_hook_state().lock()
            && let Some(state) = guard.as_ref()
        {
            unsafe {
                CGEventTapEnable(state.tap.as_ptr(), 1);
            }
        }
        return event;
    }

    let source_user_data =
        unsafe { CGEventGetIntegerValueField(event, KCG_EVENT_SOURCE_USER_DATA) };
    if source_user_data == SYNTHETIC_PASSTHROUGH_TAG {
        return event;
    }

    if let Ok(mut guard) = global_hook_state().lock()
        && let Some(state) = guard.as_mut()
    {
        let key_code = unsafe { CGEventGetIntegerValueField(event, KCG_KEYBOARD_EVENT_KEYCODE) };
        let mut handled = false;
        match state.key {
            MacosPttKey::CapsLock { key_code: expected } => {
                if key_code == expected {
                    if matches!(
                        event_type,
                        KCG_EVENT_KEY_DOWN | KCG_EVENT_KEY_UP | KCG_EVENT_FLAGS_CHANGED
                    ) {
                        // Caps Lock is a locking modifier in Quartz. Its physical up/down
                        // events are delivered by the HID callback; this tap only suppresses
                        // the real key event path when possible.
                        handled = true;
                    }
                }
            }
            MacosPttKey::Fn { flag_mask } => {
                if event_type == KCG_EVENT_FLAGS_CHANGED {
                    let flags = unsafe { CGEventGetFlags(event) };
                    let aggregate_flag_pressed = (flags & flag_mask) != 0;
                    if aggregate_flag_pressed != state.modifier_pressed {
                        let raw_event = if aggregate_flag_pressed {
                            RawKeyEvent::Press
                        } else {
                            RawKeyEvent::Release
                        };
                        state.modifier_pressed = aggregate_flag_pressed;
                        let _ = state.sender.send(raw_event);
                        handled = true;
                    }
                }
            }
            MacosPttKey::Regular { key_code: expected } => {
                if key_code == expected as i64 {
                    match event_type {
                        KCG_EVENT_KEY_DOWN => {
                            let _ = state.sender.send(RawKeyEvent::Press);
                            handled = true;
                        }
                        KCG_EVENT_KEY_UP => {
                            let _ = state.sender.send(RawKeyEvent::Release);
                            handled = true;
                        }
                        _ => {}
                    }
                }
            }
            MacosPttKey::Modifier {
                key_code: expected,
                flag_mask,
            } => {
                if event_type == KCG_EVENT_FLAGS_CHANGED && key_code == expected {
                    let flags = unsafe { CGEventGetFlags(event) };
                    let aggregate_flag_pressed = (flags & flag_mask) != 0;
                    let raw_event = if aggregate_flag_pressed != state.modifier_pressed {
                        if aggregate_flag_pressed {
                            RawKeyEvent::Press
                        } else {
                            RawKeyEvent::Release
                        }
                    } else {
                        RawKeyEvent::Release
                    };
                    state.modifier_pressed = matches!(raw_event, RawKeyEvent::Press);
                    let _ = state.sender.send(raw_event);
                    handled = true;
                }
            }
        }

        if handled && state.suppress {
            return ptr::null_mut();
        }
    }

    event
}

unsafe extern "C" fn hid_input_value_callback(
    _context: *mut core::ffi::c_void,
    _result: IOReturn,
    _sender: *mut core::ffi::c_void,
    value: IOHIDValueRef,
) {
    if value.is_null() {
        return;
    }

    let element = unsafe { IOHIDValueGetElement(value) };
    if element.is_null() {
        return;
    }

    let usage_page = unsafe { IOHIDElementGetUsagePage(element) };
    let usage = unsafe { IOHIDElementGetUsage(element) };
    if usage_page != HID_PAGE_KEYBOARD_OR_KEYPAD || usage != HID_USAGE_KEYBOARD_CAPS_LOCK {
        return;
    }

    let pressed = unsafe { IOHIDValueGetIntegerValue(value) } != 0;
    if let Ok(mut guard) = global_hook_state().lock()
        && let Some(state) = guard.as_mut()
        && matches!(state.key, MacosPttKey::CapsLock { .. })
        && pressed != state.modifier_pressed
    {
        state.modifier_pressed = pressed;
        let raw_event = if pressed {
            RawKeyEvent::Press
        } else {
            RawKeyEvent::Release
        };
        let _ = state.sender.send(raw_event);
    }
}

pub struct MacosHookHandle {
    run_loop: RunLoopHandle,
    join_handle: Option<JoinHandle<()>>,
}

impl Drop for MacosHookHandle {
    fn drop(&mut self) {
        unsafe {
            let run_loop = self.run_loop.as_ptr();
            if !run_loop.is_null() {
                CFRunLoopStop(run_loop);
                CFRunLoopWakeUp(run_loop);
            }
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
        unsafe {
            let run_loop = self.run_loop.as_ptr();
            if !run_loop.is_null() {
                CFRelease(run_loop);
                self.run_loop = RunLoopHandle::null();
            }
        }
    }
}

pub struct MacosPttController {
    receiver: UnboundedReceiver<PttActivation>,
    exit_flag: Arc<AtomicBool>,
    hook: Option<MacosHookHandle>,
    worker_join: Option<JoinHandle<()>>,
}

impl MacosPttController {
    pub fn new(key_name: &str, hold_ms: u64, passthrough_short_press: bool) -> Result<Self> {
        let key = parse_ptt_key(key_name)?;
        let (raw_tx, raw_rx) = mpsc::channel();
        let (run_loop_tx, run_loop_rx) = mpsc::channel();
        let join_handle = thread::spawn(move || {
            unsafe {
                let mask = (1u64 << KCG_EVENT_KEY_DOWN)
                    | (1u64 << KCG_EVENT_KEY_UP)
                    | (1u64 << KCG_EVENT_FLAGS_CHANGED);
                let tap = CGEventTapCreate(
                    KCG_EVENT_TAP_SESSION,
                    KCG_HEAD_INSERT_EVENT_TAP,
                    KCG_EVENT_TAP_DEFAULT,
                    mask,
                    keyboard_callback,
                    ptr::null_mut(),
                );
                if tap.is_null() {
                    let _ = run_loop_tx.send(RunLoopHandle::null());
                    return;
                }

                let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
                if source.is_null() {
                    let _ = run_loop_tx.send(RunLoopHandle::null());
                    CFRelease(tap);
                    if let Ok(mut guard) = global_hook_state().lock() {
                        *guard = None;
                    }
                    return;
                }
                let run_loop = CFRunLoopGetCurrent();
                let retained_run_loop = CFRetain(run_loop) as CFRunLoopRef;
                if let Ok(mut guard) = global_hook_state().lock() {
                    *guard = Some(HookState {
                        key,
                        suppress: true,
                        modifier_pressed: false,
                        tap: EventTapHandle::from_ptr(tap),
                        sender: raw_tx,
                    });
                }
                let hid_manager = if matches!(key, MacosPttKey::CapsLock { .. }) {
                    match create_caps_lock_hid_manager(run_loop) {
                        Some(manager) => Some(manager),
                        None => {
                            let _ = run_loop_tx.send(RunLoopHandle::null());
                            CFRelease(retained_run_loop);
                            CFRelease(source);
                            CFRelease(tap);
                            if let Ok(mut guard) = global_hook_state().lock() {
                                *guard = None;
                            }
                            return;
                        }
                    }
                } else {
                    None
                };
                if run_loop_tx
                    .send(RunLoopHandle::from_ptr(retained_run_loop))
                    .is_err()
                {
                    if let Some(hid_manager) = hid_manager {
                        IOHIDManagerUnscheduleFromRunLoop(
                            hid_manager,
                            run_loop,
                            kCFRunLoopCommonModes,
                        );
                        let _ = IOHIDManagerClose(hid_manager, KIOHID_OPTIONS_TYPE_NONE);
                        CFRelease(hid_manager);
                    }
                    CFRelease(retained_run_loop);
                    CFRelease(source);
                    CFRelease(tap);
                    if let Ok(mut guard) = global_hook_state().lock() {
                        *guard = None;
                    }
                    return;
                }
                CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, 1);
                CFRunLoopRun();
                if let Some(hid_manager) = hid_manager {
                    IOHIDManagerUnscheduleFromRunLoop(hid_manager, run_loop, kCFRunLoopCommonModes);
                    let _ = IOHIDManagerClose(hid_manager, KIOHID_OPTIONS_TYPE_NONE);
                    CFRelease(hid_manager);
                }
                CFRelease(source);
                CFRelease(tap);
            }

            if let Ok(mut guard) = global_hook_state().lock() {
                *guard = None;
            }
        });

        let run_loop = run_loop_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| anyhow!("Failed to initialize macOS event tap thread"))?;
        if run_loop.is_null() {
            let status = preflight_permissions();
            bail!(
                "Failed to create macOS event tap. {}",
                permission_failure_message(status)
            );
        }

        let (activation_tx, activation_rx) = unbounded_channel();
        let exit_flag = Arc::new(AtomicBool::new(false));
        let worker_exit_flag = exit_flag.clone();
        let worker_join = thread::spawn(move || {
            worker_loop(
                raw_rx,
                activation_tx,
                worker_exit_flag,
                key,
                hold_ms,
                passthrough_short_press,
            );
        });

        Ok(Self {
            receiver: activation_rx,
            exit_flag,
            hook: Some(MacosHookHandle {
                run_loop,
                join_handle: Some(join_handle),
            }),
            worker_join: Some(worker_join),
        })
    }

    pub async fn recv(&mut self) -> Option<PttActivation> {
        self.receiver.recv().await
    }
}

impl Drop for MacosPttController {
    fn drop(&mut self) {
        self.exit_flag.store(true, Ordering::Relaxed);
        let _ = self.hook.take();
        if let Some(handle) = self.worker_join.take() {
            let _ = handle.join();
        }
    }
}

fn worker_loop(
    raw_rx: mpsc::Receiver<RawKeyEvent>,
    activation_tx: UnboundedSender<PttActivation>,
    exit_flag: Arc<AtomicBool>,
    key: MacosPttKey,
    hold_ms: u64,
    passthrough_short_press: bool,
) {
    let mut physical_pressed = false;
    let mut activated = false;
    let mut press_started_at: Option<Instant> = None;
    let mut press_window: Option<WindowInfo> = None;
    let mut active_stop_signal: Option<Arc<AtomicBool>> = None;

    while !exit_flag.load(Ordering::Relaxed) {
        let timeout = if physical_pressed && !activated {
            if let Some(started) = press_started_at {
                let elapsed = started.elapsed();
                if elapsed >= Duration::from_millis(hold_ms) {
                    Duration::from_millis(0)
                } else {
                    Duration::from_millis(hold_ms) - elapsed
                }
            } else {
                Duration::from_millis(50)
            }
        } else {
            Duration::from_millis(50)
        };

        match raw_rx.recv_timeout(timeout) {
            Ok(RawKeyEvent::Press) => {
                if physical_pressed {
                    continue;
                }
                physical_pressed = true;
                activated = false;
                press_started_at = Some(Instant::now());
                press_window = get_frontmost_window_info();
                active_stop_signal = None;
            }
            Ok(RawKeyEvent::Release) => {
                finish_press(
                    key,
                    passthrough_short_press,
                    &mut physical_pressed,
                    &mut activated,
                    &mut press_started_at,
                    &mut press_window,
                    &mut active_stop_signal,
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !physical_pressed || activated {
                    continue;
                }
                activated = true;
                let stop_signal = Arc::new(AtomicBool::new(false));
                active_stop_signal = Some(stop_signal.clone());
                let target_window = press_window
                    .clone()
                    .or_else(get_frontmost_window_info)
                    .unwrap_or(WindowInfo {
                        hwnd: 0,
                        title: String::new(),
                        process_name: String::new(),
                    });
                let _ = activation_tx.send(PttActivation {
                    target_window,
                    stop_signal,
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn finish_press(
    key: MacosPttKey,
    passthrough_short_press: bool,
    physical_pressed: &mut bool,
    activated: &mut bool,
    press_started_at: &mut Option<Instant>,
    press_window: &mut Option<WindowInfo>,
    active_stop_signal: &mut Option<Arc<AtomicBool>>,
) {
    if *activated {
        if let Some(stop_signal) = active_stop_signal.take() {
            stop_signal.store(true, Ordering::Relaxed);
        }
    } else if *physical_pressed && passthrough_short_press {
        let _ = send_key_tap(key);
    }
    *physical_pressed = false;
    *activated = false;
    *press_started_at = None;
    *press_window = None;
}

fn parse_ptt_key(name: &str) -> Result<MacosPttKey> {
    let normalized = name.trim().to_ascii_lowercase();
    if let Some(value) = parse_function_key(&normalized) {
        return Ok(MacosPttKey::Regular { key_code: value });
    }
    match normalized.as_str() {
        "space" => Ok(MacosPttKey::Regular { key_code: 49 }),
        "enter" => Ok(MacosPttKey::Regular { key_code: 36 }),
        "capslock" | "caps-lock" | "caps_lock" => Ok(MacosPttKey::CapsLock {
            key_code: MACOS_KEY_CODE_CAPS_LOCK,
        }),
        "fn" | "function" | "globe" => Ok(MacosPttKey::Fn {
            flag_mask: KCG_EVENT_FLAG_MASK_SECONDARY_FN,
        }),
        "left-win" | "left_win" | "leftwin" | "lwin" | "windows" | "win" => {
            Ok(MacosPttKey::Modifier {
                key_code: 55,
                flag_mask: KCG_EVENT_FLAG_MASK_COMMAND,
            })
        }
        "right-control" | "right_control" | "rightcontrol" | "rctrl" | "rcontrol" => {
            Ok(MacosPttKey::Modifier {
                key_code: 62,
                flag_mask: KCG_EVENT_FLAG_MASK_CONTROL,
            })
        }
        "right-shift" | "right_shift" | "rightshift" | "rshift" => Ok(MacosPttKey::Modifier {
            key_code: 60,
            flag_mask: KCG_EVENT_FLAG_MASK_SHIFT,
        }),
        _ if normalized.len() == 1 => {
            let ch = normalized.chars().next().unwrap();
            let key_code = parse_letter_key(ch)
                .ok_or_else(|| anyhow!("Unsupported single-letter macOS PTT key: {name}"))?;
            Ok(MacosPttKey::Regular { key_code })
        }
        _ => bail!("Unsupported macOS ptt key: {name}"),
    }
}

fn parse_function_key(normalized: &str) -> Option<u16> {
    match normalized {
        "f1" => Some(122),
        "f2" => Some(120),
        "f3" => Some(99),
        "f4" => Some(118),
        "f5" => Some(96),
        "f6" => Some(97),
        "f7" => Some(98),
        "f8" => Some(100),
        "f9" => Some(101),
        "f10" => Some(109),
        "f11" => Some(103),
        "f12" => Some(111),
        _ => None,
    }
}

fn parse_letter_key(ch: char) -> Option<u16> {
    match ch.to_ascii_lowercase() {
        'a' => Some(0),
        's' => Some(1),
        'd' => Some(2),
        'f' => Some(3),
        'h' => Some(4),
        'g' => Some(5),
        'z' => Some(6),
        'x' => Some(7),
        'c' => Some(8),
        'v' => Some(9),
        'b' => Some(11),
        'q' => Some(12),
        'w' => Some(13),
        'e' => Some(14),
        'r' => Some(15),
        'y' => Some(16),
        't' => Some(17),
        'o' => Some(31),
        'u' => Some(32),
        'i' => Some(34),
        'p' => Some(35),
        'l' => Some(37),
        'j' => Some(38),
        'k' => Some(40),
        'n' => Some(45),
        'm' => Some(46),
        _ => None,
    }
}

fn send_key_tap(key: MacosPttKey) -> Result<()> {
    let key_code = match key {
        MacosPttKey::CapsLock { key_code } => key_code as u16,
        MacosPttKey::Fn { .. } => return Ok(()),
        MacosPttKey::Regular { key_code } => key_code,
        MacosPttKey::Modifier { key_code, .. } => key_code as u16,
    };
    unsafe {
        let down = CGEventCreateKeyboardEvent(ptr::null_mut(), key_code, 1);
        let up = CGEventCreateKeyboardEvent(ptr::null_mut(), key_code, 0);
        if down.is_null() || up.is_null() {
            if !down.is_null() {
                CFRelease(down);
            }
            if !up.is_null() {
                CFRelease(up);
            }
            bail!("Failed to create macOS keyboard events for short-tap replay");
        }
        CGEventSetIntegerValueField(down, KCG_EVENT_SOURCE_USER_DATA, SYNTHETIC_PASSTHROUGH_TAG);
        CGEventSetIntegerValueField(up, KCG_EVENT_SOURCE_USER_DATA, SYNTHETIC_PASSTHROUGH_TAG);
        CGEventPost(KCG_SESSION_EVENT_TAP, down);
        CGEventPost(KCG_SESSION_EVENT_TAP, up);
        CFRelease(down);
        CFRelease(up);
    }
    Ok(())
}

unsafe fn create_caps_lock_hid_manager(run_loop: CFRunLoopRef) -> Option<IOHIDManagerRef> {
    let manager = unsafe { IOHIDManagerCreate(ptr::null(), KIOHID_OPTIONS_TYPE_NONE) };
    if manager.is_null() {
        return None;
    }

    let device_matching = unsafe {
        create_hid_matching_dictionary(
            b"DeviceUsagePage\0".as_ptr().cast(),
            b"DeviceUsage\0".as_ptr().cast(),
            HID_PAGE_GENERIC_DESKTOP,
            HID_USAGE_GENERIC_DESKTOP_KEYBOARD,
        )
    };
    if device_matching.is_null() {
        unsafe { CFRelease(manager) };
        return None;
    }
    unsafe {
        IOHIDManagerSetDeviceMatching(manager, device_matching);
        CFRelease(device_matching);
    }

    let input_matching = unsafe {
        create_hid_matching_dictionary(
            b"UsagePage\0".as_ptr().cast(),
            b"Usage\0".as_ptr().cast(),
            HID_PAGE_KEYBOARD_OR_KEYPAD as i32,
            HID_USAGE_KEYBOARD_CAPS_LOCK as i32,
        )
    };
    if input_matching.is_null() {
        unsafe { CFRelease(manager) };
        return None;
    }
    unsafe {
        IOHIDManagerSetInputValueMatching(manager, input_matching);
        CFRelease(input_matching);
        IOHIDManagerRegisterInputValueCallback(
            manager,
            Some(hid_input_value_callback),
            ptr::null_mut(),
        );
    }

    let result = unsafe { IOHIDManagerOpen(manager, KIOHID_OPTIONS_TYPE_NONE) };
    if result != KIO_RETURN_SUCCESS {
        unsafe { CFRelease(manager) };
        return None;
    }

    unsafe {
        IOHIDManagerScheduleWithRunLoop(manager, run_loop, kCFRunLoopCommonModes);
    }

    Some(manager)
}

unsafe fn create_hid_matching_dictionary(
    usage_page_key: *const core::ffi::c_char,
    usage_key: *const core::ffi::c_char,
    usage_page: i32,
    usage: i32,
) -> CFMutableDictionaryRef {
    let dict = unsafe {
        CFDictionaryCreateMutable(
            ptr::null(),
            0,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    };
    if dict.is_null() {
        return ptr::null_mut();
    }

    if unsafe { !set_cf_dictionary_i32(dict, usage_page_key, usage_page) }
        || unsafe { !set_cf_dictionary_i32(dict, usage_key, usage) }
    {
        unsafe { CFRelease(dict) };
        return ptr::null_mut();
    }

    dict
}

unsafe fn set_cf_dictionary_i32(
    dict: CFMutableDictionaryRef,
    key_name: *const core::ffi::c_char,
    value: i32,
) -> bool {
    let key = unsafe { CFStringCreateWithCString(ptr::null(), key_name, KCF_STRING_ENCODING_UTF8) };
    if key.is_null() {
        return false;
    }

    let number = unsafe {
        CFNumberCreate(
            ptr::null(),
            KCF_NUMBER_INT_TYPE,
            &value as *const i32 as *const core::ffi::c_void,
        )
    };
    if number.is_null() {
        unsafe { CFRelease(key) };
        return false;
    }

    unsafe {
        CFDictionarySetValue(
            dict,
            key as *const core::ffi::c_void,
            number as *const core::ffi::c_void,
        );
        CFRelease(number);
        CFRelease(key);
    }
    true
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDManagerCreate(allocator: CFAllocatorRef, options: u32) -> IOHIDManagerRef;
    fn IOHIDManagerSetDeviceMatching(manager: IOHIDManagerRef, matching: CFMutableDictionaryRef);
    fn IOHIDManagerSetInputValueMatching(
        manager: IOHIDManagerRef,
        matching: CFMutableDictionaryRef,
    );
    fn IOHIDManagerRegisterInputValueCallback(
        manager: IOHIDManagerRef,
        callback: Option<
            unsafe extern "C" fn(
                *mut core::ffi::c_void,
                IOReturn,
                *mut core::ffi::c_void,
                IOHIDValueRef,
            ),
        >,
        context: *mut core::ffi::c_void,
    );
    fn IOHIDManagerScheduleWithRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    fn IOHIDManagerUnscheduleFromRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    fn IOHIDManagerOpen(manager: IOHIDManagerRef, options: u32) -> IOReturn;
    fn IOHIDManagerClose(manager: IOHIDManagerRef, options: u32) -> IOReturn;
    fn IOHIDValueGetElement(value: IOHIDValueRef) -> IOHIDElementRef;
    fn IOHIDValueGetIntegerValue(value: IOHIDValueRef) -> CFIndex;
    fn IOHIDElementGetUsagePage(element: IOHIDElementRef) -> u32;
    fn IOHIDElementGetUsage(element: IOHIDElementRef) -> u32;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: unsafe extern "C" fn(
            CGEventTapProxy,
            CGEventType,
            CGEventRef,
            *mut core::ffi::c_void,
        ) -> CGEventRef,
        user_info: *mut core::ffi::c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: u8);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: i32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: i32, value: i64);
    fn CGEventCreateKeyboardEvent(
        source: *mut core::ffi::c_void,
        virtual_key: u16,
        key_down: u8,
    ) -> CGEventRef;
    fn CGEventPost(tap: u32, event: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: CFIndex,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRunLoopWakeUp(rl: CFRunLoopRef);
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const core::ffi::c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFDictionaryCreateMutable(
        allocator: CFAllocatorRef,
        capacity: CFIndex,
        key_callbacks: *const CFDictionaryKeyCallBacks,
        value_callbacks: *const CFDictionaryValueCallBacks,
    ) -> CFMutableDictionaryRef;
    fn CFDictionarySetValue(
        dict: CFMutableDictionaryRef,
        key: *const core::ffi::c_void,
        value: *const core::ffi::c_void,
    );
    fn CFNumberCreate(
        allocator: CFAllocatorRef,
        the_type: i32,
        value_ptr: *const core::ffi::c_void,
    ) -> CFNumberRef;
    fn CFRetain(value: *const core::ffi::c_void) -> *const core::ffi::c_void;
    fn CFRelease(value: *const core::ffi::c_void);

    static kCFRunLoopCommonModes: CFStringRef;
    static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
}
