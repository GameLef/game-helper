use game_helper_core::{
    config::{Config, hotkey_canonical, hotkey_display, hotkey_to_ahk},
    storage::ConfigFile,
};
use serde_json::{Value, json};
use std::fs;

fn snapshot(config: &Config) -> Value {
    serde_json::to_value(config).unwrap()
}

#[test]
fn legacy_enabled_defaults_and_new_binding_starts_off() {
    let mut config =
        Config::parse(br#"{"hotkeys":{"bindings":[{"route":"old","hotkey":"F8"}]}}"#).unwrap();
    assert!(config.binding_for("old").unwrap().enabled);
    config.set_binding("new", "F9", 2).unwrap();
    assert!(!config.binding_for("new").unwrap().enabled);
    config.set_binding("old", "F10", 3).unwrap();
    assert!(config.binding_for("old").unwrap().enabled);
    assert_eq!(config.binding_for("old").unwrap().start_at, 3);
    assert!(
        snapshot(&config)["hotkeys"]["bindings"][0]["enabled"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn enabling_last_route_wins_and_disabling_does_not_restore_displaced_route() {
    let mut config = Config::default();
    config.set_binding("first", "Ctrl+Alt+F8", 1).unwrap();
    config.set_binding("second", "!^f8", 2).unwrap();
    config.set_binding_enabled("first", true).unwrap();
    assert!(config.binding_for("first").unwrap().enabled);
    assert!(!config.binding_for("second").unwrap().enabled);
    config.set_binding_enabled("second", true).unwrap();
    assert!(!config.binding_for("first").unwrap().enabled);
    assert!(config.binding_for("second").unwrap().enabled);
    config.set_binding_enabled("second", false).unwrap();
    assert!(!config.binding_for("first").unwrap().enabled);
    assert!(!config.binding_for("second").unwrap().enabled);
    config.set_binding_enabled("first", true).unwrap();
    assert!(config.binding_for("first").unwrap().enabled);
}

#[test]
fn editing_active_binding_displaces_conflict_but_editing_disabled_binding_does_not() {
    let mut config = Config::default();
    config.set_binding("first", "F7", 1).unwrap();
    config.set_binding("second", "F8", 1).unwrap();
    config.set_binding_enabled("first", true).unwrap();
    config.set_binding_enabled("second", true).unwrap();
    config.set_binding("first", "f8", 2).unwrap();
    assert!(config.binding_for("first").unwrap().enabled);
    assert!(!config.binding_for("second").unwrap().enabled);
    config.set_binding("second", "F8", 3).unwrap();
    assert!(config.binding_for("first").unwrap().enabled);
    assert!(!config.binding_for("second").unwrap().enabled);
    config.set_binding("first", "", 1).unwrap();
    assert!(config.binding_for("first").is_none());
    assert!(!config.binding_for("second").unwrap().enabled);
}

#[test]
fn parsing_legacy_duplicates_keeps_last_enabled_item_and_unknown_fields() {
    let config = Config::parse(
        &serde_json::to_vec(&json!({
            "custom": {"preserved": true},
            "hotkeys": {"custom": 42, "bindings": [
                {"route":"first", "hotkey":"^!F8", "custom":9},
                {"route":"second", "hotkey":"Alt+Ctrl+f8"},
                {"route":"disabled", "hotkey":"!^F8", "enabled":false},
                {"route":"other", "hotkey":"F9"},
                {"route":"future", "hotkey":"F9", "action":"future_action"}
            ]}
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!config.binding_for("first").unwrap().enabled);
    assert!(config.binding_for("second").unwrap().enabled);
    assert!(!config.binding_for("disabled").unwrap().enabled);
    assert!(config.binding_for("other").unwrap().enabled);
    let serialized = snapshot(&config);
    assert_eq!(serialized["custom"]["preserved"], true);
    assert_eq!(serialized["hotkeys"]["custom"], 42);
    assert_eq!(serialized["hotkeys"]["bindings"][0]["custom"], 9);
    let reparsed = Config::parse(&serde_json::to_vec(&serialized).unwrap()).unwrap();
    assert_eq!(snapshot(&reparsed), serialized);
}

#[test]
fn canonical_keys_normalize_modifiers_aliases_case_and_preserve_side_semantics() {
    for (input, expected) in [
        ("Alt+Ctrl+Shift+Win+f8", "^!+#f8"),
        ("#+!^F8", "^!+#f8"),
        ("~$!^F8", "^!f8"),
        ("RCtrl+LAlt+f8", ">^<!f8"),
        ("<!>^F8", ">^<!f8"),
        ("LCtrl+RAlt+F8", "<^>!f8"),
        ("Ctrl+Esc", "^escape"),
        ("ctrl-return", "^enter"),
        ("^DEL", "^delete"),
        ("^INS", "^insert"),
        ("^BS", "^backspace"),
        ("^F8 Up", "^f8 up"),
        ("'", "'"),
        ("", ""),
    ] {
        assert_eq!(hotkey_canonical(input).unwrap(), expected, "{input}");
    }
    for (a, b) in [
        ("^F8", "<^F8"),
        ("<^F8", ">^F8"),
        ("*^F8", "^F8"),
        ("F8 up", "F8"),
    ] {
        assert_ne!(hotkey_canonical(a).unwrap(), hotkey_canonical(b).unwrap());
    }
    assert_eq!(hotkey_to_ahk("LCtrl+RAlt+F8").unwrap(), "<^>!F8");
    assert!(hotkey_canonical("Ctrl+Alt").is_err());
    assert!(hotkey_canonical("<F8").is_err());
    assert!(hotkey_canonical("^!").is_err());
}

#[test]
fn display_is_human_readable_and_keeps_invalid_input_visible() {
    for (input, expected) in [
        ("!^f8", "Ctrl+Alt+F8"),
        ("<!>^F8", "RCtrl+LAlt+F8"),
        ("Ctrl+Esc", "Ctrl+Esc"),
        ("^f8 up", "Ctrl+F8 Up"),
        ("^RButton Up", "Ctrl+RButton Up"),
        ("'", "'"),
        ("", ""),
        ("Ctrl+", "Ctrl+"),
    ] {
        assert_eq!(hotkey_display(input), expected);
    }
}

#[test]
fn side_specific_routes_can_remain_active_together() {
    let mut config = Config::default();
    config.set_binding("left", "LCtrl+F8", 1).unwrap();
    config.set_binding("right", "RCtrl+F8", 1).unwrap();
    config.set_binding_enabled("left", true).unwrap();
    config.set_binding_enabled("right", true).unwrap();
    assert!(config.binding_for("left").unwrap().enabled);
    assert!(config.binding_for("right").unwrap().enabled);
}

#[test]
fn stop_key_conflicts_are_rejected_without_mutation_and_legacy_conflicts_are_disabled() {
    let mut config = Config::default();
    config.hotkeys.stop_all = "Ctrl+Alt+Esc".into();
    let before = snapshot(&config);
    for key in ["!^Escape", "~$!^ESC", "*^!Esc"] {
        assert!(config.set_binding("bad", key, 1).is_err(), "{key}");
        assert_eq!(snapshot(&config), before);
    }
    config.set_binding("route", "F8", 1).unwrap();
    config.hotkeys.stop_all = "F8".into();
    let before = snapshot(&config);
    assert!(config.set_binding_enabled("route", true).is_err());
    assert_eq!(snapshot(&config), before);
    config.set_binding_enabled("route", false).unwrap();
    let normalized = Config::parse(
        br#"{"hotkeys":{"stopAll":"f12","bindings":[{"route":"legacy","hotkey":"~*F12"}]}}"#,
    )
    .unwrap();
    assert!(!normalized.binding_for("legacy").unwrap().enabled);
}

#[test]
fn stop_protection_covers_wildcards_release_and_sided_modifier_overlap() {
    for (stop, route) in [
        ("F12", "F12 up"),
        ("Ctrl+F12", "*F12"),
        ("*F12", "Ctrl+F12"),
        ("Ctrl+F12", "LCtrl+F12"),
        ("LCtrl+F12", "Ctrl+F12 Up"),
    ] {
        let mut config = Config::default();
        config.hotkeys.stop_all = stop.into();
        assert!(
            config.set_binding("route", route, 1).is_err(),
            "{stop} / {route}"
        );
    }
    let mut config = Config::default();
    config.hotkeys.stop_all = "Ctrl+F12".into();
    // With no wildcard these require different modifier combinations.
    config.set_binding("route", "Alt+F12", 1).unwrap();
    config.set_binding_enabled("route", true).unwrap();
}

#[test]
fn invalid_enable_and_binding_edits_do_not_partially_disable_routes() {
    let mut config = Config::default();
    config.set_binding("good", "F8", 1).unwrap();
    config.set_binding_enabled("good", true).unwrap();
    let before = snapshot(&config);
    assert!(config.set_binding_enabled("missing", true).is_err());
    assert!(config.set_binding("bad", "F9", 0).is_err());
    assert!(config.set_binding("good", "Ctrl+", 1).is_err());
    assert_eq!(snapshot(&config), before);
    // A malformed external binding must still be possible to disable for repair.
    config.hotkeys.bindings[0].hotkey = "Ctrl+".into();
    config.set_binding_enabled("good", false).unwrap();
    assert!(!config.binding_for("good").unwrap().enabled);
}

#[test]
fn normalized_legacy_no_op_preserves_original_bytes_and_enabled_edits_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let original = "{\n\t\"custom\": 9, \"hotkeys\": {\"bindings\": [\n {\"route\":\"a\",\"hotkey\":\"F8\",\"unknown\":true},\n {\"route\":\"b\",\"hotkey\":\"f8\"}]}}\n";
    fs::write(&path, original).unwrap();
    let mut file = ConfigFile::open(path.clone()).unwrap();
    assert!(!file.value.binding_for("a").unwrap().enabled);
    assert!(file.value.binding_for("b").unwrap().enabled);
    file.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(!dir.path().join(".backups").exists());
    file.value.set_binding_enabled("a", true).unwrap();
    file.save().unwrap();
    let reloaded = ConfigFile::open(path.clone()).unwrap();
    assert!(reloaded.value.binding_for("a").unwrap().enabled);
    assert!(!reloaded.value.binding_for("b").unwrap().enabled);
    assert_eq!(
        reloaded.value.binding_for("a").unwrap().extra["unknown"],
        true
    );
    assert_eq!(reloaded.value.extra["custom"], 9);
    assert_eq!(
        fs::read_dir(dir.path().join(".backups")).unwrap().count(),
        1
    );
    file.save().unwrap();
    assert_eq!(
        fs::read_dir(dir.path().join(".backups")).unwrap().count(),
        1
    );
    fs::write(&path, "{\"external\":true}").unwrap();
    file.value.set_binding_enabled("a", false).unwrap();
    assert!(file.save().is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "{\"external\":true}");
}
