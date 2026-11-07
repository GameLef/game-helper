//! Window snapshots and client-area physical pixels. HWND values are borrowed,
//! may be recycled by Windows, and must be rechecked before later use.

use windows::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GA_ROOT, GetAncestor, GetClassNameW, GetClientRect, GetCursorPos,
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, WindowFromPoint,
    },
};
use windows::core::{BOOL, Error, HRESULT, PWSTR};

/// An owned metadata snapshot; hwnd itself does not own the external window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    /// Opaque Win32 HWND value, safe to transport as an integer.
    pub hwnd: isize,
    /// Window title.
    pub title: String,
    /// Window class name.
    pub class: String,
    /// Executable file name, empty if access was denied.
    pub process: String,
    /// Owner PID used to reject most recycled handles.
    pub pid: u32,
}
impl WindowInfo {
    /// Recheck both HWND existence and owner PID.
    pub fn is_alive(&self) -> bool {
        if !is_window_alive(self.hwnd) {
            return false;
        }
        let mut pid = 0;
        // SAFETY: pid lives on this stack for the synchronous call.
        unsafe {
            GetWindowThreadProcessId(as_hwnd(self.hwnd), Some(&mut pid));
        }
        pid == self.pid
    }
    /// Obtain a fresh client-area snapshot.
    pub fn client_area(&self) -> Result<ClientArea, Error> {
        if !self.is_alive() {
            return Err(invalid_window());
        }
        client_area(self.hwnd)
    }
}

/// Client-area origin and extent, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientArea {
    /// Borrowed HWND value.
    pub hwnd: isize,
    /// Origin X, including negative monitor coordinates.
    pub screen_x: i32,
    /// Origin Y.
    pub screen_y: i32,
    /// Client width.
    pub width: i32,
    /// Client height.
    pub height: i32,
}
impl ClientArea {
    /// Convert a screen point inside the client to route/v1 coordinates.
    /// Degenerate extents and points outside the client are rejected.
    pub fn normalize_screen(&self, x: i32, y: i32) -> Option<(f64, f64)> {
        if self.width <= 1 || self.height <= 1 {
            return None;
        }
        let rx = i64::from(x) - i64::from(self.screen_x);
        let ry = i64::from(y) - i64::from(self.screen_y);
        if rx < 0 || ry < 0 || rx >= i64::from(self.width) || ry >= i64::from(self.height) {
            return None;
        }
        Some((
            rx as f64 / f64::from(self.width - 1),
            ry as f64 / f64::from(self.height - 1),
        ))
    }
}

fn as_hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}
fn invalid_window() -> Error {
    Error::from_hresult(HRESULT(0x8007_0006_u32 as i32))
}

/// Visible top-level windows with nonempty titles; no target activation.
pub fn list_windows() -> Result<Vec<WindowInfo>, Error> {
    let mut output = Vec::new();
    // SAFETY: EnumWindows calls synchronously; the pointer remains exclusive and
    // valid until return. The callback neither stores it nor reenters enumeration.
    unsafe {
        EnumWindows(
            Some(enum_window),
            LPARAM((&mut output as *mut Vec<WindowInfo>) as isize),
        )?;
    }
    Ok(output)
}
unsafe extern "system" fn enum_window(hwnd: HWND, param: LPARAM) -> BOOL {
    // SAFETY: The pointer was created by list_windows and outlives every callback.
    let output = unsafe { &mut *(param.0 as *mut Vec<WindowInfo>) };
    // SAFETY: Win32 queries accept borrowed handles and report invalid windows.
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let info = snapshot(hwnd);
        if !info.title.is_empty() {
            output.push(info);
        }
    }
    true.into()
}

/// Top-level visible window under a physical screen point.
pub fn window_from_point(x: i32, y: i32) -> Option<WindowInfo> {
    // SAFETY: POINT is passed by value; these calls do not retain pointers.
    let hwnd = unsafe { WindowFromPoint(POINT { x, y }) };
    if hwnd.is_invalid() {
        return None;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let root = if root.is_invalid() { hwnd } else { root };
    unsafe { IsWindowVisible(root) }
        .as_bool()
        .then(|| snapshot(root))
}

/// Mouse physical screen coordinates, or None if unavailable.
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    // SAFETY: The stack output buffer lives for the entire call.
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

/// Client origin and extent; reject invalid, minimized or empty windows.
pub fn client_area(value: isize) -> Result<ClientArea, Error> {
    let hwnd = as_hwnd(value);
    // SAFETY: All calls only query a borrowed system handle. Stack outputs are
    // valid until return; ClientToScreen cannot retain them.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() {
            return Err(invalid_window());
        }
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect)?;
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        if !ClientToScreen(hwnd, &mut origin).as_bool() {
            return Err(Error::from_thread());
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return Err(invalid_window());
        }
        Ok(ClientArea {
            hwnd: value,
            screen_x: origin.x,
            screen_y: origin.y,
            width,
            height,
        })
    }
}

/// Whether a borrowed HWND currently exists.
pub fn is_window_alive(value: isize) -> bool {
    // SAFETY: IsWindow accepts arbitrary handle values without dereferencing them.
    value != 0 && unsafe { IsWindow(Some(as_hwnd(value))) }.as_bool()
}

/// Current foreground window metadata.
pub fn foreground_window() -> Option<WindowInfo> {
    // SAFETY: No input pointers or owned handles.
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid()).then(|| snapshot(hwnd))
}

fn snapshot(hwnd: HWND) -> WindowInfo {
    let mut title = [0_u16; 1024];
    let mut class = [0_u16; 256];
    let mut pid = 0;
    // SAFETY: Fixed-size stack slices provide checked buffer lengths to windows-rs.
    let (title_len, class_len) = unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (
            GetWindowTextW(hwnd, &mut title),
            GetClassNameW(hwnd, &mut class),
        )
    };
    WindowInfo {
        hwnd: hwnd.0 as isize,
        title: String::from_utf16_lossy(&title[..title_len.max(0) as usize]),
        class: String::from_utf16_lossy(&class[..class_len.max(0) as usize]),
        process: process_name(pid),
        pid,
    }
}

fn process_name(pid: u32) -> String {
    // SAFETY: Open for limited query only; every successful handle is closed below.
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return String::new();
    };
    let mut buffer = [0_u16; 1024];
    let mut len = buffer.len() as u32;
    // SAFETY: Buffer and length live through the synchronous query.
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    // SAFETY: This function owns handle and closes it exactly once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    if result.is_err() {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..len as usize])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_owned()
}
