//! Legacy AHK protocol client. Blocking methods belong on background workers.
use crate::daemon_protocol::{DAEMON_WINDOW_TITLE, MSG_NAME_PING, MSG_NAME_RELOAD, MSG_NAME_STOP};
use anyhow::{Context, Result, ensure};
use game_helper_core::{
    config::Config,
    storage::{AppPaths, validate_stem},
};
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::DataExchange::COPYDATASTRUCT,
        UI::WindowsAndMessaging::{
            FindWindowW, GetWindowThreadProcessId, RegisterWindowMessageW, SMTO_ABORTIFHUNG,
            SMTO_BLOCK, SendMessageTimeoutW, WM_COPYDATA,
        },
    },
    core::PCWSTR,
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: u32,
    pub hotkeys: usize,
    pub busy: bool,
}
impl DaemonStatus {
    pub fn decode(pid: u32, value: usize) -> Self {
        Self {
            running: true,
            pid,
            hotkeys: (value >> 1) & 0x7fffffff,
            busy: value & 1 != 0,
        }
    }
}

/// Make the exact UTF-8 payload (byte count includes its terminal NUL).
pub fn run_payload(stem: &str, start_at: usize, only: usize) -> Result<Vec<u8>> {
    validate_stem(stem)?;
    ensure!(start_at > 0, "起始点必须 ≥1");
    let mut bytes =
        serde_json::to_vec(&serde_json::json!({"route":stem, "startAt":start_at, "only":only}))?;
    bytes.push(0);
    Ok(bytes)
}

#[derive(Clone, Debug)]
pub struct DaemonClient {
    pub paths: AppPaths,
    title: String,
    timeout_ms: u32,
}
impl DaemonClient {
    pub fn new(paths: AppPaths) -> Self {
        Self::with_window_title(paths, DAEMON_WINDOW_TITLE)
    }
    /// Separate title for isolated protocol tests; never stop/reload a live user's daemon.
    pub fn with_window_title(paths: AppPaths, title: impl Into<String>) -> Self {
        Self {
            paths,
            title: title.into(),
            timeout_ms: 700,
        }
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_ms = timeout.as_millis().clamp(1, 30_000) as u32;
        self
    }
    fn hwnd(&self) -> Option<HWND> {
        let title = wide(&self.title);
        // SAFETY: Title buffer remains valid through the synchronous Win32 query.
        unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }.ok()
    }
    fn message_id(name: &str) -> Result<u32> {
        let name = wide(name);
        // SAFETY: NUL-terminated UTF-16 input lives through the call.
        let id = unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) };
        ensure!(id != 0, "注册 daemon 窗口消息失败");
        Ok(id)
    }
    fn send(&self, hwnd: HWND, message: u32, parameter: LPARAM) -> Result<usize> {
        let mut value = 0;
        // SAFETY: The caller retains any pointed-to COPYDATA buffer until this
        // synchronous send completes. WM_COPYDATA is marshalled by Windows for
        // cross-process delivery; no raw pointers are posted asynchronously.
        let sent = unsafe {
            SendMessageTimeoutW(
                hwnd,
                message,
                WPARAM(0),
                parameter,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                self.timeout_ms,
                Some(&mut value),
            )
        };
        ensure!(
            sent.0 != 0,
            "daemon 通信超时、权限不足或窗口退出；执行请求的结果可能未知，未自动重发"
        );
        Ok(value)
    }
    pub fn status(&self) -> Result<DaemonStatus> {
        let Some(hwnd) = self.hwnd() else {
            return Ok(DaemonStatus::default());
        };
        let value = self.send(hwnd, Self::message_id(MSG_NAME_PING)?, LPARAM(0))?;
        let mut pid = 0;
        // SAFETY: Output PID is local and valid throughout the call.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        Ok(DaemonStatus::decode(pid, value))
    }
    pub fn start(&self, config: &Config) -> Result<()> {
        if self.hwnd().is_some() {
            self.status()?;
            return Ok(());
        }
        ensure!(
            self.title == DAEMON_WINDOW_TITLE,
            "自定义标题仅用于隔离联调，不能启动生产 daemon"
        );
        let executable = find_ahk(config)?;
        let mut child = Command::new(executable)
            .arg(self.paths.root.join("ahk/daemon.ahk"))
            .current_dir(self.paths.root.join("ahk"))
            .creation_flags(0x0000_0200 | 0x0000_0008)
            .spawn()
            .context("启动 AutoHotkey daemon 失败")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.hwnd().is_some() {
                self.status()?;
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                anyhow::bail!("daemon 提前退出：{status}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        anyhow::bail!("daemon 启动超过 5 秒，尚未发现消息窗口；不会重复拉起")
    }
    pub fn stop(&self) -> Result<()> {
        let Some(hwnd) = self.hwnd() else {
            return Ok(());
        };
        ensure!(
            self.send(hwnd, Self::message_id(MSG_NAME_STOP)?, LPARAM(0))? == 1,
            "daemon 拒绝停止请求"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if self.hwnd().is_none() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        anyhow::bail!("停止 daemon 超时，尚未确认退出；独立 runner 不受此操作影响")
    }
    pub fn reload(&self) -> Result<()> {
        let hwnd = self.hwnd().context("daemon 未运行")?;
        ensure!(!self.status()?.busy, "daemon 正在执行，空闲后才能重载");
        ensure!(
            self.send(hwnd, Self::message_id(MSG_NAME_RELOAD)?, LPARAM(0))? == 1,
            "daemon 拒绝重载"
        );
        Ok(())
    }
    /// Acknowledgement means accepted only, never that playback completed.
    pub fn run_route(&self, stem: &str, start_at: usize, only: usize) -> Result<()> {
        let bytes = run_payload(stem, start_at, only)?;
        let hwnd = self.hwnd().context("daemon 未运行")?;
        ensure!(!self.status()?.busy, "daemon 正在执行，不能重复提交路线");
        let packet = COPYDATASTRUCT {
            dwData: 1,
            cbData: bytes.len().try_into()?,
            lpData: bytes.as_ptr().cast_mut().cast(),
        };
        let accepted = self.send(
            hwnd,
            WM_COPYDATA,
            LPARAM((&packet as *const COPYDATASTRUCT) as isize),
        )?;
        ensure!(accepted == 1, "daemon 拒绝执行请求，请检查路线缓存");
        Ok(())
    }
    pub fn spawn_runner(
        &self,
        config: &Config,
        cache: &Path,
        start_at: usize,
        only: usize,
    ) -> Result<Child> {
        ensure!(
            start_at > 0 && cache.is_file(),
            "起始点无效或执行缓存不存在"
        );
        let mut command = Command::new(find_ahk(config)?);
        command
            .arg(self.paths.root.join("ahk/runner.ahk"))
            .arg(cache);
        if start_at > 1 {
            command.args(["--start", &start_at.to_string()]);
        }
        if only > 0 {
            command.args(["--only", &only.to_string()]);
        }
        command.args(["--stop", &config.hotkeys.stop_all]);
        command
            .current_dir(self.paths.root.join("ahk"))
            .creation_flags(0x0000_0200)
            .spawn()
            .context("启动 runner 失败")
    }
}

pub fn find_ahk(config: &Config) -> Result<PathBuf> {
    if let Some(path) = config.ahk_path.as_ref().filter(|s| !s.trim().is_empty()) {
        let path = PathBuf::from(path);
        ensure!(
            path.is_absolute() && path.is_file(),
            "config.json 中 ahkPath 不存在或不是绝对路径：{}",
            path.display()
        );
        return Ok(path);
    }
    let mut roots = Vec::new();
    for name in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(name) {
            roots.push(PathBuf::from(root).join("AutoHotkey"));
        }
    }
    if let Ok(key) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\AutoHotkey")
    {
        if let Ok(path) = key.get_value::<String, _>("InstallDir") {
            roots.push(path.into());
        }
    }
    for root in roots {
        for file in ["v2/AutoHotkey64.exe", "v2/AutoHotkey.exe"] {
            let path = root.join(file);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    anyhow::bail!("未找到 AutoHotkey v2，请设置 config.json 的 ahkPath")
}
