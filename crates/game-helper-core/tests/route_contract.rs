use game_helper_core::route::{OpKind, Operation, Point, Route};
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
    assert_eq!(route.points[0].action_position(), [0.9, 0.9]);
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
    rejected(
        |v| v["points"][0]["operations"][0]["pos"] = json!([1.5, 0.5]),
        "points[0]",
    );
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
    rejected(
        |v| v["points"][1]["operations"] = json!([{"type":"click", "pos":[-0.1,0.9]}]),
        "points[1]",
    );
}
#[test]
fn structured_edit_reports_per_point_location() {
    let mut route = Route::parse(VALID).unwrap();
    route.points[1]
        .operations
        .push(Operation::new(OpKind::Click, [1.5, 0.9]));
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
    raw["points"][0]["operations"][0]["pos"] = json!([0.0, 1.0]);
    assert_eq!(
        Route::from_value(&raw).unwrap().points[0].action_position(),
        [0.0, 1.0]
    );
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
    rejected(
        |v| v["points"][0]["operations"][0]["pos"] = json!([true, 0.5]),
        "pos",
    );
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
fn spatial_actions_and_chinese_cache_survive_without_point_coordinates() {
    let mut route = Route::parse(VALID).unwrap();
    route.points[0].operations[0]
        .params
        .insert("pos".into(), json!([0.123456789, 0.876543219]));
    let json = route.cache_json().unwrap();
    assert!(json.contains("测试路线") && !json.contains("\\u6d4b"));
    let value: Value = serde_json::from_str(&json).unwrap();
    assert!(
        value["points"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("pos").is_none())
    );
    assert_eq!(
        value["points"][0]["operations"][0]["pos"],
        json!([0.123456789, 0.876543219])
    );
    assert_eq!(
        value["points"][0]["operations"][4]["from"],
        json!([0.1, 0.1])
    );
    assert_eq!(value["points"][0]["operations"][4]["to"], json!([0.2, 0.2]));
}
#[test]
fn legacy_point_coordinates_are_optional_ignored_and_not_generated() {
    let legacy = Route::parse(VALID).unwrap();
    let mut without = raw();
    for point in without["points"].as_array_mut().unwrap() {
        point.as_object_mut().unwrap().remove("pos");
    }
    assert_eq!(Route::from_value(&without).unwrap(), legacy);
    // Obsolete point metadata must not affect action validation or execution.
    without["points"][0]["pos"] = json!("obsolete");
    assert_eq!(Route::from_value(&without).unwrap(), legacy);
    let yaml: Value = yaml_serde::from_str(&legacy.yaml().unwrap()).unwrap();
    assert!(
        yaml["points"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("pos").is_none())
    );
    assert_eq!(Route::from_value(&yaml).unwrap(), legacy);
    let decoded: Route = serde_json::from_str(&legacy.cache_json().unwrap()).unwrap();
    assert_eq!(decoded, legacy);
}
#[test]
fn new_actions_use_existing_spatial_coordinates_or_window_center() {
    let mut point = Point::new(1);
    assert_eq!(point.action_position(), [0.5, 0.5]);
    point
        .operations
        .push(Operation::new(OpKind::Wait, [0.0, 0.0]));
    point
        .operations
        .push(Operation::new(OpKind::Drag, [0.2, 0.3]));
    point
        .operations
        .push(Operation::new(OpKind::Click, [0.8, 0.9]));
    assert_eq!(point.action_position(), [0.2, 0.3]);
    point.operations.remove(1);
    assert_eq!(point.action_position(), [0.8, 0.9]);
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

#[test]
fn operation_names_default_empty_for_new_and_legacy_actions() {
    for kind in OpKind::ALL {
        let operation = Operation::new(kind, [0.25, 0.75]);
        assert!(operation.name.is_empty());
        let value = serde_json::to_value(&operation).unwrap();
        assert!(value.get("name").is_none());
        let decoded: Operation = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, operation);
    }

    let legacy = Route::parse(VALID).unwrap();
    let mut explicit_empty = raw();
    for operation in explicit_empty["points"][0]["operations"]
        .as_array_mut()
        .unwrap()
    {
        operation["name"] = json!("");
    }
    assert_eq!(Route::from_value(&explicit_empty).unwrap(), legacy);
    for source in [legacy.yaml().unwrap(), legacy.cache_json().unwrap()] {
        let value: Value = yaml_serde::from_str(&source).unwrap();
        for operation in value["points"][0]["operations"].as_array().unwrap() {
            assert!(operation.get("name").is_none());
        }
    }
    assert!(
        legacy.points[0]
            .operations
            .iter()
            .all(|operation| operation.name.is_empty())
    );
}

#[test]
fn names_and_parameters_round_trip_for_all_six_action_kinds() {
    let names = [
        "中文动作 🦀",
        "  引号 '和\"  ",
        "第一行\n第二行",
        "",
        "重复名称",
        "重复名称",
    ];
    let baseline = Route::parse(VALID).unwrap();
    let mut value = raw();
    for (operation, name) in value["points"][0]["operations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(names)
    {
        operation["name"] = json!(name);
    }
    let route = Route::from_value(&value).unwrap();
    route.validate().unwrap();
    assert_eq!(route.format, "route/v1");
    for (index, operation) in route.points[0].operations.iter().enumerate() {
        assert_eq!(operation.kind, OpKind::ALL[index]);
        assert_eq!(operation.name, names[index]);
        assert_eq!(
            operation.params,
            baseline.points[0].operations[index].params
        );
        assert!(!operation.params.contains_key("name"));
    }

    let yaml = route.yaml().unwrap();
    assert_eq!(Route::parse(&yaml).unwrap(), route);
    let cache = route.cache_json().unwrap();
    let cache_value: Value = serde_json::from_str(&cache).unwrap();
    assert_eq!(cache_value["format"], "route/v1");
    for (index, name) in names.into_iter().enumerate() {
        let operation = &cache_value["points"][0]["operations"][index];
        if name.is_empty() {
            assert!(operation.get("name").is_none());
        } else {
            assert_eq!(operation["name"], name);
        }
    }
    assert_eq!(serde_json::from_str::<Route>(&cache).unwrap(), route);
    assert_eq!(Route::from_value(&cache_value).unwrap(), route);
}

#[test]
fn supplied_operation_names_require_strings_with_action_location() {
    for index in 0..OpKind::ALL.len() {
        for name in [
            Value::Null,
            json!(0),
            json!(1.5),
            json!(true),
            json!(false),
            json!([]),
            json!({}),
        ] {
            let mut value = raw();
            value["points"][0]["operations"][index]["name"] = name;
            let location = format!("operations[{index}].name");
            let error = Route::from_value(&value).unwrap_err().to_string();
            assert!(error.contains(&location), "{error}");
            assert!(error.contains("必须是字符串"), "{error}");
            let yaml = yaml_serde::to_string(&value).unwrap();
            let error = Route::parse(&yaml).unwrap_err().to_string();
            assert!(error.contains(&location), "{error}");
        }
    }
}

#[test]
fn action_names_survive_normalization_while_unknown_fields_are_filtered() {
    let baseline = Route::parse(VALID).unwrap();
    let mut value = raw();
    for operation in value["points"][0]["operations"].as_array_mut().unwrap() {
        operation["name"] = json!("已命名的动作");
        operation["unknown"] = json!({"nested": [1, true, null]});
    }
    let mut route = Route::from_value(&value).unwrap();
    for (index, operation) in route.points[0].operations.iter_mut().enumerate() {
        assert_eq!(operation.name, "已命名的动作");
        assert_eq!(
            operation.params,
            baseline.points[0].operations[index].params
        );
        // Exercise cache_json's second validation pass on in-memory parameters.
        operation.params.insert("unknown".into(), json!("discard"));
    }
    route.validate().unwrap();
    let cache: Value = serde_json::from_str(&route.cache_json().unwrap()).unwrap();
    for operation in cache["points"][0]["operations"].as_array().unwrap() {
        assert_eq!(operation["name"], "已命名的动作");
        assert!(operation.get("unknown").is_none());
    }
    let normalized = Route::from_value(&cache).unwrap();
    for (index, operation) in normalized.points[0].operations.iter().enumerate() {
        assert_eq!(operation.name, "已命名的动作");
        assert_eq!(
            operation.params,
            baseline.points[0].operations[index].params
        );
    }
}
