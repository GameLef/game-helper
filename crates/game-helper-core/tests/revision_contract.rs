use game_helper_core::{
    config::Config,
    document::{EditMode, RouteDocument},
    storage::{AppPaths, ConfigFile, DiskSnapshot, delete_route},
};
use std::fs;

fn sandbox() -> (tempfile::TempDir, AppPaths) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools/tmp");
    fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    fs::create_dir_all(dir.path().join("ahk")).unwrap();
    fs::create_dir_all(dir.path().join("routes")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let paths = AppPaths::new(dir.path()).unwrap();
    (dir, paths)
}
const ROUTE: &str =
    "# original\nformat: route/v1\nname: test\ntarget: {process: harmless.exe}\npoints: []\n";

#[test]
fn no_op_config_preserves_original_formatting_and_missing_defaults() {
    let (_dir, paths) = sandbox();
    let original = "{\n\t\"unknown\": true\n}\n";
    fs::write(paths.config(), original).unwrap();
    let mut config = ConfigFile::open(paths.config()).unwrap();
    config.save().unwrap();
    assert_eq!(fs::read_to_string(paths.config()).unwrap(), original);
    assert!(!paths.root.join(".backups").exists());
    fs::write(paths.config(), "{\"externallyChanged\":true}").unwrap();
    assert!(config.save().is_err());
}
#[test]
fn changing_binding_retains_unknown_binding_fields() {
    let mut config = Config::parse(
        br#"{"hotkeys":{"bindings":[{"hotkey":"F7","route":"test","custom":{"keep":true}}]}}"#,
    )
    .unwrap();
    config.set_binding("test", "Ctrl+Alt+F8", 2).unwrap();
    let binding = config.binding_for("test").unwrap();
    assert_eq!(binding.hotkey, "^!F8");
    assert_eq!(binding.extra["custom"]["keep"], true);
}
#[test]
fn malformed_file_is_repairable_in_source_mode_with_original_snapshot() {
    let (_dir, paths) = sandbox();
    let path = paths.route_path("broken").unwrap();
    fs::write(&path, "format: [").unwrap();
    let mut doc = RouteDocument::open_for_edit(path.clone()).unwrap();
    assert_eq!(doc.mode(), EditMode::Source);
    assert!(!doc.dirty());
    assert_eq!(doc.source(), "format: [");
    assert!(doc.switch(EditMode::Form).is_err());
    assert_eq!(doc.source(), "format: [");
    doc.set_source(ROUTE.into()).unwrap();
    doc.save(&paths, false).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), ROUTE);
}
#[test]
fn yml_without_sibling_can_be_deleted_but_stem_collision_blocks_creation() {
    let (_dir, paths) = sandbox();
    let path = paths.routes().join("only.yml");
    fs::write(&path, ROUTE).unwrap();
    assert!(paths.route_path("only").is_err());
    let expected = DiskSnapshot::read(&path).unwrap();
    let mut config = ConfigFile::open(paths.config()).unwrap();
    delete_route(&paths, &mut config, &path, &expected).unwrap();
    assert!(!path.exists());
}
