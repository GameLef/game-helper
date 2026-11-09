//! Ctrl+Alt+P on a dedicated Win32 thread. The bounded channel never blocks the pump.
//! Shutdown uses an atomic flag and a bounded polling interval, so a failed posted
//! message cannot leave Drop waiting on GetMessage indefinitely.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::Win32::UI::{
    Input::KeyboardAndMouse::{
        GetAsyncKeyState, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
        VK_P,
    },
    WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY},
};

const HOTKEY_ID: i32 = 0x5C4A;

/// True when every keyboard key, including modifiers, has been released.
/// Mouse buttons (VK 1..=6) are intentionally excluded.
pub fn keys_released() -> bool {
    (8..=254).all(|key| !key_down(key))
}

/// Currently held nonmodifier key in stable AHK notation. OEM punctuation is
/// returned before Shift is applied; the caller supplies the modifier state.
/// If several keys are held, the lowest virtual-key code is returned.
pub fn pressed_key() -> Option<String> {
    (8..=254)
        .find(|key| !is_modifier(*key) && key_down(*key))
        .map(vk_to_ahk)
}

fn key_down(key: i32) -> bool {
    // SAFETY: GetAsyncKeyState only accepts a value, retains no pointers. Its
    // high bit is current down state; the unreliable low transition bit is ignored.
    unsafe { GetAsyncKeyState(key) as u16 & 0x8000 != 0 }
}

fn is_modifier(key: i32) -> bool {
    matches!(key, 0x10..=0x12 | 0x5b..=0x5c | 0xa0..=0xa5)
}

fn vk_to_ahk(key: i32) -> String {
    match key {
        0x30..=0x39 => (key as u8 as char).to_string(),
        0x41..=0x5a => (key as u8 as char).to_ascii_lowercase().to_string(),
        0x60..=0x69 => format!("Numpad{}", key - 0x60),
        0x70..=0x87 => format!("F{}", key - 0x6f),
        _ => match key {
            0x08 => "Backspace",
            0x09 => "Tab",
            0x0c => "Clear",
            0x0d => "Enter",
            0x13 => "Pause",
            0x14 => "CapsLock",
            0x1b => "Escape",
            0x20 => "Space",
            0x21 => "PgUp",
            0x22 => "PgDn",
            0x23 => "End",
            0x24 => "Home",
            0x25 => "Left",
            0x26 => "Up",
            0x27 => "Right",
            0x28 => "Down",
            0x2c => "PrintScreen",
            0x2d => "Insert",
            0x2e => "Delete",
            0x5d => "AppsKey",
            0x6a => "NumpadMult",
            0x6b => "NumpadAdd",
            0x6d => "NumpadSub",
            0x6e => "NumpadDot",
            0x6f => "NumpadDiv",
            0x90 => "NumLock",
            0x91 => "ScrollLock",
            0xba => ";",
            0xbb => "=",
            0xbc => ",",
            0xbd => "-",
            0xbe => ".",
            0xbf => "/",
            0xc0 => "`",
            0xdb => "[",
            0xdc => "\\",
            0xdd => "]",
            0xde => "'",
            _ => return format!("vk{key:02X}"),
        }
        .into(),
    }
}

#[cfg(test)]
mod recording_tests {
    use super::{is_modifier, vk_to_ahk};

    #[test]
    fn capture_maps_physical_punctuation_before_shift() {
        assert_eq!(vk_to_ahk(0xbb), "=");
        assert_eq!(vk_to_ahk(0xbd), "-");
        assert_eq!(vk_to_ahk(0xde), "'");
        assert_eq!(vk_to_ahk(0xdc), "\\");
        assert_eq!(vk_to_ahk(0x31), "1");
        assert_eq!(vk_to_ahk(0x41), "a");
        assert_eq!(vk_to_ahk(0x87), "F24");
        assert_eq!(vk_to_ahk(0xe2), "vkE2");
        assert!(is_modifier(0xa3));
        assert!(is_modifier(0x5c));
        assert!(!is_modifier(0x0d));
    }
}

/// A press sequence; a gap indicates dropped events when the UI was backlogged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeyEvent {
    /// Monotonically wrapping sequence, beginning with 1.
    pub seq: u64,
}

/// Registering or starting the hotkey worker failed.
#[derive(Debug)]
pub struct HotkeyError(String);
impl std::fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for HotkeyError {}

/// Owns one thread-local registration. Dropping it stops and joins its worker.
pub struct HotkeyService {
    rx: Receiver<HotkeyEvent>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HotkeyService {
    /// Waits for the registration acknowledgement. Call from a background worker.
    pub fn start() -> Result<Self, HotkeyError> {
        let (tx, rx) = mpsc::sync_channel(32);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = stopped.clone();
        let worker = thread::Builder::new()
            .name("gh-hotkey".into())
            .spawn(move || {
                let mut msg = MSG::default();
                // SAFETY: The stack MSG is valid for this call. PeekMessage explicitly
                // creates this worker's queue before registration/acknowledgement.
                unsafe {
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE);
                }
                // SAFETY: No borrowed pointers; None registers a thread-level hotkey.
                if let Err(error) = unsafe {
                    RegisterHotKey(
                        None,
                        HOTKEY_ID,
                        MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                        VK_P.0.into(),
                    )
                } {
                    let _ = ready_tx.send(Err(error.to_string()));
                    return;
                }
                if ready_tx.send(Ok(())).is_ok() {
                    let mut seq = 0_u64;
                    while !worker_stop.load(Ordering::Acquire) {
                        // SAFETY: Only this thread accesses msg and its message queue.
                        while !worker_stop.load(Ordering::Acquire)
                            && unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool()
                        {
                            if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_ID as usize {
                                seq = seq.wrapping_add(1);
                                let _ = tx.try_send(HotkeyEvent { seq });
                            }
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                // SAFETY: Registration and unregistration occur on the same thread.
                unsafe {
                    let _ = UnregisterHotKey(None, HOTKEY_ID);
                }
            })
            .map_err(|error| HotkeyError(error.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                rx,
                stopped,
                worker: Some(worker),
            }),
            response => {
                stopped.store(true, Ordering::Release);
                let _ = worker.join();
                let message = match response {
                    Ok(Err(error)) => format!("Ctrl+Alt+P 注册失败：{error}"),
                    _ => "热键线程在注册确认前退出".into(),
                };
                Err(HotkeyError(message))
            }
        }
    }

    /// Nonblocking UI-side drain.
    pub fn try_recv(&self) -> Result<HotkeyEvent, TryRecvError> {
        self.rx.try_recv()
    }

    /// Blocking wait for non-UI callers.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<HotkeyEvent, RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
