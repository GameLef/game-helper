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
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_P,
    },
    WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY},
};

const HOTKEY_ID: i32 = 0x5C4A;

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
