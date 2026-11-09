#![cfg(windows)]
//! Runtime selection never starts AHK or depends on the developer's installation.
use game_helper_core::{config::Config, storage::AppPaths};
use game_helper_windows::daemon::find_ahk;
use std::{fs, path::PathBuf};

fn fixture() -> (tempfile::TempDir, AppPaths) {
    let dir = tempfile::Builder::new()
        .prefix("AHK 内置路径 ")
        .tempdir()
        .unwrap();
    fs::create_dir_all(dir.path().join("ahk/runtime")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let paths = AppPaths::new(dir.path()).unwrap();
    (dir, paths)
}

#[test]
fn bundled_runtime_wins_over_valid_and_stale_machine_overrides() {
    let (_dir, paths) = fixture();
    let bundled = paths.root.join("ahk/runtime/AutoHotkey64.exe");
    fs::write(&bundled, "bundled fixture, never executed").unwrap();
    let custom = paths.root.join("custom.exe");
    fs::write(&custom, "custom fixture, never executed").unwrap();
    let mut config = Config::default();
    assert_eq!(find_ahk(&paths, &config).unwrap(), bundled);
    config.ahk_path = Some(custom.to_string_lossy().into());
    assert_eq!(find_ahk(&paths, &config).unwrap(), bundled);
    config.ahk_path = Some(paths.root.join("missing.exe").to_string_lossy().into());
    assert_eq!(find_ahk(&paths, &config).unwrap(), bundled);
    config.ahk_path = Some("relative-and-stale.exe".into());
    assert_eq!(find_ahk(&paths, &config).unwrap(), bundled);
}

#[test]
fn explicit_runtime_is_a_fallback_and_relative_paths_are_rejected() {
    let (_dir, paths) = fixture();
    let custom = paths.root.join("custom.exe");
    fs::write(&custom, "fixture, never executed").unwrap();
    let mut config = Config {
        ahk_path: Some(custom.to_string_lossy().into()),
        ..Config::default()
    };
    assert_eq!(find_ahk(&paths, &config).unwrap(), custom);
    config.ahk_path = Some("custom.exe".into());
    assert!(find_ahk(&paths, &config).is_err());
    config.ahk_path = Some(paths.root.join("missing.exe").to_string_lossy().into());
    assert!(find_ahk(&paths, &config).is_err());
    config.ahk_path = Some(paths.root.to_string_lossy().into());
    assert!(find_ahk(&paths, &config).is_err());
}

#[test]
fn runtime_is_resolved_from_app_root_not_current_directory() {
    let (_dir, paths) = fixture();
    let bundled = paths.root.join("ahk/runtime/AutoHotkey64.exe");
    fs::write(&bundled, "fixture, never executed").unwrap();
    let chosen = find_ahk(&paths, &Config::default()).unwrap();
    assert_eq!(chosen, bundled);
    assert!(chosen.is_absolute());
    assert_ne!(chosen, PathBuf::from("ahk/runtime/AutoHotkey64.exe"));
}
