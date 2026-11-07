use game_helper_core::route::{OpKind, Route};
use serde_json::{Value, json};

const VALID: &str = r#"
format: route/v1
name: 测试路线
target: {process: notepad.exe, windowClass: "", title: ""}
points:
  - id: 1
    name: 第一个点
    pos: [0.5, 0.5]
    operations:
      - {type: click, pos: [0.9, 0.9], button: left, clicks: 2, jitter: 3}
      - {type: key, key: m}
      - {type: press, key: w, ms: 800}
      - {type: wheel, delta: -5}
      - {type: drag, from: [0.1, 0.1], to: [0.2, 0.2], duration: 150}
      - {type: wait, ms: 100}
  - id: 2
    name: 第二个点
    pos: [0.1, 0.9]
"#;
fn raw() -> Value {
    yaml_serde::from_str(VALID).unwrap()
}
fn rejected(change: impl FnOnce(&mut Value), location: &str) {
    let mut raw = raw();
    change(&mut raw);
    let error = Route::from_value(&raw).unwrap_err().to_string();
    assert!(
        error.contains(location),
        "{error:?} should contain {location}"
    );
}
#[test]
fn load_valid() {
    let route = Route::parse(VALID).unwrap();
    assert_eq!(route.name, "测试路线");
    assert_eq!(route.points.len(), 2);
    assert_eq!(route.points[0].pos, [0.5, 0.5]);
    assert_eq!(route.points[0].operations.len(), 6);
    assert_eq!(route.points[0].operations[0].kind, OpKind::Click);
}
#[test]
fn json_defaults_and_jitter() {
    let cache: Value =
        serde_json::from_str(&Route::parse(VALID).unwrap().cache_json().unwrap()).unwrap();
    assert_eq!(cache["format"], "route/v1");
    assert_eq!(cache["points"][0]["operations"][0]["jitter"], 3);
    assert_eq!(cache["points"][0]["operations"][1]["up"], false);
}
#[test]
fn yaml_save_reload() {
    let route = Route::parse(VALID).unwrap();
    assert_eq!(Route::parse(&route.yaml().unwrap()).unwrap(), route);
}
#[test]
fn bad_format() {
    rejected(|v| v["format"] = json!("route/v999"), "format");
}
#[test]
fn empty_target() {
    rejected(|v| v["target"] = json!({}), "target");
}
#[test]
fn position_outside() {
    rejected(|v| v["points"][0]["pos"] = json!([1.5, 0.5]), "points[0]");
}
#[test]
fn unknown_operation() {
    rejected(
        |v| v["points"][0]["operations"][0]["type"] = json!("explode"),
        "operations[0]",
    );
}
#[test]
fn missing_press_ms() {
    rejected(
        |v| {
            v["points"][0]["operations"][2]
                .as_object_mut()
                .unwrap()
                .remove("ms");
        },
        "ms",
    );
}
#[test]
fn second_point_location() {
    rejected(|v| v["points"][1]["pos"] = json!([-0.1, 0.9]), "points[1]");
}
#[test]
fn structured_edit_reports_per_point_location() {
    let mut route = Route::parse(VALID).unwrap();
    route.points[1].pos = [1.5, 0.9];
    let error = route.validate().unwrap_err().to_string();
    assert!(
        error.contains("points[1]") && error.contains("#2"),
        "{error}"
    );
    assert!(!error.contains("points[0]"), "不应误报第一个点: {error}");
}
#[test]
fn yaml_syntax_has_location() {
    let error = Route::parse("format: route/v1\nname: [unclosed\npoints: []")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("YAML 语法错误") && error.contains("行"),
        "{error}"
    );
}
#[test]
fn bad_id() {
    rejected(|v| v["points"][1]["id"] = json!("abc"), "id");
}
#[test]
fn empty_points_allowed() {
    let mut raw = raw();
    raw["points"] = json!([]);
    assert!(Route::from_value(&raw).unwrap().points.is_empty());
}
#[test]
fn boundary_coordinates() {
    let mut raw = raw();
    raw["points"][1]["pos"] = json!([0.0, 1.0]);
    assert_eq!(Route::from_value(&raw).unwrap().points[1].pos, [0.0, 1.0]);
}
#[test]
fn wait_zero_rejected() {
    rejected(|v| v["points"][0]["operations"][5]["ms"] = json!(0), "ms");
}
#[test]
fn wheel_zero_rejected() {
    rejected(
        |v| v["points"][0]["operations"][3]["delta"] = json!(0),
        "delta",
    );
}
#[test]
fn booleans_cannot_be_numbers() {
    rejected(|v| v["points"][0]["id"] = json!(true), "id");
    rejected(|v| v["points"][0]["pos"] = json!([true, 0.5]), "pos");
    rejected(
        |v| v["points"][0]["operations"][0]["jitter"] = json!(true),
        "jitter",
    );
    rejected(
        |v| v["points"][0]["operations"][2]["ms"] = json!(true),
        "ms",
    );
}
#[test]
fn integral_float_durations_work_but_ids_and_clicks_require_integer_representation() {
    let mut value = raw();
    value["points"][0]["operations"][2]["ms"] = json!(800.0);
    assert!(Route::from_value(&value).is_ok());
    rejected(
        |v| v["points"][0]["operations"][2]["ms"] = json!(800.5),
        "ms",
    );
    rejected(|v| v["points"][0]["id"] = json!(1.0), "id");
    rejected(
        |v| v["points"][0]["operations"][0]["clicks"] = json!(2.0),
        "clicks",
    );
}
#[test]
fn invalid_operation_parameters() {
    rejected(
        |v| v["points"][0]["operations"][0]["button"] = json!("center"),
        "button",
    );
    rejected(
        |v| v["points"][0]["operations"][0]["clicks"] = json!(0),
        "clicks",
    );
    rejected(
        |v| v["points"][0]["operations"][0]["jitter"] = json!(-1),
        "jitter",
    );
    rejected(|v| v["points"][0]["operations"][1]["up"] = json!(1), "up");
}
#[test]
fn omitted_click_and_drag_defaults_and_zero_jitter() {
    let mut value = raw();
    value["points"][0]["operations"][0] = json!({"type":"click", "pos":[0.5,0.5], "jitter":0});
    value["points"][0]["operations"][4]
        .as_object_mut()
        .unwrap()
        .remove("duration");
    let cache: Value =
        serde_json::from_str(&Route::from_value(&value).unwrap().cache_json().unwrap()).unwrap();
    assert_eq!(cache["points"][0]["operations"][0]["button"], "left");
    assert_eq!(cache["points"][0]["operations"][0]["clicks"], 1);
    assert!(cache["points"][0]["operations"][0].get("jitter").is_none());
    assert_eq!(cache["points"][0]["operations"][4]["duration"], 200);
}
#[test]
fn anchor_six_decimal_and_chinese_cache() {
    let mut route = Route::parse(VALID).unwrap();
    route.points[0].pos = [0.123456789, 0.876543219];
    let json = route.cache_json().unwrap();
    assert!(json.contains("测试路线") && !json.contains("\\u6d4b"));
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["points"][0]["pos"], json!([0.123457, 0.876543]));
}
#[test]
fn malformed_structures_rejected_and_list_order_retained() {
    rejected(|v| v["points"] = json!({}), "points");
    rejected(
        |v| v["points"][0]["operations"] = json!("wrong"),
        "operations",
    );
    let mut value = raw();
    value["points"][0]["id"] = json!(9);
    value["points"][1]["id"] = json!(2);
    let route = Route::from_value(&value).unwrap();
    assert_eq!(
        route.points.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![9, 2]
    );
}
