use super::*;
use anyhow::{bail, ensure};

pub(super) fn route(raw: &Value) -> Result<Route> {
    ensure!(raw.is_object(), "路线文件必须是 YAML 对象");
    ensure!(raw["format"] == FORMAT, "format 必须是 {FORMAT}");
    let name = required_string(&raw["name"], "路线.name")?;
    let target = &raw["target"];
    ensure!(target.is_object(), "target 必须是对象");
    let target = Target {
        process: text(&target["process"]),
        window_class: text(&target["windowClass"]),
        title: text(&target["title"]),
    };
    ensure!(
        !target.process.is_empty() || !target.window_class.is_empty() || !target.title.is_empty(),
        "target 中 process/windowClass/title 至少填一个"
    );
    let points = raw["points"].as_array().context("points 必须是列表")?;
    let points = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let location = format!(
                "points[{index}] #{}({})",
                text(&point["id"]),
                text(&point["name"])
            );
            ensure!(point.is_object(), "{location}: 点位必须是对象");
            let id = point["id"]
                .as_u64()
                .filter(|n| *n > 0)
                .with_context(|| format!("{location}: id 必须是正整数"))?;
            let pos = pair(&point["pos"], &format!("{location}.pos"))?;
            let operations = match point.get("operations") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => value
                    .as_array()
                    .with_context(|| format!("{location}.operations 必须是列表"))?
                    .iter()
                    .enumerate()
                    .map(|(i, op)| operation(op, &format!("{location}.operations[{i}]")))
                    .collect::<Result<_>>()?,
            };
            Ok(Point {
                id,
                pos,
                name: text(&point["name"]),
                note: text(&point["note"]),
                operations,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Route {
        format: FORMAT.into(),
        name,
        note: text(&raw["note"]),
        target,
        points,
    })
}

fn text(value: &Value) -> String {
    match value {
        Value::Null | Value::Bool(false) => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".into(),
        Value::Number(n) if n.as_f64() == Some(0.0) => String::new(),
        value => value.to_string(),
    }
}
fn required_string(value: &Value, location: &str) -> Result<String> {
    let value = value
        .as_str()
        .with_context(|| format!("{location}: 必须是字符串"))?;
    ensure!(!value.trim().is_empty(), "{location}: 不能为空");
    Ok(value.into())
}
fn pair(value: &Value, location: &str) -> Result<[f64; 2]> {
    let pair = value
        .as_array()
        .filter(|v| v.len() == 2)
        .with_context(|| format!("{location}: 坐标必须是 [x, y] 两个数字"))?;
    let mut result = [0.0; 2];
    for (i, value) in pair.iter().enumerate() {
        let n = value
            .as_f64()
            .with_context(|| format!("{location}: 坐标必须是数字"))?;
        ensure!(
            n.is_finite() && (0.0..=1.0).contains(&n),
            "{location}: 坐标超出 0~1 范围"
        );
        result[i] = n;
    }
    Ok(result)
}
fn number(value: &Value, location: &str, integer: bool, min: Option<f64>) -> Result<Value> {
    let n = value
        .as_f64()
        .with_context(|| format!("{location}: 缺少必填字段或字段必须是数字"))?;
    ensure!(n.is_finite(), "{location}: 必须是有限数字");
    ensure!(!integer || n.fract() == 0.0, "{location}: 必须是整数");
    ensure!(min.is_none_or(|m| n >= m), "{location}: 小于允许的最小值");
    Ok(value.clone())
}
fn operation(raw: &Value, location: &str) -> Result<Operation> {
    ensure!(raw.is_object(), "{location}: 操作必须是对象");
    let kind = raw["type"]
        .as_str()
        .and_then(|s| OpKind::ALL.into_iter().find(|k| k.label() == s))
        .with_context(|| {
            format!("{location}: 未知操作类型（支持 click/key/press/wheel/drag/wait）")
        })?;
    let mut params = Map::new();
    if let Some(jitter) = raw.get("jitter") {
        let value = number(jitter, &format!("{location}.jitter"), false, Some(0.0))?;
        if value.as_f64() != Some(0.0) {
            params.insert("jitter".into(), value);
        }
    }
    match kind {
        OpKind::Click => {
            params.insert(
                "pos".into(),
                json!(pair(&raw["pos"], &format!("{location}.pos"))?),
            );
            let button = raw.get("button").cloned().unwrap_or(json!("left"));
            ensure!(
                matches!(button.as_str(), Some("left" | "right" | "middle")),
                "{location}: button 只能是 left/right/middle"
            );
            let clicks = raw.get("clicks").cloned().unwrap_or(json!(1));
            ensure!(
                clicks.as_u64().is_some_and(|n| n > 0),
                "{location}: clicks 必须是 ≥1 的整数"
            );
            params.insert("button".into(), button);
            params.insert("clicks".into(), clicks);
        }
        OpKind::Key | OpKind::Press => {
            params.insert(
                "key".into(),
                json!(required_string(&raw["key"], &format!("{location}.key"))?),
            );
            if kind == OpKind::Key {
                let up = raw.get("up").cloned().unwrap_or(json!(false));
                ensure!(up.is_boolean(), "{location}: up 必须是布尔值");
                params.insert("up".into(), up);
            } else {
                params.insert(
                    "ms".into(),
                    number(&raw["ms"], &format!("{location}.ms"), true, Some(1.0))?,
                );
            }
        }
        OpKind::Wheel => {
            let delta = number(&raw["delta"], &format!("{location}.delta"), true, None)?;
            ensure!(delta.as_f64() != Some(0.0), "{location}: delta 不能为 0");
            params.insert("delta".into(), delta);
        }
        OpKind::Drag => {
            for field in ["from", "to"] {
                params.insert(
                    field.into(),
                    json!(pair(&raw[field], &format!("{location}.{field}"))?),
                );
            }
            let duration = raw.get("duration").cloned().unwrap_or(json!(200));
            params.insert(
                "duration".into(),
                number(&duration, &format!("{location}.duration"), true, Some(1.0))?,
            );
        }
        OpKind::Wait => {
            params.insert(
                "ms".into(),
                number(&raw["ms"], &format!("{location}.ms"), true, Some(1.0))?,
            );
        }
    }
    // Normalize all parameters here; unknown operation fields are not inserted
    // into executable caches.
    if params.contains_key("type") {
        bail!("{location}: 参数不能覆盖操作类型");
    }
    Ok(Operation { kind, params })
}
