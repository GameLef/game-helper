use game_helper_core::{
    config::{Config, hotkey_to_ahk},
    document::{EditMode, RouteDocument},
    storage::{
        AppPaths, ConfigFile, CursorState, DiskSnapshot, delete_route, json_bytes, list_routes,
        read_cursor, save_atomic, validate_stem,
    },
};
use serde_json::{Value, json};
use std::fs;

const YAML: &str = "# preserve this comment\nformat: route/v1\nname: 中文路线\ntarget: {process: notepad.exe}\npoints: []\n";
fn fixture() -> (tempfile::TempDir, AppPaths, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("ahk")).unwrap();
    fs::create_dir(dir.path().join("routes")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let paths = AppPaths::new(dir.path()).unwrap();
    let route = paths.route_path("中文 -6").unwrap();
    fs::write(&route, YAML).unwrap();
    (dir, paths, route)
}
#[test]
fn opening_and_browsing_does_not_write() {
    let (_dir, paths, path) = fixture();
    let mut doc = RouteDocument::open(path.clone()).unwrap();
    doc.switch(EditMode::Source).unwrap();
    doc.switch(EditMode::Form).unwrap();
    assert!(!doc.dirty());
    assert_eq!(fs::read_to_string(path).unwrap(), YAML);
    assert!(!paths.cache().exists());
}
#[test]
fn source_save_keeps_comments_and_unknown_fields_exactly() {
    let (_dir, paths, path) = fixture();
    let mut doc = RouteDocument::open(path.clone()).unwrap();
    doc.switch(EditMode::Source).unwrap();
    let changed = format!("{YAML}custom: preserved\n").replace("中文路线", "已修改");
    doc.set_source(changed.clone()).unwrap();
    doc.save(&paths, false).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), changed);
    assert!(paths.cache_path("中文 -6").unwrap().is_file());
    assert!(!doc.dirty());
}
#[test]
fn invalid_source_cannot_replace_model_or_leave_source_view() {
    let (_dir, _, path) = fixture();
    let mut doc = RouteDocument::open(path).unwrap();
    let before = doc.route().clone();
    doc.switch(EditMode::Source).unwrap();
    doc.set_source("format: [broken".into()).unwrap();
    assert!(doc.switch(EditMode::Form).is_err());
    assert_eq!(doc.mode(), EditMode::Source);
    assert_eq!(doc.source(), "format: [broken");
    assert_eq!(*doc.route(), before);
}
#[test]
fn normalization_notice_survives_round_trip_and_backup_is_original() {
    let (_dir, paths, path) = fixture();
    let mut doc = RouteDocument::open(path.clone()).unwrap();
    doc.edit(|r| r.name = "结构化修改".into()).unwrap();
    doc.switch(EditMode::Source).unwrap();
    doc.switch(EditMode::Form).unwrap();
    assert!(doc.needs_normalization_notice());
    assert!(doc.save(&paths, false).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), YAML);
    doc.save(&paths, true).unwrap();
    let backup = fs::read_dir(path.parent().unwrap().join(".backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read_to_string(backup).unwrap(), YAML);
    assert!(!doc.needs_normalization_notice());
}
#[test]
fn conflict_does_not_overwrite_external_change() {
    let (_dir, paths, path) = fixture();
    let mut doc = RouteDocument::open(path.clone()).unwrap();
    doc.edit(|r| r.name = "本地修改".into()).unwrap();
    fs::write(&path, "external edit").unwrap();
    assert!(doc.save(&paths, true).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "external edit");
    assert!(doc.dirty());
}
#[test]
fn invalid_draft_is_memory_only() {
    let (_dir, paths, _) = fixture();
    let path = paths.route_path("草稿").unwrap();
    let mut doc = RouteDocument::draft(path.clone(), "草稿".into()).unwrap();
    assert!(doc.save(&paths, true).is_err());
    assert!(!path.exists());
}
#[test]
fn cache_failure_after_save_can_be_retried_without_false_conflict() {
    let (_dir, paths, path) = fixture();
    fs::write(paths.cache(), "blocking file").unwrap();
    let mut doc = RouteDocument::open(path.clone()).unwrap();
    doc.edit(|r| r.name = "已保存".into()).unwrap();
    let error = doc.save(&paths, true).unwrap_err();
    assert!(error.to_string().contains("路线已保存"));
    assert!(!doc.dirty());
    fs::remove_file(paths.cache()).unwrap();
    doc.save(&paths, false).unwrap();
}
#[test]
fn missing_snapshot_never_overwrites_existing_file() {
    let (_dir, _, path) = fixture();
    assert!(save_atomic(&path, &DiskSnapshot(None), b"replace", true).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), YAML);
}
#[test]
fn config_unknown_fields_survive_and_apostrophe_binding_remains() {
    let input = json!({"routesDir":"routes", "custom":{"a":1}, "hotkeys":{"stopAll":"F12", "custom":true, "bindings":[{"hotkey":"'", "route":"-6", "startAt":1, "extra":9}]}});
    let mut config = Config::parse(&serde_json::to_vec(&input).unwrap()).unwrap();
    config.auto_start_daemon = false;
    let output = serde_json::to_value(config).unwrap();
    assert_eq!(output["custom"], input["custom"]);
    assert_eq!(output["hotkeys"]["custom"], true);
    assert_eq!(output["hotkeys"]["bindings"][0]["hotkey"], "'");
    assert_eq!(output["hotkeys"]["bindings"][0]["extra"], 9);
}
#[test]
fn cursor_json_accepts_utf8_bom() {
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend_from_slice(br#"{"routes":{"-6":{"lastDone":27,"total":27,"startAt":1}}}"#);
    let value: CursorState = json_bytes(&bytes).unwrap();
    assert_eq!(value.routes["-6"].last_done, 27);
}
#[test]
fn human_and_native_ahk_hotkeys() {
    for (input, expected) in [
        ("ctrl+alt+F8", "^!F8"),
        ("F9", "F9"),
        ("'", "'"),
        ("^!p", "^!p"),
        ("", ""),
    ] {
        assert_eq!(hotkey_to_ahk(input).unwrap(), expected);
    }
    assert!(hotkey_to_ahk("ctrl+alt").is_err());
}
#[test]
fn windows_filename_rules_and_legal_stems() {
    for invalid in [
        "", "..", "CON", "lpt1.txt", "a/b", "a\\b", "bad.", "bad ", "a:b",
    ] {
        assert!(validate_stem(invalid).is_err(), "{invalid}");
    }
    for valid in ["-6", "中文路线", "route with spaces"] {
        assert!(validate_stem(valid).is_ok());
    }
}
#[test]
fn config_only_supported_routes_dir_is_accepted() {
    assert!(Config::parse(br#"{"routesDir":"elsewhere"}"#).is_err());
    let value: Value = serde_json::to_value(Config::default()).unwrap();
    assert_eq!(value["hotkeys"]["stopAll"], "F12");
}
#[test]
fn set_binding_persists_ahk_syntax_not_human_notation() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("ahk")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let mut config = ConfigFile::open(dir.path().join("config.json")).unwrap();
    config.value.set_binding("-6", "ctrl+alt+F8", 3).unwrap();
    config.save().unwrap();
    // The daemon reads this file directly: it must contain AHK syntax.
    let on_disk = fs::read_to_string(dir.path().join("config.json")).unwrap();
    assert!(
        on_disk.contains("^!F8") && !on_disk.contains("ctrl+alt+F8"),
        "{on_disk}"
    );
    let reloaded = ConfigFile::open(dir.path().join("config.json")).unwrap();
    let binding = reloaded.value.binding_for("-6").unwrap();
    assert_eq!(binding.hotkey, "^!F8");
    assert_eq!(binding.start_at, 3);
    assert!(reloaded.value.binding_for("other").is_none());
}
#[test]
fn config_save_without_changes_does_not_touch_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("ahk")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let mut config = ConfigFile::open(dir.path().join("config.json")).unwrap();
    // An actual preference change writes once and creates the initial backup.
    config.value.auto_start_daemon = false;
    config.save().unwrap();
    let time_before = fs::metadata(dir.path().join("config.json"))
        .unwrap()
        .modified()
        .unwrap();
    let snapshot_before = config.disk_snapshot().clone();
    // Repeated no-op saves must not rewrite the file or add backups.
    config.save().unwrap();
    let time_after = fs::metadata(dir.path().join("config.json"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(time_before, time_after, "无变化的重复保存不应触碰文件");
    assert_eq!(*config.disk_snapshot(), snapshot_before);
    let backups_after = fs::read_dir(dir.path().join(".backups")).unwrap().count();
    config.save().unwrap();
    assert_eq!(
        fs::read_dir(dir.path().join(".backups")).unwrap().count(),
        backups_after,
        "无变化保存不应新增备份"
    );
}
#[test]
fn read_cursor_defaults_when_daemon_has_not_stepped() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("ahk")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let paths = AppPaths::new(dir.path()).unwrap();
    // No state/ directory at all: not an error, just an empty cursor state.
    let state = read_cursor(&paths).unwrap();
    assert!(state.routes.is_empty());
}
#[test]
fn read_cursor_round_trips_written_state() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("ahk")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let paths = AppPaths::new(dir.path()).unwrap();
    fs::create_dir_all(paths.cursor().parent().unwrap()).unwrap();
    fs::write(
        &paths.cursor(),
        r#"{"routes":{"中文 -6":{"lastDone":27,"total":28,"startAt":3}}}"#,
    )
    .unwrap();
    let state = read_cursor(&paths).unwrap();
    let cursor = state.routes.get("中文 -6").unwrap();
    assert_eq!(
        (cursor.last_done, cursor.total, cursor.start_at),
        (27, 28, 3)
    );
}
#[test]
fn delete_route_removes_source_cache_and_binding_with_report() {
    let (_dir, paths, route) = fixture();
    fs::create_dir_all(paths.cache()).unwrap();
    fs::write(
        paths.cache_path("中文 -6").unwrap(),
        r#"{"format":"route/v1"}"#,
    )
    .unwrap();
    let config_path = paths.root.join("config.json");
    let mut config = ConfigFile::open(config_path).unwrap();
    config.value.set_binding("中文 -6", "F9", 2).unwrap();
    config.save().unwrap();
    let expected = DiskSnapshot::read(&route).unwrap();
    delete_route(&paths, &mut config, &route, &expected).unwrap();
    assert!(!route.exists());
    assert!(!paths.cache_path("中文 -6").unwrap().exists());
    assert!(config.value.binding_for("中文 -6").is_none());
    let backup_dir = paths.routes().join(".backups");
    let backup = fs::read_dir(&backup_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read_to_string(backup).unwrap(), YAML);
}
#[test]
fn delete_route_refuses_foreign_paths_and_yml_siblings() {
    let (_dir, paths, route) = fixture();
    let config_path = paths.root.join("config.json");
    let mut config = ConfigFile::open(config_path).unwrap();
    // Same stem, .yml extension: delete_route must never touch it.
    let yml_sibling = paths.routes().join("中文 -6.yml");
    fs::write(&yml_sibling, "untouched").unwrap();
    let expected = DiskSnapshot::read(&route).unwrap();
    delete_route(&paths, &mut config, &yml_sibling, &expected).unwrap_err();
    assert!(yml_sibling.exists(), "同 stem 的 .yml 文件不能被误删");
    assert!(route.exists());
    // A path outside routes/ is rejected even with the right stem.
    let outside = paths.root.join("中文 -6.yaml");
    fs::write(&outside, "outside").unwrap();
    let outside_snapshot = DiskSnapshot::read(&outside).unwrap();
    delete_route(&paths, &mut config, &outside, &outside_snapshot).unwrap_err();
    assert!(outside.exists());
    assert!(route.exists());
}
#[test]
fn delete_route_aborts_on_external_modification_without_deleting() {
    let (_dir, paths, route) = fixture();
    let config_path = paths.root.join("config.json");
    let mut config = ConfigFile::open(config_path).unwrap();
    let expected = DiskSnapshot::read(&route).unwrap();
    fs::write(&route, "external edit").unwrap();
    delete_route(&paths, &mut config, &route, &expected).unwrap_err();
    assert_eq!(fs::read_to_string(&route).unwrap(), "external edit");
    assert!(config.value.binding_for("中文 -6").is_none());
}
#[test]
fn list_routes_reports_single_broken_file_without_losing_the_rest() {
    let (_dir, paths, _) = fixture();
    fs::write(paths.route_path("好的").unwrap(), YAML).unwrap();
    fs::write(paths.route_path("坏的").unwrap(), "format: [broken").unwrap();
    fs::write(paths.routes().join("中文 路线.yml"), YAML).unwrap();
    fs::write(paths.routes().join(".hidden.yaml"), YAML).unwrap();
    fs::write(paths.routes().join("txt.txt"), "ignored").unwrap();
    let entries = list_routes(&paths).unwrap();
    let stems = entries.iter().map(|e| e.stem.as_str()).collect::<Vec<_>>();
    // Byte-order listing: 坏(U+574F) < 好(U+597D), deterministic across runs.
    assert_eq!(
        stems,
        vec!["中文 -6", "中文 路线", "坏的", "好的"],
        "点前缀隐藏与非 YAML 文件不进入列表: {stems:?}"
    );
    let broken = entries.iter().find(|e| e.stem == "坏的").unwrap();
    assert!(broken.error.is_some());
    let good = entries.iter().find(|e| e.stem == "好的").unwrap();
    assert!(good.error.is_none());
    assert_eq!(good.points, 0);
}
#[test]
fn import_requires_acknowledgement_then_saves_over_existing_with_backup() {
    let (_dir, paths, route) = fixture();
    let incoming = "# incoming comment\nformat: route/v1\nname: 导入路线\ntarget: {process: notepad.exe}\npoints: []\n";
    // Existing target without acknowledgement must abort, untouched.
    assert!(RouteDocument::from_source(route.clone(), incoming, false).is_err());
    assert_eq!(fs::read_to_string(&route).unwrap(), YAML);
    // Acknowledged: nothing is written yet, but the current target bytes are
    // recorded as the document's snapshot for conflict detection.
    let mut doc = RouteDocument::from_source(route.clone(), incoming, true).unwrap();
    assert_eq!(doc.source(), incoming);
    assert!(doc.dirty());
    assert_eq!(fs::read_to_string(&route).unwrap(), YAML);
    // External modification after acknowledgement blocks the overwrite.
    fs::write(&route, "external edit").unwrap();
    assert!(doc.save(&paths, false).is_err());
    assert_eq!(fs::read_to_string(&route).unwrap(), "external edit");
    // Retry with the fresh snapshot: commit goes through with backup of the
    // bytes that are being replaced (here the external edit, not the original).
    let mut doc = RouteDocument::from_source(route.clone(), incoming, true).unwrap();
    doc.save(&paths, false).unwrap();
    assert_eq!(fs::read_to_string(&route).unwrap(), incoming);
    let backup = fs::read_dir(paths.routes().join(".backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| fs::read_to_string(p).unwrap() == "external edit")
        .expect("被替换的字节应有备份");
    let _ = backup;
}
#[test]
fn import_into_free_target_saves_as_new_route() {
    let (_dir, paths, _) = fixture();
    let target = paths.route_path("导入新路线").unwrap();
    let mut doc = RouteDocument::from_source(target.clone(), YAML, false).unwrap();
    assert!(doc.dirty());
    doc.save(&paths, false).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), YAML);
    assert!(paths.cache_path("导入新路线").unwrap().is_file());
}
#[test]
fn round_trip_source_form_source_preserves_dirty_state_and_notice() {
    let (_dir, paths, route) = fixture();
    let mut doc = RouteDocument::open(route.clone()).unwrap();
    // Structured edit keeps the model as the source of truth across views.
    doc.edit(|r| r.name = "结构化修改".into()).unwrap();
    doc.switch(EditMode::Source).unwrap();
    doc.switch(EditMode::Form).unwrap();
    assert!(doc.dirty(), "往返后仍是脏文档，编辑不能丢失");
    assert_eq!(doc.route().name, "结构化修改");
    assert!(
        doc.needs_normalization_notice(),
        "往返后保存会覆盖磁盘原文，必须提醒"
    );
    // The generated export is still on screen: saving without acknowledgement
    // must keep refusing even after the round trip.
    assert!(doc.save(&paths, false).is_err());
    assert_eq!(fs::read_to_string(&route).unwrap(), YAML);
    doc.save(&paths, true).unwrap();
    assert!(!doc.dirty());
    assert!(!doc.needs_normalization_notice());
    // Typing in Source view is authoring: no normalization notice.
    let mut doc = RouteDocument::open(route.clone()).unwrap();
    doc.switch(EditMode::Source).unwrap();
    doc.set_source(fs::read_to_string(&route).unwrap()).unwrap();
    doc.switch(EditMode::Form).unwrap();
    assert!(!doc.needs_normalization_notice());
    // Pure browsing never rewrites or marks dirty.
    let before = fs::read_to_string(&route).unwrap();
    let mut doc = RouteDocument::open(route.clone()).unwrap();
    doc.switch(EditMode::Source).unwrap();
    doc.switch(EditMode::Form).unwrap();
    assert!(!doc.dirty() && !doc.needs_normalization_notice());
    assert_eq!(fs::read_to_string(&route).unwrap(), before);
}
#[test]
fn structured_edit_notice_covers_unknown_field_loss_and_clears_on_identical_generation() {
    let (_dir, paths, route) = fixture();
    let mut doc = RouteDocument::open(route.clone()).unwrap();
    doc.edit(|r| r.name = "未知字段测试".into()).unwrap();
    assert!(
        doc.needs_normalization_notice(),
        "磁盘原文含注释/未知字段，结构化保存必然重写"
    );
    doc.save(&paths, true).unwrap();
    // After save the generated text IS the disk text: editing again to the
    // same content regenerates identical bytes, so no notice is needed.
    let mut doc = RouteDocument::open(route.clone()).unwrap();
    doc.edit(|r| r.name = "未知字段测试".into()).unwrap();
    assert!(
        !doc.needs_normalization_notice(),
        "生成文本与磁盘一致时不应打扰用户"
    );
    doc.save(&paths, true).unwrap();
}
