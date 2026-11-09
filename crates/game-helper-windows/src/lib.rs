//! game-helper-windows：game_helper v2 的 **Windows 平台层**（Phase 1 真实实现）。
//!
//! 阶段边界（只做平台最小验证，不碰业务）：
//! - [`dpi`]：进程级 per-monitor DPI aware（与 AHK executor 的物理像素坐标系对齐，
//!   否则高 DPI 下归一化坐标错位——Phase 0 调查报告 §7 / docs/route-format.md L129）；
//! - [`window`]：窗口枚举 / 命中 / 客户区 / 归一化坐标（`(w-1)` 公式，与 AHK
//!   `EXE_ToScreen`（executor.ahk L83-91）互逆）；
//! - [`hotkey`]：`RegisterHotKey` Ctrl+Alt+P 专用线程 RAII 服务（MOD_NOREPEAT，
//!   事件走有界 channel 可 `try_recv`，`Drop` 可靠注销且不死等消息泵）。
//!
//! 不在 Phase 1 实现：路线/配置业务、输入注入、GPUI 界面。
//!
//! Phase 3 新增（AHK 生命周期与 Win32 IPC）：
//! - [`daemon`]：[`daemon::DaemonClient`] —— daemon 窗口消息 IPC 客户端与 AHK
//!   生命周期管理（status/start/stop/reload/run_route、录制租约、find_ahk）。
//!   协议契约见模块文档与 `daemon_protocol`。
//! - [`instance`]：[`instance::InstanceGuard`] —— 按 app root + 模式区分的
//!   命名互斥量单实例守卫（RAII CloseHandle；已存在即报错，不结束他人进程）。
//!
//! # Win32 句柄/指针生命周期（安全约定）
//!
//! - `HWND` 本模块对外一律以 `isize`/`usize` 存储（[`WindowInfo::hwnd`] /
//!   [`ClientArea::hwnd`]），保证 `Send`/`Copy` 可跨线程传给 GPUI；
//!   窗口句柄不是进程内堆指针，**不拥有引用计数**，它仅在整个 Windows 会话期间
//!   由系统保证稳定；使用前必须经 [`WindowInfo::is_alive`] 或
//!   [`is_window_alive`](window::is_window_alive) 复核（窗口可能已被用户关闭，
//!   句柄值可能被系统复用指向另一个窗口——这是句柄语义，不是悬垂指针）。
//! - `HANDLE`（进程句柄）：本模块内通过 `OpenProcess` 获取、作用域内
//!   `CloseHandle` 关闭，**不跨作用域泄漏**；不对外输出。
//! - `WNDENUMPROC` 回调指针：`EnumWindows` 是同步调用，回调仅在调用期间被
//!   user32 串行调用，`lParam` 携带的 `&mut Vec` 指针的生命周期严格覆盖整个
//!   `EnumWindows` 调用（回调不可能晚于调用返回而存活），因此裸指针转型是安全的。
//! - `crate::window::client_area` 返回的 `ClientArea` 是纯数值快照（坐标/尺寸），
//!   复制后与窗口状态无关；窗口移动/缩放后旧快照失效，需要重新获取。

#![cfg_attr(not(windows), allow(unused_imports))]
#![warn(missing_docs)]

/// Rust 管理器与 AHK daemon 的窗口消息 IPC 协议常量。
///
/// [`daemon::DaemonClient`] 使用的协议契约：
/// - 窗口标题：`game_helper_daemon_v2`；
/// - `RegisterWindowMessage`：`GameHelperDaemon_Ping` / `_Reload` / `_Stop`
///   （ID 是会话级动态分配，两端只能靠相同字符串匹配，禁止硬编码数值）；
/// - `WM_COPYDATA` `dwData=1`：UTF-8 JSON `{route,startAt,only}`（含 `\0` 终止，
///   cbData 含 NUL）；daemon 侧 `SetTimer(-1)` 延迟执行后立即返回 1=受理；
/// - Ping 返回值位编码：`(热键数 << 1) | (busy ? 1 : 0)`，热键数掩 `0x7FFFFFFF`。
///
/// Rust 侧使用 `SendMessageTimeout(SMTO_ABORTIFHUNG)`，避免 daemon 无响应时
/// 无限阻塞消息发送。
pub mod daemon_protocol {
    /// daemon 隐藏消息窗口标题（`FindWindowW(None, _)`）。
    pub const DAEMON_WINDOW_TITLE: &str = "game_helper_daemon_v2";
    /// `RegisterWindowMessageW` 字符串：心跳。
    pub const MSG_NAME_PING: &str = "GameHelperDaemon_Ping";
    /// `RegisterWindowMessageW` 字符串：重载 config/热键/路线并复位游标。
    pub const MSG_NAME_RELOAD: &str = "GameHelperDaemon_Reload";
    /// `RegisterWindowMessageW` 字符串：优雅退出。
    pub const MSG_NAME_STOP: &str = "GameHelperDaemon_Stop";
    /// `RegisterWindowMessageW`: begin capture, `wParam=owner HWND`, `lParam=0`.
    /// Returns 1 only for an idle service and a live foreground owner; route
    /// hotkeys are suppressed while emergency stop remains available.
    pub const MSG_NAME_CAPTURE_BEGIN: &str = "GameHelperDaemon_CaptureBegin";
    /// Renew the same foreground owner's capture lease; 1=renewed, 0=rejected.
    pub const MSG_NAME_CAPTURE_RENEW: &str = "GameHelperDaemon_CaptureRenew";
    /// End this owner's capture; 1=ended/already absent, 0=another owner holds it.
    pub const MSG_NAME_CAPTURE_END: &str = "GameHelperDaemon_CaptureEnd";
    /// Service-side lease expiry; loss of owner/focus restores sooner.
    pub const CAPTURE_LEASE_MS: u64 = 3_000;
    /// Manager heartbeat interval; failure must end local key recording.
    pub const CAPTURE_HEARTBEAT_MS: u64 = 1_000;
    /// `WM_COPYDATA`，`u32` 值 `0x004A`。
    pub const WM_COPYDATA: u32 = 0x004A;
    /// `WM_COPYDATA` 的 `dwData` magic，`1` 表示运行请求。
    pub const COPYDATA_RUN_MAGIC: usize = 1;
}

#[cfg(windows)]
pub mod daemon;
#[cfg(windows)]
pub mod dpi;
#[cfg(windows)]
pub mod hotkey;
#[cfg(windows)]
pub mod instance;
#[cfg(windows)]
pub mod window;
