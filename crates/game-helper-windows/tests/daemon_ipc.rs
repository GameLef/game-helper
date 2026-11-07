#![cfg(windows)]
//! Each protocol server runs in its own child test process with a unique title.
//! No default daemon window or AHK executable is contacted or started.
use game_helper_core::storage::AppPaths;
use game_helper_windows::{
    daemon::{DaemonClient, DaemonStatus, run_payload},
    instance::InstanceGuard,
};
use std::{
    fs,
    process::{Child, Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::DataExchange::COPYDATASTRUCT,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_WNDPROC,
            GetMessageW, MSG, PostQuitMessage, RegisterWindowMessageW, SetWindowLongPtrW,
            WINDOW_EX_STYLE, WINDOW_STYLE, WM_COPYDATA,
        },
    },
    core::{PCWSTR, w},
};

static IDS: OnceLock<[u32; 3]> = OnceLock::new();
static RECORD: OnceLock<std::path::PathBuf> = OnceLock::new();
static SLEEP_MS: OnceLock<u64> = OnceLock::new();
static BUSY: OnceLock<bool> = OnceLock::new();

unsafe extern "system" fn handler(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(ids) = IDS.get() else {
        return unsafe { DefWindowProcW(hwnd, message, wp, lp) };
    };
    if message == ids[0] {
        std::thread::sleep(Duration::from_millis(*SLEEP_MS.get().unwrap_or(&0)));
        return LRESULT((3 << 1) | usize::from(*BUSY.get().unwrap_or(&false)) as isize);
    }
    if message == ids[1] {
        return LRESULT(1);
    }
    if message == ids[2] {
        unsafe {
            PostQuitMessage(0);
        }
        return LRESULT(1);
    }
    if message == WM_COPYDATA {
        // SAFETY: WM_COPYDATA marshals this packet from the parent process. Copy
        // its bytes now; never retain the pointer after the native handler returns.
        let packet = unsafe { &*(lp.0 as *const COPYDATASTRUCT) };
        if packet.dwData != 1
            || packet.cbData == 0
            || packet.cbData > 4096
            || packet.lpData.is_null()
        {
            return LRESULT(0);
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(packet.lpData.cast::<u8>(), packet.cbData as usize)
        };
        if bytes.last() != Some(&0) {
            return LRESULT(0);
        }
        let content = &bytes[..bytes.len() - 1];
        if serde_json::from_slice::<serde_json::Value>(content).is_err() {
            return LRESULT(0);
        }
        return LRESULT(
            if RECORD.get().is_some_and(|p| fs::write(p, content).is_ok()) {
                1
            } else {
                0
            },
        );
    }
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}

#[test]
#[ignore = "private child server, started only by the parent integration tests"]
fn helper_server() {
    let Ok(title) = std::env::var("GH_IPC_TEST_TITLE") else {
        return;
    };
    RECORD
        .set(std::env::var_os("GH_IPC_TEST_RECORD").unwrap().into())
        .unwrap();
    SLEEP_MS
        .set(std::env::var("GH_IPC_TEST_SLEEP").unwrap().parse().unwrap())
        .unwrap();
    BUSY.set(std::env::var("GH_IPC_TEST_BUSY").unwrap() == "1")
        .unwrap();
    let names = [
        w!("GameHelperDaemon_Ping"),
        w!("GameHelperDaemon_Reload"),
        w!("GameHelperDaemon_Stop"),
    ];
    IDS.set(names.map(|name| unsafe { RegisterWindowMessageW(name) }))
        .unwrap();
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY: STATIC is a built-in class. This hidden window lives on the thread
    // pumping it. The callback remains a static function until window destruction.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR(title.as_ptr()),
            WINDOW_STYLE::default(),
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
    }
    .unwrap();
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_WNDPROC, handler as *const () as isize);
    }
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe {
            DispatchMessageW(&message);
        }
    }
    unsafe {
        DestroyWindow(hwnd).unwrap();
    }
}

struct Server {
    child: Child,
    client: DaemonClient,
    dir: tempfile::TempDir,
}
impl Server {
    fn start(sleep: u64, busy: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("ahk")).unwrap();
        fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
        fs::write(dir.path().join("config.json"), "{}").unwrap();
        let paths = AppPaths::new(dir.path()).unwrap();
        let title = format!(
            "GameHelper.RustTest.{}.{}",
            std::process::id(),
            dir.path().file_name().unwrap().to_string_lossy()
        );
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "helper_server", "--nocapture"])
            .env("GH_IPC_TEST_TITLE", &title)
            .env("GH_IPC_TEST_RECORD", dir.path().join("request.json"))
            .env("GH_IPC_TEST_SLEEP", sleep.to_string())
            .env("GH_IPC_TEST_BUSY", if busy { "1" } else { "0" })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let client =
            DaemonClient::with_window_title(paths, &title).with_timeout(Duration::from_millis(700));
        let mut server = Self { child, client, dir };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if server.client.status().is_ok_and(|s| s.running) {
                return server;
            }
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "test server exited early"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("isolated IPC server did not start");
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn payload_utf8_and_nul() {
    let bytes = run_payload("中文 -6", 2, 0).unwrap();
    assert_eq!(bytes.last(), Some(&0));
    let value: serde_json::Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
    assert_eq!(value["route"], "中文 -6");
    assert_eq!(value["startAt"], 2);
    assert_eq!(value["only"], 0);
    assert!(run_payload("../bad", 1, 0).is_err());
    assert!(run_payload("ok", 0, 0).is_err());
}
#[test]
fn ping_decode() {
    assert_eq!(
        DaemonStatus::decode(7, 7),
        DaemonStatus {
            running: true,
            pid: 7,
            hotkeys: 3,
            busy: true
        }
    );
    assert_eq!(DaemonStatus::decode(7, 0).hotkeys, 0);
}
#[test]
fn native_ipc_round_trip_and_stop() {
    let mut server = Server::start(0, false);
    assert_eq!(server.client.status().unwrap().hotkeys, 3);
    server.client.reload().unwrap();
    server.client.run_route("中文 -6", 2, 4).unwrap();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(server.dir.path().join("request.json")).unwrap()).unwrap();
    assert_eq!(record["route"], "中文 -6");
    assert_eq!(record["only"], 4);
    server.client.stop().unwrap();
    assert!(!server.client.status().unwrap().running);
    let _ = server.child.wait();
}
#[test]
fn busy_refuses_reload_and_run() {
    let server = Server::start(0, true);
    assert!(server.client.reload().is_err());
    assert!(server.client.run_route("ok", 1, 0).is_err());
    assert!(!server.dir.path().join("request.json").exists());
}
#[test]
fn hung_ping_has_bounded_timeout() {
    let server = Server::start(250, false);
    let client = server
        .client
        .clone()
        .with_timeout(Duration::from_millis(30));
    let start = Instant::now();
    assert!(client.status().is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn instance_guard_releases_and_separates_modes() {
    let dir = tempfile::tempdir().unwrap();
    let first = InstanceGuard::acquire(dir.path(), false).unwrap();
    assert!(InstanceGuard::acquire(dir.path(), false).is_err());
    let picker = InstanceGuard::acquire(dir.path(), true).unwrap();
    drop(first);
    assert!(InstanceGuard::acquire(dir.path(), false).is_ok());
    drop(picker);
}
