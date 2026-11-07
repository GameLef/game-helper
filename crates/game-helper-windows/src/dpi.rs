//! Set per-monitor awareness before creating GPUI windows. An embedded manifest
//! may have already selected it; access-denied alone does not prove compatibility.

use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_PER_MONITOR_AWARE,
    GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext,
    SetProcessDpiAwarenessContext,
};

/// Returns true if configured here, false if already per-monitor aware.
/// A process locked to system-aware/unaware is rejected to prevent wrong points.
pub fn try_set_per_monitor_v2() -> windows::core::Result<bool> {
    // SAFETY: Only predefined DPI context handles are passed. Called before windows.
    match unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } {
        Ok(()) => Ok(true),
        Err(error) => {
            // SAFETY: Thread context handles are read-only pseudo-handles.
            let awareness =
                unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
            if awareness == DPI_AWARENESS_PER_MONITOR_AWARE {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}
