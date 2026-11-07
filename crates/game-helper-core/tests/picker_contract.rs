use game_helper_core::{
    picker::{CapturedPoint, PickerFile, PickerGroup, PickerState, PickerTarget},
    storage::{AppPaths, DiskSnapshot},
};
use serde_json::json;
use std::fs;

fn fixture() -> (tempfile::TempDir, AppPaths) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("ahk")).unwrap();
    fs::write(dir.path().join("ahk/daemon.ahk"), "").unwrap();
    fs::write(dir.path().join("config.json"), "{}").unwrap();
    let paths = AppPaths::new(dir.path()).unwrap();
    (dir, paths)
}

fn target(title: &str) -> PickerTarget {
    PickerTarget::new("GAME.EXE".into(), "游戏窗口类".into(), title.into())
}

fn point(id: u64) -> CapturedPoint {
    CapturedPoint {
        id,
        name: "中文点位 🦀\n'\"".into(),
        position: [0.0, 1.0],
    }
}

fn write_state(paths: &AppPaths, bytes: &[u8]) {
    fs::create_dir_all(paths.picker_state().parent().unwrap()).unwrap();
    fs::write(paths.picker_state(), bytes).unwrap();
}

#[test]
fn missing_state_and_no_op_save_do_not_write() {
    let (_dir, paths) = fixture();
    assert_eq!(paths.picker_state(), paths.root.join("state/picker.json"));
    let mut file = PickerFile::open(&paths).unwrap();
    assert_eq!(file.state, PickerState::default());
    assert_eq!(file.snapshot, DiskSnapshot(None));
    file.save(&paths).unwrap();
    assert!(!paths.root.join("state").exists());
}

#[test]
fn unicode_round_trip_title_isolation_and_process_case_matching() {
    let (_dir, paths) = fixture();
    let first = target("游戏 — 窗口一 🦀");
    let second = target("游戏 — 窗口二");
    let mut file = PickerFile::open(&paths).unwrap();
    file.state.set_points(first.clone(), vec![point(1)]);
    file.state
        .set_points(second.clone(), vec![point(1), point(2)]);
    assert_eq!(file.state.last_target, Some(second.clone()));
    file.save(&paths).unwrap();
    assert!(!paths.root.join("state/.backups").exists());
    let loaded = PickerFile::open(&paths).unwrap();
    assert_eq!(loaded.state, file.state);
    let mut uppercase = first.clone();
    uppercase.process = "GaMe.ExE".into();
    assert!(first.equivalent(&uppercase));
    assert_eq!(
        loaded.state.group(&uppercase).unwrap().points,
        vec![point(1)]
    );
    assert_eq!(loaded.state.group(&second).unwrap().points.len(), 2);
    let mut other_class = first.clone();
    other_class.window_class.push('A');
    assert!(loaded.state.group(&other_class).is_none());
    let mut other_title = first.clone();
    other_title.title.push('A');
    assert!(!first.equivalent(&other_title));
    assert_eq!(first.process, "game.exe");
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(paths.picker_state()).unwrap()).unwrap();
    assert_eq!(value["lastTarget"]["title"], second.title);
    assert_eq!(value["targets"][0]["target"]["windowClass"], "游戏窗口类");
}

#[test]
fn upsert_and_clear_replace_only_one_target() {
    let (_dir, paths) = fixture();
    let first = target("一");
    let second = target("二");
    let mut file = PickerFile::open(&paths).unwrap();
    file.state.set_points(first.clone(), vec![point(1)]);
    file.state.set_points(second.clone(), vec![point(2)]);
    let mut uppercase = first.clone();
    uppercase.process = "GAME.EXE".into();
    file.state.set_points(uppercase, vec![point(3)]);
    assert_eq!(file.state.targets.len(), 2);
    assert_eq!(file.state.group(&first).unwrap().points, vec![point(3)]);
    file.save(&paths).unwrap();
    file.state.set_points(first.clone(), vec![]);
    file.save(&paths).unwrap();
    let loaded = PickerFile::open(&paths).unwrap();
    assert_eq!(loaded.state.targets.len(), 2);
    assert!(loaded.state.group(&first).unwrap().points.is_empty());
    assert_eq!(loaded.state.group(&second).unwrap().points, vec![point(2)]);
    assert_eq!(loaded.state.last_target, Some(first));
}

#[test]
fn open_normalizes_processes_without_rewriting_original_bytes() {
    let (_dir, paths) = fixture();
    let value = json!({
        "format": "picker/v1",
        "lastTarget": {"process":"GAME.EXE", "windowClass":"类", "title":"标题"},
        "targets": [{"target":{"process":"GaMe.ExE", "windowClass":"类", "title":"标题"}, "points":[]}]
    });
    let bytes = serde_json::to_vec(&value).unwrap();
    let mut bom_bytes = vec![0xef, 0xbb, 0xbf];
    bom_bytes.extend_from_slice(&bytes);
    write_state(&paths, &bom_bytes);
    let mut file = PickerFile::open(&paths).unwrap();
    assert_eq!(file.state.last_target.as_ref().unwrap().process, "game.exe");
    assert_eq!(file.state.targets[0].target.process, "game.exe");
    let before = fs::metadata(paths.picker_state())
        .unwrap()
        .modified()
        .unwrap();
    file.save(&paths).unwrap();
    assert_eq!(fs::read(paths.picker_state()).unwrap(), bom_bytes);
    assert_eq!(
        fs::metadata(paths.picker_state())
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    assert_eq!(file.snapshot, DiskSnapshot(Some(bom_bytes)));
}

#[test]
fn invalid_version_coordinates_ids_and_duplicate_targets_are_rejected() {
    let (_dir, paths) = fixture();
    let base = json!({
        "format":"picker/v1", "lastTarget":null,
        "targets":[{"target":{"process":"game.exe", "windowClass":"类", "title":"标题"},
            "points":[{"id":1,"name":"", "position":[0.25,0.75]}]}]
    });
    let mut invalid = Vec::new();
    let mut version = base.clone();
    version["format"] = json!("picker/v2");
    invalid.push(version);
    for coordinate in [-0.01, 1.01] {
        let mut value = base.clone();
        value["targets"][0]["points"][0]["position"][0] = json!(coordinate);
        invalid.push(value);
    }
    let mut zero = base.clone();
    zero["targets"][0]["points"][0]["id"] = json!(0);
    invalid.push(zero);
    let mut duplicate_id = base.clone();
    let duplicate = duplicate_id["targets"][0]["points"][0].clone();
    duplicate_id["targets"][0]["points"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    invalid.push(duplicate_id);
    let mut duplicate_target = base.clone();
    let mut duplicate = duplicate_target["targets"][0].clone();
    duplicate["target"]["process"] = json!("GAME.EXE");
    duplicate_target["targets"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    invalid.push(duplicate_target);
    let mut no_identity = base.clone();
    no_identity["targets"][0]["target"]["process"] = json!("");
    no_identity["targets"][0]["target"]["windowClass"] = json!("");
    invalid.push(no_identity);
    let mut invalid_last_target = base.clone();
    invalid_last_target["lastTarget"] = json!({"process":"", "windowClass":"", "title":"标题"});
    invalid.push(invalid_last_target);
    for value in invalid {
        let bytes = serde_json::to_vec(&value).unwrap();
        write_state(&paths, &bytes);
        assert!(PickerFile::open(&paths).is_err(), "accepted {value}");
        assert_eq!(fs::read(paths.picker_state()).unwrap(), bytes);
    }
}

#[test]
fn nonfinite_coordinates_fail_before_save_and_names_are_unrestricted() {
    let (_dir, paths) = fixture();
    let mut file = PickerFile::open(&paths).unwrap();
    let mut captured = point(1);
    captured.name.clear();
    file.state.set_points(target("标题"), vec![captured]);
    assert!(file.state.validate().is_ok());
    for coordinate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        file.state.targets[0].points[0].position[1] = coordinate;
        assert!(file.state.validate().is_err());
        assert!(file.save(&paths).is_err());
        assert!(!paths.picker_state().exists());
        assert_eq!(file.snapshot, DiskSnapshot(None));
    }
    file.state.targets[0].points[0].position[1] = 0.5;
    file.save(&paths).unwrap();
    assert_eq!(
        PickerFile::open(&paths).unwrap().state.targets[0].points[0].name,
        ""
    );
}

#[test]
fn target_identity_accepts_process_or_class_and_preserves_non_ascii_case() {
    let mut state = PickerState::default();
    state.set_points(
        PickerTarget::new("game.exe".into(), "".into(), "".into()),
        vec![],
    );
    state.set_points(PickerTarget::new("".into(), "类".into(), "".into()), vec![]);
    assert!(state.validate().is_ok());
    let upper = PickerTarget::new("ÄGAME.EXE".into(), "类".into(), "标题".into());
    assert_eq!(upper.process, "Ägame.exe");
    let lower = PickerTarget::new("ägame.exe".into(), "类".into(), "标题".into());
    assert!(!upper.equivalent(&lower));
}

#[test]
fn corrupted_state_is_not_replaced_and_external_conflicts_preserve_disk() {
    let (_dir, paths) = fixture();
    let mut stale_missing = PickerFile::open(&paths).unwrap();
    write_state(&paths, b"{broken json");
    assert!(PickerFile::open(&paths).is_err());
    stale_missing
        .state
        .set_points(target("本地"), vec![point(1)]);
    assert!(stale_missing.save(&paths).is_err());
    assert_eq!(fs::read(paths.picker_state()).unwrap(), b"{broken json");
    let mut forged = PickerFile {
        state: PickerState::default(),
        snapshot: DiskSnapshot(Some(b"{broken json".to_vec())),
    };
    assert!(forged.save(&paths).is_err());
    assert_eq!(fs::read(paths.picker_state()).unwrap(), b"{broken json");
    write_state(
        &paths,
        &serde_json::to_vec(&PickerState::default()).unwrap(),
    );
    let mut first = PickerFile::open(&paths).unwrap();
    let mut second = PickerFile::open(&paths).unwrap();
    first.state.set_points(target("其他程序"), vec![point(1)]);
    first.save(&paths).unwrap();
    let external = fs::read(paths.picker_state()).unwrap();
    assert!(
        second.save(&paths).is_err(),
        "no-op must still detect a conflict"
    );
    second.state.set_points(target("本地"), vec![point(2)]);
    assert!(second.save(&paths).is_err());
    assert_eq!(fs::read(paths.picker_state()).unwrap(), external);
}

#[test]
fn repeated_no_op_saves_preserve_snapshot_mtime_and_directory_contents() {
    let (_dir, paths) = fixture();
    let mut file = PickerFile::open(&paths).unwrap();
    file.state.set_points(target("标题"), vec![point(1)]);
    file.save(&paths).unwrap();
    let before = fs::metadata(paths.picker_state())
        .unwrap()
        .modified()
        .unwrap();
    let snapshot = file.snapshot.clone();
    for _ in 0..3 {
        file.save(&paths).unwrap();
        assert_eq!(file.snapshot, snapshot);
        assert_eq!(
            fs::metadata(paths.picker_state())
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
    }
    assert_eq!(fs::read_dir(paths.root.join("state")).unwrap().count(), 1);
}

#[test]
fn direct_duplicate_groups_fail_validation_with_ascii_equivalence() {
    let mut first = target("标题");
    let second = first.clone();
    first.process = "GAME.EXE".into();
    let state = PickerState {
        targets: vec![
            PickerGroup {
                target: first,
                points: vec![],
            },
            PickerGroup {
                target: second,
                points: vec![],
            },
        ],
        ..PickerState::default()
    };
    assert!(state.validate().is_err());
}
