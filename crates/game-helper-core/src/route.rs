//! route/v1 models and validated YAML/JSON conversion.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};

mod validate;
pub const FORMAT: &str = "route/v1";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub process: String,
    #[serde(rename = "windowClass")]
    pub window_class: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub format: String,
    pub name: String,
    pub note: String,
    pub target: Target,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub id: u64,
    pub name: String,
    pub note: String,
    #[serde(serialize_with = "serialize_anchor")]
    pub pos: [f64; 2],
    pub operations: Vec<Operation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
    Click,
    Key,
    Press,
    Wheel,
    Drag,
    Wait,
}
impl OpKind {
    pub const ALL: [Self; 6] = [
        Self::Click,
        Self::Key,
        Self::Press,
        Self::Wheel,
        Self::Drag,
        Self::Wait,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Key => "key",
            Self::Press => "press",
            Self::Wheel => "wheel",
            Self::Drag => "drag",
            Self::Wait => "wait",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    #[serde(rename = "type")]
    pub kind: OpKind,
    #[serde(flatten)]
    pub params: Map<String, Value>,
}
impl Operation {
    pub fn new(kind: OpKind, anchor: [f64; 2]) -> Self {
        let value = match kind {
            OpKind::Click => json!({"pos": anchor, "button": "left", "clicks": 1}),
            OpKind::Key => json!({"key": "w", "up": false}),
            OpKind::Press => json!({"key": "w", "ms": 500}),
            OpKind::Wheel => json!({"delta": 1}),
            OpKind::Drag => json!({"from": anchor, "to": anchor, "duration": 200}),
            OpKind::Wait => json!({"ms": 100}),
        };
        Self {
            kind,
            params: value.as_object().cloned().unwrap_or_default(),
        }
    }
}

impl Point {
    pub fn new(id: u64, pos: [f64; 2]) -> Self {
        Self {
            id,
            pos,
            name: format!("点位 {id}"),
            note: String::new(),
            operations: Vec::new(),
        }
    }
}

impl Route {
    /// A draft may have an empty target; it cannot be saved/executed until valid.
    pub fn draft(name: impl Into<String>) -> Self {
        Self {
            format: FORMAT.into(),
            name: name.into(),
            note: String::new(),
            target: Target::default(),
            points: Vec::new(),
        }
    }
    pub fn parse(source: &str) -> Result<Self> {
        ensure!(!source.trim().is_empty(), "路线文件为空");
        let source = source.trim_start_matches('\u{feff}');
        let raw: Value = yaml_serde::from_str(source).map_err(|error| {
            let line = error
                .location()
                .map(|p| format!("（第 {} 行，第 {} 列）", p.line(), p.column()))
                .unwrap_or_default();
            anyhow::anyhow!("YAML 语法错误{line}: {error}")
        })?;
        validate::route(&raw)
    }
    pub fn from_value(value: &Value) -> Result<Self> {
        validate::route(value)
    }
    pub fn validate(&self) -> Result<()> {
        // Validate unrounded values with per-point locations: out-of-range
        // inputs cannot become valid by rounding. Mirrors the YAML-path errors.
        for (index, point) in self.points.iter().enumerate() {
            ensure!(
                point
                    .pos
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "points[{index}] #{}({}).pos: 坐标超出 0~1 范围",
                point.id,
                point.name
            );
        }
        validate::route(&serde_json::to_value(self)?)?;
        Ok(())
    }
    pub fn yaml(&self) -> Result<String> {
        self.validate()?;
        yaml_serde::to_string(self).context("生成 YAML 失败")
    }
    pub fn cache_json(&self) -> Result<String> {
        self.validate()?;
        // Normalize omitted defaults and zero jitter before producing the AHK cache.
        let route = validate::route(&serde_json::to_value(self)?)?;
        serde_json::to_string(&route).context("生成路线缓存失败")
    }
    pub fn next_id(&self) -> u64 {
        self.points
            .iter()
            .map(|p| p.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
}

fn serialize_anchor<S: Serializer>(
    pos: &[f64; 2],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let rounded = pos.map(|v| {
        // Decimal formatting rounds the underlying float to six decimal places,
        // avoiding multiplication-induced halfway errors before ties-to-even.
        format!("{v:.6}").parse::<f64>().unwrap_or(v)
    });
    rounded.serialize(serializer)
}
