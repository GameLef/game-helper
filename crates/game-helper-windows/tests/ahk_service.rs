#![cfg(windows)]
//! Real bundled AHK with private titles, copied scripts and temporary config.
//! Never contacts the production daemon; the foreground lease test is opt-in.
use game_helper_core::{config::Config, storage::AppPaths};
use game_helper_windows::daemon::{DaemonClient, find_ahk};
use std::{
    fs,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetForegroundWindow, RegisterWindowMessageW,
            SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetForegroundWindow, WINDOW_EX_STYLE,
            WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        },
    },
    core::w,
};

const CONFIG: &str = r#"{"hotkeys":{"stopAll":"^!+F24","bindings":[
{"hotkey":"Ctrl+Alt+Shift+F23","route":"Earlier","action":"run_route"},
{"hotkey":"F22","route":"Disabled","enabled":false},
{"hotkey":"+!^f23","route":"Winner","enabled":true},
{"hotkey":"*F24","route":"StopConflict","enabled":true}
]}}"#;
const PROBE: &str = r#"
OnMessage(DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_PrivateTestProbe", "uint"), DAEMON_PrivateTestProbe)
DAEMON_PrivateTestProbe(operation, parameter, *) {
    switch operation {
        case 1: return DAEMON_CaptureOwner
        case 2: return DAEMON_Paused ? 1 : 0
        case 3:
            DAEMON_TogglePause()
            return DAEMON_Paused ? 1 : 0
        case 4: return DAEMON_Cursors.Has("Winner") ? DAEMON_Cursors["Winner"] : 0
        case 5:
            DAEMON_Cursors["Winner"] := 2
            DAEMON_WriteCursorState()
            return 2
        case 6: return DAEMON_Hotkeys.Has("^!+f23") && DAEMON_Hotkeys["^!+f23"].route = "Winner" ? 1 : 0
        case 7: return DAEMON_Canonical("Alt+Ctrl+Esc") = "^!escape"
            && DAEMON_Canonical("~$!^Return Up") = "^!enter up"
            && DAEMON_Canonical(">!<^F9") = "<^>!f9"
            && DAEMON_Canonical("LCtrl+RAlt+F9") = "<^>!f9"
            && DAEMON_Canonical(" LCtrl + RAlt + F9   Up ") = "<^>!f9 up"
            && DAEMON_Canonical("*^F9") != DAEMON_Canonical("^F9")
        case 8: return DAEMON_StopConflicts("*f12", "^f12")
            && DAEMON_StopConflicts("^f12", "<^f12")
            && DAEMON_StopConflicts("f12 up", "f12")
            && !DAEMON_StopConflicts("^f12", "!f12")
        default: return 0
    }
}
"#;

struct Service {
    child: Child,
    client: DaemonClient,
    dir: tempfile::TempDir,
}
impl Service {
    fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("AHK 服务 ")
            .tempdir()
            .unwrap();
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        fs::create_dir_all(dir.path().join("ahk/lib")).unwrap();
        fs::create_dir_all(dir.path().join("ahk/runtime")).unwrap();
        fs::create_dir_all(dir.path().join("routes/_cache")).unwrap();
        for name in ["JSON.ahk", "executor.ahk"] {
            fs::copy(
                project.join("ahk/lib").join(name),
                dir.path().join("ahk/lib").join(name),
            )
            .unwrap();
        }
        fs::copy(
            project.join("ahk/runtime/AutoHotkey64.exe"),
            dir.path().join("ahk/runtime/AutoHotkey64.exe"),
        )
        .unwrap();
        let title = format!(
            "GameHelper.AhkTest.{}.{}",
            std::process::id(),
            dir.path().file_name().unwrap().to_string_lossy()
        );
        let script = fs::read_to_string(project.join("ahk/daemon.ahk"))
            .unwrap()
            .replace("#SingleInstance Force", "#SingleInstance Off")
            .replace("\"game_helper_daemon_v2\"", &format!("\"{title}\""));
        fs::write(
            dir.path().join("ahk/daemon.ahk"),
            format!("{script}\n{PROBE}"),
        )
        .unwrap();
        fs::write(dir.path().join("config.json"), CONFIG).unwrap();
        fs::write(
            dir.path().join("routes/_cache/Winner.json"),
            r#"{"name":"Winner","target":{"process":"never-existing-ahk-test.exe"},"points":[{"id":1,"operations":[]},{"id":2,"operations":[]}]}"#,
        )
        .unwrap();
        let paths = AppPaths::new(dir.path()).unwrap();
        let executable = find_ahk(
            &paths,
            &Config {
                ahk_path: Some("missing-legacy-path.exe".into()),
                ..Config::default()
            },
        )
        .unwrap();
        assert!(executable.starts_with(&paths.root));
        let child = Command::new(&executable)
            .arg("/ErrorStdOut")
            .arg(dir.path().join("ahk/daemon.ahk"))
            .current_dir(std::env::temp_dir())
            .stdout(Stdio::from(
                fs::File::create(dir.path().join("stdout.log")).unwrap(),
            ))
            .stderr(Stdio::from(
                fs::File::create(dir.path().join("stderr.log")).unwrap(),
            ))
            .creation_flags(0x0000_0200 | 0x0000_0008)
            .spawn()
            .unwrap();
        let mut service = Self {
            child,
            client: DaemonClient::with_window_title(paths, title),
            dir,
        };
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(exit) = service.child.try_wait().unwrap() {
                panic!(
                    "AHK exited {exit}: {} {}",
                    service.output("stdout.log"),
                    service.output("stderr.log")
                );
            }
            if service.client.status().is_ok_and(|state| state.running) {
                // Probe is registered after the daemon's startup load has finished.
                if service.probe(7) == 1 {
                    break;
                }
            }
            assert!(
                Instant::now() < until,
                "AHK startup timeout: {} {}",
                service.output("stdout.log"),
                service.output("stderr.log")
            );
            std::thread::sleep(Duration::from_millis(30));
        }
        service
    }
    fn output(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
    }
    fn probe(&self, operation: usize) -> usize {
        let status = self.client.status().unwrap();
        if !status.running {
            return 0;
        }
        let title: Vec<u16> = self.client_title().encode_utf16().chain(Some(0)).collect();
        unsafe {
            let hwnd = windows::Win32::UI::WindowsAndMessaging::FindWindowW(
                None,
                windows::core::PCWSTR(title.as_ptr()),
            )
            .unwrap();
            let message = RegisterWindowMessageW(w!("GameHelperDaemon_PrivateTestProbe"));
            let mut reply = 0;
            let sent = SendMessageTimeoutW(
                hwnd,
                message,
                WPARAM(operation),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                700,
                Some(&mut reply),
            );
            assert_ne!(sent.0, 0);
            reply
        }
    }
    fn client_title(&self) -> String {
        format!(
            "GameHelper.AhkTest.{}.{}",
            std::process::id(),
            self.dir.path().file_name().unwrap().to_string_lossy()
        )
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.client.stop();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn bundled_service_filters_enabled_winners_and_preserves_pause_on_reload() {
    let service = Service::start();
    assert_eq!(service.client.status().unwrap().hotkeys, 1);
    assert_eq!(service.probe(6), 1, "last array binding must win");
    assert_eq!(
        service.probe(8),
        1,
        "stop overlap must include side/wildcard/up"
    );
    assert_eq!(service.probe(3), 1);
    service.client.reload().unwrap();
    assert_eq!(service.probe(2), 1, "reload must preserve manual pause");
    fs::write(service.client.paths.config(), r#"{"hotkeys":{"stopAll":"^!+F24","bindings":[{"hotkey":"+!^f23","route":"Winner","enabled":false}]}}"#).unwrap();
    service.client.reload().unwrap();
    assert_eq!(service.client.status().unwrap().hotkeys, 0);
    assert_eq!(service.probe(2), 1);
}

struct Owner(HWND);
impl Owner {
    fn new() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("GameHelper private capture owner"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                80,
                80,
                420,
                180,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        Self(hwnd)
    }
    fn foreground(&self) {
        unsafe {
            let _ = SetForegroundWindow(self.0);
        }
        let until = Instant::now() + Duration::from_secs(2);
        while unsafe { GetForegroundWindow() } != self.0 {
            assert!(
                Instant::now() < until,
                "foreground permission unavailable for private capture owner"
            );
            std::thread::sleep(Duration::from_millis(20));
            unsafe {
                let _ = SetForegroundWindow(self.0);
            }
        }
    }
    fn id(&self) -> isize {
        self.0.0 as isize
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

#[test]
#[ignore = "uses an owned foreground window; run explicitly with --ignored --test-threads=1"]
fn capture_lease_renews_expires_and_restores_pause_without_config_or_cursor_changes() {
    let service = Service::start();
    let owner = Owner::new();
    owner.foreground();
    assert_eq!(service.probe(5), 2);
    let before_config = fs::read(service.client.paths.config()).unwrap();
    let before_cursor = fs::read(service.client.paths.cursor()).unwrap();
    service.client.capture_begin(owner.id()).unwrap();
    assert_eq!(service.probe(1), owner.id() as usize);
    assert!(service.client.run_route("Winner", 1, 1).is_err());
    service.client.reload().unwrap();
    assert_eq!(service.probe(1), owner.id() as usize);
    assert_eq!(service.probe(4), 2);
    assert_eq!(
        fs::read(service.client.paths.cursor()).unwrap(),
        before_cursor
    );
    std::thread::sleep(Duration::from_millis(1100));
    service.client.capture_renew(owner.id()).unwrap();
    service.client.capture_end(owner.id()).unwrap();
    assert_eq!(service.probe(1), 0);
    assert_eq!(service.probe(2), 0);
    assert_eq!(service.probe(3), 1, "set manual pause before another lease");
    service.client.capture_begin(owner.id()).unwrap();
    std::thread::sleep(Duration::from_millis(3300));
    assert_eq!(service.probe(1), 0, "heartbeat loss must expire the lease");
    assert_eq!(service.probe(2), 1, "expiry restores the previous pause");
    assert!(service.client.capture_renew(owner.id()).is_err());
    service.client.capture_begin(owner.id()).unwrap();
    let other = Owner::new();
    other.foreground();
    assert!(service.client.capture_renew(owner.id()).is_err());
    assert_eq!(
        service.probe(1),
        0,
        "foreground loss must release the lease"
    );
    owner.foreground();
    service.client.capture_begin(owner.id()).unwrap();
    drop(owner);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        service.probe(1),
        0,
        "destroying the owner must release the lease"
    );
    assert_eq!(service.probe(2), 1);
    assert_eq!(
        fs::read(service.client.paths.config()).unwrap(),
        before_config
    );
    assert_eq!(
        fs::read(service.client.paths.cursor()).unwrap(),
        before_cursor
    );
}
