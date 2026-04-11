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
const KCG_EVENT_FLAG_MASK_SHIFT: CGEventFlags = 1 << 17;
const KCG_EVENT_FLAG_MASK_CONTROL: CGEventFlags = 1 << 18;
const KCG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 1 << 20;
const KCG_EVENT_SOURCE_USER_DATA: i32 = 42;
const SYNTHETIC_PASSTHROUGH_TAG: i64 = 0x0043_5642_5050_5454;

#[derive(Debug)]
enum RawKeyEvent {
    Press,
    Release,
}

#[derive(Debug, Clone, Copy)]
enum MacosPttKey {
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
                if run_loop_tx
                    .send(RunLoopHandle::from_ptr(retained_run_loop))
                    .is_err()
                {
                    CFRelease(retained_run_loop);
                    CFRelease(source);
                    CFRelease(tap);
                    if let Ok(mut guard) = global_hook_state().lock() {
                        *guard = None;
                    }
                    return;
                }
                if let Ok(mut guard) = global_hook_state().lock() {
                    *guard = Some(HookState {
                        key,
                        suppress: true,
                        modifier_pressed: false,
                        tap: EventTapHandle::from_ptr(tap),
                        sender: raw_tx,
                    });
                }
                CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, 1);
                CFRunLoopRun();
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
                if activated {
                    if let Some(stop_signal) = active_stop_signal.take() {
                        stop_signal.store(true, Ordering::Relaxed);
                    }
                } else if physical_pressed && passthrough_short_press {
                    let _ = send_key_tap(key);
                }
                physical_pressed = false;
                activated = false;
                press_started_at = None;
                press_window = None;
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

fn parse_ptt_key(name: &str) -> Result<MacosPttKey> {
    let normalized = name.trim().to_ascii_lowercase();
    if let Some(value) = parse_function_key(&normalized) {
        return Ok(MacosPttKey::Regular { key_code: value });
    }
    match normalized.as_str() {
        "space" => Ok(MacosPttKey::Regular { key_code: 49 }),
        "enter" => Ok(MacosPttKey::Regular { key_code: 36 }),
        "capslock" | "caps-lock" | "caps_lock" => bail!(
            "capslock is not supported for hold-to-talk on macOS; it is a toggle key. Use right-control, right-shift, left-win, or f1-f12 instead"
        ),
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
    fn CFRetain(value: *const core::ffi::c_void) -> *const core::ffi::c_void;
    fn CFRelease(value: *const core::ffi::c_void);

    static kCFRunLoopCommonModes: CFStringRef;
}
