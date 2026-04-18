use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::ptr;

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, MSG, PostThreadMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_CAPITAL, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8,
    VK_F9, VK_LWIN, VK_RCONTROL, VK_RETURN, VK_RSHIFT, VK_SPACE,
};

use crate::platform::types::{PttActivation, WindowInfo};
use crate::platform::windows::paste::send_key_tap;
use crate::platform::windows::window::get_foreground_window_info;

#[derive(Debug)]
enum RawKeyEvent {
    Press,
    Release,
}

#[derive(Debug)]
struct HookState {
    vk_code: u32,
    suppress: bool,
    sender: mpsc::Sender<RawKeyEvent>,
}

fn global_hook_state() -> &'static Mutex<Option<HookState>> {
    static STATE: OnceLock<Mutex<Option<HookState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

unsafe extern "system" fn keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if (kb.flags & LLKHF_INJECTED).0 == 0 {
            if let Ok(guard) = global_hook_state().lock() {
                if let Some(state) = guard.as_ref() {
                    if kb.vkCode == state.vk_code {
                        match wparam.0 as u32 {
                            WM_KEYDOWN | WM_SYSKEYDOWN => {
                                let _ = state.sender.send(RawKeyEvent::Press);
                                if state.suppress {
                                    return LRESULT(1);
                                }
                            }
                            WM_KEYUP | WM_SYSKEYUP => {
                                let _ = state.sender.send(RawKeyEvent::Release);
                                if state.suppress {
                                    return LRESULT(1);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(Some(HHOOK(ptr::null_mut())), code, wparam, lparam) }
}

pub struct KeyboardHookHandle {
    thread_id: u32,
    join_handle: Option<JoinHandle<()>>,
}

impl Drop for KeyboardHookHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

pub struct KeyboardPttController {
    receiver: UnboundedReceiver<PttActivation>,
    exit_flag: Arc<AtomicBool>,
    hook: Option<KeyboardHookHandle>,
    worker_join: Option<JoinHandle<()>>,
}

impl KeyboardPttController {
    pub fn new(vk_code: u32, hold_ms: u64, passthrough_short_press: bool) -> Result<Self> {
        let (raw_tx, raw_rx) = mpsc::channel();
        let (hook_init_tx, hook_init_rx) = mpsc::channel();
        let join_handle = thread::spawn(move || {
            let thread_id = unsafe { GetCurrentThreadId() };

            if let Ok(mut guard) = global_hook_state().lock() {
                *guard = Some(HookState {
                    vk_code,
                    suppress: true,
                    sender: raw_tx,
                });
            }

            let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) };
            let Ok(hook) = hook else {
                if let Ok(mut guard) = global_hook_state().lock() {
                    *guard = None;
                }
                let _ = hook_init_tx.send(Err("SetWindowsHookExW failed".to_owned()));
                return;
            };
            let _ = hook_init_tx.send(Ok(thread_id));

            let mut msg = MSG::default();
            loop {
                let status = unsafe { GetMessageW(&mut msg, None, 0, 0) };
                if status.0 == 0 || status.0 == -1 {
                    break;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            unsafe {
                let _ = UnhookWindowsHookEx(hook);
            }
            if let Ok(mut guard) = global_hook_state().lock() {
                *guard = None;
            }
        });

        let thread_id = hook_init_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| anyhow!("Failed to initialize keyboard hook thread"))?
            .map_err(|error| anyhow!(error))?;

        let (activation_tx, activation_rx) = unbounded_channel();
        let exit_flag = Arc::new(AtomicBool::new(false));
        let worker_exit_flag = exit_flag.clone();
        let worker_join = thread::spawn(move || {
            worker_loop(
                raw_rx,
                activation_tx,
                worker_exit_flag,
                vk_code,
                hold_ms,
                passthrough_short_press,
            );
        });

        Ok(Self {
            receiver: activation_rx,
            exit_flag,
            hook: Some(KeyboardHookHandle {
                thread_id,
                join_handle: Some(join_handle),
            }),
            worker_join: Some(worker_join),
        })
    }

    pub async fn recv(&mut self) -> Option<PttActivation> {
        self.receiver.recv().await
    }
}

impl Drop for KeyboardPttController {
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
    vk_code: u32,
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
                press_window = get_foreground_window_info();
                active_stop_signal = None;
            }
            Ok(RawKeyEvent::Release) => {
                if activated {
                    if let Some(stop_signal) = active_stop_signal.take() {
                        stop_signal.store(true, Ordering::Relaxed);
                    }
                } else if physical_pressed && passthrough_short_press {
                    let _ = send_key_tap(vk_code);
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
                    .or_else(get_foreground_window_info)
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

pub fn parse_ptt_key(name: &str) -> Result<u32> {
    let normalized = name.trim().to_ascii_lowercase();
    if let Some(value) = parse_function_key(&normalized) {
        return Ok(value);
    }
    match normalized.as_str() {
        "space" => Ok(VK_SPACE.0 as u32),
        "enter" => Ok(VK_RETURN.0 as u32),
        "capslock" | "caps-lock" | "caps_lock" => Ok(VK_CAPITAL.0 as u32),
        "left-win" | "left_win" | "leftwin" | "lwin" | "windows" | "win" => Ok(VK_LWIN.0 as u32),
        "right-control" | "right_control" | "rightcontrol" | "rctrl" | "rcontrol" => {
            Ok(VK_RCONTROL.0 as u32)
        }
        "right-shift" | "right_shift" | "rightshift" | "rshift" => Ok(VK_RSHIFT.0 as u32),
        "fn" | "function" | "globe" => bail!(
            "Unsupported Windows ptt key: {name}. Windows does not expose Fn as a standard virtual key; use capslock, space, enter, left-win, right-control, right-shift, f1-f12, or a single letter."
        ),
        _ if normalized.len() == 1 => {
            let ch = normalized.chars().next().unwrap();
            Ok(ch.to_ascii_uppercase() as u32)
        }
        _ => bail!("Unsupported ptt key: {name}"),
    }
}

fn parse_function_key(normalized: &str) -> Option<u32> {
    match normalized {
        "f1" => Some(VK_F1.0 as u32),
        "f2" => Some(VK_F2.0 as u32),
        "f3" => Some(VK_F3.0 as u32),
        "f4" => Some(VK_F4.0 as u32),
        "f5" => Some(VK_F5.0 as u32),
        "f6" => Some(VK_F6.0 as u32),
        "f7" => Some(VK_F7.0 as u32),
        "f8" => Some(VK_F8.0 as u32),
        "f9" => Some(VK_F9.0 as u32),
        "f10" => Some(VK_F10.0 as u32),
        "f11" => Some(VK_F11.0 as u32),
        "f12" => Some(VK_F12.0 as u32),
        _ => None,
    }
}
