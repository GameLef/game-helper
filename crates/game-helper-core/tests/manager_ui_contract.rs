use game_helper_core::{
    manager_ui::{ManagerUiFile, ManagerUiState, PointUiKey, normalize_route_key},
    route::{OpKind, Operation, Point},
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

fn operation(name: &str) -> Operation {
    let mut operation = Operation::new(OpKind::Wait, [0.5, 0.5]);
    operation.name = name.into();
    operation
}

fn write_state(paths: &AppPaths, bytes: &[u8]) {
    fs::create_dir_all(paths.manager_ui_state().parent().unwrap()).unwrap();
    fs::write(paths.manager_ui_state(), bytes).unwrap();
}

fn populated() -> ManagerUiState {
    let mut state = ManagerUiState::default();
    let operations = state
        .reconcile_point(
            "routes/测试.yaml",
            PointUiKey::new(42, 0),
            &[operation("中文动作 🦀")],
        )
        .unwrap();
    state
        .put_point("routes/测试.yaml", PointUiKey::new(42, 0), operations)
        .unwrap();
    state
}

#[test]
fn missing_state_and_noop_save_do_not_create_anything() {
    let (_dir, paths) = fixture();
    assert_eq!(
        paths.manager_ui_state(),
        paths.root.join("state/manager-ui.json")
    );
    let mut file = ManagerUiFile::open(&paths).unwrap();
    assert_eq!(file.state, ManagerUiState::default());
    assert_eq!(file.snapshot, DiskSnapshot(None));
    file.save(&paths).unwrap();
    assert!(!paths.root.join("state").exists());
}

#[test]
fn route_keys_are_relative_portable_case_folded_and_cannot_escape_root() {
    let (_dir, paths) = fixture();
    assert_eq!(
        normalize_route_key(&paths, "Routes\\Folder\\..\\ÄBC.YAML").unwrap(),
        "routes/äbc.yaml"
    );
    assert_eq!(
        normalize_route_key(&paths, paths.root.join("Routes/ÄBC.YAML")).unwrap(),
        "routes/äbc.yaml"
    );
    let uppercase = paths
        .root
        .join("Routes/ABC.YAML")
        .to_str()
        .unwrap()
        .to_uppercase();
    assert_eq!(
        normalize_route_key(&paths, uppercase).unwrap(),
        "routes/abc.yaml"
    );
    for invalid in [
        "",
        "..",
        "../outside.yaml",
        "routes/../../outside.yaml",
        "routes/x\ny.yaml",
    ] {
        assert!(
            normalize_route_key(&paths, invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
    assert!(
        normalize_route_key(&paths, paths.root.parent().unwrap().join("outside.yaml")).is_err()
    );
    assert!(
        normalize_route_key(
            &paths,
            format!("{}-other/routes/x.yaml", paths.root.display())
        )
        .is_err()
    );
}

#[test]
fn duplicate_point_ids_and_duplicate_operations_keep_distinct_states() {
    let points = vec![Point::new(8), Point::new(3), Point::new(8)];
    assert_eq!(
        PointUiKey::for_index(&points, 0),
        Some(PointUiKey::new(8, 0))
    );
    assert_eq!(
        PointUiKey::for_index(&points, 2),
        Some(PointUiKey::new(8, 1))
    );
    assert_eq!(PointUiKey::for_index(&points, 3), None);
    let mut state = ManagerUiState::default();
    let a = operation("重复");
    let b = operation("不同");
    let mut rows = state
        .reconcile_point(
            "routes/a.yaml",
            PointUiKey::new(8, 0),
            &[a.clone(), b.clone(), a.clone()],
        )
        .unwrap();
    assert!(rows.iter().all(|row| row.expanded));
    rows[0].expanded = false;
    let identities: Vec<_> = rows.iter().map(|row| row.id).collect();
    assert_ne!(identities[0], identities[2]);
    state
        .put_point("routes/a.yaml", PointUiKey::new(8, 0), rows)
        .unwrap();
    let restored = state
        .reconcile_point(
            "ROUTES\\A.YAML",
            PointUiKey::new(8, 0),
            &[a.clone(), a.clone(), b],
        )
        .unwrap();
    assert_eq!(
        restored.iter().map(|row| row.id).collect::<Vec<_>>(),
        [identities[0], identities[2], identities[1]]
    );
    assert!(!restored[0].expanded);
    assert!(restored[1].expanded);
    let other = state
        .reconcile_point("routes/a.yaml", PointUiKey::new(8, 1), &[a])
        .unwrap();
    assert!(other[0].expanded);
    assert!(!identities.contains(&other[0].id));
    state
        .put_point("routes/a.yaml", PointUiKey::new(8, 1), other)
        .unwrap();
    assert_eq!(state.routes["routes/a.yaml"].len(), 2);
}

#[test]
fn reorder_insert_delete_and_descriptor_edits_preserve_identity() {
    let mut state = populated();
    let key = PointUiKey::new(42, 0);
    let mut rows = state
        .point("routes/测试.yaml", key)
        .unwrap()
        .operations
        .clone();
    let original_id = rows[0].id;
    rows[0].expanded = false;
    rows[0].descriptor.name = "修改后的名称".into();
    rows[0].descriptor.params.insert("ms".into(), json!(555));
    let new = state.fresh_operation(&operation("插入")).unwrap();
    let new_id = new.id;
    rows.insert(0, new);
    state
        .put_point("routes/测试.yaml", key, rows.clone())
        .unwrap();
    let descriptors: Vec<_> = rows.iter().map(|row| row.descriptor.clone()).collect();
    let restored = state
        .reconcile_point("routes/测试.yaml", key, &descriptors)
        .unwrap();
    assert_eq!(restored[1].id, original_id);
    assert!(!restored[1].expanded);
    rows.remove(0);
    state.put_point("routes/测试.yaml", key, rows).unwrap();
    let replacement = state
        .reconcile_point("routes/测试.yaml", key, &[operation("插入")])
        .unwrap();
    assert_ne!(replacement[0].id, new_id);
    assert!(replacement[0].expanded);
}

#[test]
fn omitted_optional_defaults_and_zero_jitter_match_materialized_descriptors() {
    let mut state = ManagerUiState::default();
    let key = PointUiKey::new(1, 0);
    let mut minimal = Operation::new(OpKind::DragNative, [0.25, 0.75]);
    for key in [
        "speed",
        "pre_drag_distance",
        "pre_drag_speed",
        "pre_drag_delay",
    ] {
        minimal.params.remove(key);
    }
    minimal.params.insert("jitter".into(), json!(0.0));
    let mut rows = state
        .reconcile_point("routes/a.yaml", key, &[minimal])
        .unwrap();
    rows[0].expanded = false;
    let identity = rows[0].id;
    state.put_point("routes/a.yaml", key, rows).unwrap();
    let restored = state
        .reconcile_point(
            "routes/a.yaml",
            key,
            &[Operation::new(OpKind::DragNative, [0.25, 0.75])],
        )
        .unwrap();
    assert_eq!(restored[0].id, identity);
    assert!(!restored[0].expanded);
    assert!(!restored[0].descriptor.params.contains_key("jitter"));
}

#[test]
fn copied_actions_have_independent_identities_and_inherit_expansion() {
    let mut state = populated();
    let key = PointUiKey::new(42, 0);
    let mut source = state
        .point("routes/测试.yaml", key)
        .unwrap()
        .operations
        .clone();
    source[0].expanded = false;
    state
        .put_point("routes/测试.yaml", key, source.clone())
        .unwrap();
    let mut copied = state.copy_operations(&source).unwrap();
    assert_ne!(copied[0].id, source[0].id);
    assert!(!copied[0].expanded);
    copied[0].expanded = true;
    copied[0].descriptor.name = "副本".into();
    state
        .put_point("routes/测试.yaml", PointUiKey::new(43, 0), copied)
        .unwrap();
    assert_eq!(
        state.point("routes/测试.yaml", key).unwrap().operations,
        source
    );
    assert!(state.validate().is_ok());
}

#[test]
fn roundtrip_states_are_independent_of_route_config_and_execution_cache() {
    let (_dir, paths) = fixture();
    fs::create_dir_all(paths.cache()).unwrap();
    fs::write(paths.routes().join("test.yaml"), "route bytes").unwrap();
    fs::write(paths.cache().join("test.json"), "cache bytes").unwrap();
    let mut file = ManagerUiFile::open(&paths).unwrap();
    file.state = populated();
    file.state.routes.get_mut("routes/测试.yaml").unwrap()[0].operations[0].expanded = false;
    file.save(&paths).unwrap();
    let restored = ManagerUiFile::open(&paths).unwrap();
    assert_eq!(restored.state, file.state);
    assert!(
        !restored
            .state
            .point("routes/测试.yaml", PointUiKey::new(42, 0))
            .unwrap()
            .operations[0]
            .expanded
    );
    assert_eq!(
        fs::read_to_string(paths.routes().join("test.yaml")).unwrap(),
        "route bytes"
    );
    assert_eq!(
        fs::read_to_string(paths.cache().join("test.json")).unwrap(),
        "cache bytes"
    );
    assert_eq!(fs::read_to_string(paths.config()).unwrap(), "{}");
    assert!(!paths.root.join("state/.backups").exists());
}

#[test]
fn normalization_and_semantic_noop_preserve_original_bytes_bom_and_mtime() {
    let (_dir, paths) = fixture();
    let mut value = serde_json::to_value(populated()).unwrap();
    let points = value["routes"]
        .as_object_mut()
        .unwrap()
        .remove("routes/测试.yaml")
        .unwrap();
    value["routes"]["ROUTES\\.\\测试.YAML"] = points;
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend_from_slice(&serde_json::to_vec(&value).unwrap());
    write_state(&paths, &bytes);
    let mut file = ManagerUiFile::open(&paths).unwrap();
    assert!(file.state.routes.contains_key("routes/测试.yaml"));
    let modified = fs::metadata(paths.manager_ui_state())
        .unwrap()
        .modified()
        .unwrap();
    for _ in 0..3 {
        file.save(&paths).unwrap();
        assert_eq!(fs::read(paths.manager_ui_state()).unwrap(), bytes);
        assert_eq!(
            fs::metadata(paths.manager_ui_state())
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
    }
    assert_eq!(fs::read_dir(paths.root.join("state")).unwrap().count(), 1);
}

#[test]
fn bad_version_corrupt_json_and_invalid_identities_are_preserved() {
    let (_dir, paths) = fixture();
    let base = serde_json::to_value(populated()).unwrap();
    let mut invalid = Vec::new();
    let mut version = base.clone();
    version["format"] = json!("manager-ui/v2");
    invalid.push(serde_json::to_vec(&version).unwrap());
    invalid.push(b"{bad json".to_vec());
    let mut zero = base.clone();
    zero["routes"]["routes/测试.yaml"][0]["operations"][0]["id"] = json!(0);
    invalid.push(serde_json::to_vec(&zero).unwrap());
    let mut duplicate = base.clone();
    let point = duplicate["routes"]["routes/测试.yaml"][0].clone();
    duplicate["routes"]["routes/测试.yaml"]
        .as_array_mut()
        .unwrap()
        .push(point);
    invalid.push(serde_json::to_vec(&duplicate).unwrap());
    let mut duplicate_route = base.clone();
    duplicate_route["routes"]["ROUTES\\测试.YAML"] =
        duplicate_route["routes"]["routes/测试.yaml"].clone();
    invalid.push(serde_json::to_vec(&duplicate_route).unwrap());
    let mut traversal = base;
    let point = traversal["routes"]
        .as_object_mut()
        .unwrap()
        .remove("routes/测试.yaml")
        .unwrap();
    traversal["routes"]["../outside.yaml"] = point;
    invalid.push(serde_json::to_vec(&traversal).unwrap());
    for bytes in invalid {
        write_state(&paths, &bytes);
        assert!(ManagerUiFile::open(&paths).is_err());
        let mut forged = ManagerUiFile {
            state: ManagerUiState::default(),
            snapshot: DiskSnapshot(Some(bytes.clone())),
        };
        assert!(forged.save(&paths).is_err());
        assert_eq!(fs::read(paths.manager_ui_state()).unwrap(), bytes);
    }
}

#[test]
fn concurrent_writers_and_noop_saves_detect_conflicts() {
    let (_dir, paths) = fixture();
    let mut first = ManagerUiFile::open(&paths).unwrap();
    let mut second = ManagerUiFile::open(&paths).unwrap();
    first.state = populated();
    first.save(&paths).unwrap();
    let bytes = fs::read(paths.manager_ui_state()).unwrap();
    assert!(second.save(&paths).is_err());
    second.state = populated();
    assert!(second.save(&paths).is_err());
    assert_eq!(fs::read(paths.manager_ui_state()).unwrap(), bytes);
}

#[test]
fn remove_and_allocation_overflow_do_not_corrupt_other_point_states() {
    let mut state = populated();
    let before = state.clone();
    let duplicate = before
        .point("routes/测试.yaml", PointUiKey::new(42, 0))
        .unwrap()
        .operations
        .clone();
    assert!(
        state
            .put_point("routes/测试.yaml", PointUiKey::new(43, 0), duplicate)
            .is_err()
    );
    assert_eq!(state, before);
    state.next_id = u64::MAX;
    let exhausted = state.clone();
    assert!(state.fresh_operation(&operation("new")).is_err());
    assert!(
        state
            .reconcile_point("routes/a.yaml", PointUiKey::new(1, 0), &[operation("new")])
            .is_err()
    );
    assert_eq!(state, exhausted);
    state.remove_point("ROUTES\\测试.YAML", &PointUiKey::new(42, 0));
    assert!(
        state
            .point("routes/测试.yaml", PointUiKey::new(42, 0))
            .is_none()
    );
    state.remove_route("ROUTES\\测试.YAML");
    assert!(state.routes.is_empty());
}
