//! Route-manager presentation state, stored separately from executable routes.
use crate::{
    route::{Operation, Point},
    storage::{AppPaths, DiskSnapshot, json_bytes, save_atomic},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

pub const FORMAT: &str = "manager-ui/v1";

/// A point ID is not required to be unique in route/v1. Occurrence is zero-based
/// among points with this ID, rather than the point's absolute list position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PointUiKey {
    pub id: u64,
    pub occurrence: usize,
}

impl PointUiKey {
    pub fn new(id: u64, occurrence: usize) -> Self {
        Self { id, occurrence }
    }

    pub fn for_index(points: &[Point], index: usize) -> Option<Self> {
        let id = points.get(index)?.id;
        Some(Self::new(
            id,
            points[..index]
                .iter()
                .filter(|point| point.id == id)
                .count(),
        ))
    }
}

/// Move this entire value when reordering actions. Update `descriptor` after a
/// successful form edit to keep its identity even when its parameters change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationUiState {
    pub id: u64,
    pub expanded: bool,
    pub descriptor: Operation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointUiState {
    pub key: PointUiKey,
    pub operations: Vec<OperationUiState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerUiState {
    pub format: String,
    /// The next identity to allocate; never decreases when entries are removed.
    pub next_id: u64,
    pub routes: BTreeMap<String, Vec<PointUiState>>,
}

impl Default for ManagerUiState {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            next_id: 1,
            routes: BTreeMap::new(),
        }
    }
}

/// Produce a portable, case-folded key without requiring a route to exist.
/// Absolute paths must be inside the app root. Relative paths are rooted at the
/// app root, and `..` may not escape it. Both Windows separators are accepted.
pub fn normalize_route_key(paths: &AppPaths, path: impl AsRef<Path>) -> Result<String> {
    let path = path
        .as_ref()
        .to_str()
        .context("路线文件路径不是有效 Unicode")?;
    let path = path.replace('\\', "/");
    if is_absolute(&path) {
        let root = paths.root.to_str().context("应用路径不是有效 Unicode")?;
        let root = normalize_path(&root.replace('\\', "/"), true)?;
        let path = normalize_path(&path, true)?;
        let relative = path
            .strip_prefix(&format!("{}/", root.trim_end_matches('/')))
            .context("路线文件不在应用目录内")?;
        normalize_relative_key(relative)
    } else {
        normalize_relative_key(&path)
    }
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || path.as_bytes().get(1) == Some(&b':')
}

fn normalize_path(path: &str, absolute: bool) -> Result<String> {
    let path = path.strip_prefix("//?/").unwrap_or(path);
    let prefix = if path.starts_with("//") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    };
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                ensure!(
                    components
                        .last()
                        .is_some_and(|part: &String| !part.ends_with(':')),
                    "路径超出根目录"
                );
                components.pop();
            }
            value => {
                ensure!(!value.chars().any(char::is_control), "路径包含控制字符");
                ensure!(
                    absolute || !value.contains(':'),
                    "路线状态路径必须相对于应用目录"
                );
                components.push(value.to_lowercase());
            }
        }
    }
    ensure!(!components.is_empty(), "路线状态路径为空");
    Ok(format!("{prefix}{}", components.join("/")))
}

fn normalize_relative_key(path: &str) -> Result<String> {
    let path = path.replace('\\', "/");
    ensure!(!is_absolute(&path), "路线状态路径必须相对于应用目录");
    normalize_path(&path, false)
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let sorted: BTreeMap<_, _> = values
                .iter()
                .map(|(key, value)| (key.clone(), canonical_value(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        value => value.clone(),
    }
}

fn canonical_operation(operation: &Operation) -> Operation {
    let mut operation = operation.clone();
    let defaults = Operation::new(operation.kind, [0.5, 0.5]);
    // These fields are optional in route/v1 and materialized by the form/parser.
    // Required coordinates, keys and durations must not be invented here.
    for key in [
        "button",
        "clicks",
        "up",
        "duration",
        "speed",
        "pre_drag_distance",
        "pre_drag_speed",
        "pre_drag_delay",
    ] {
        if let Some(value) = defaults.params.get(key) {
            operation
                .params
                .entry(key.to_owned())
                .or_insert_with(|| value.clone());
        }
    }
    if operation.params.get("jitter").and_then(Value::as_f64) == Some(0.0) {
        operation.params.remove("jitter");
    }
    let params: BTreeMap<_, _> = operation
        .params
        .iter()
        .map(|(key, value)| (key.clone(), canonical_value(value)))
        .collect();
    Operation {
        kind: operation.kind,
        name: operation.name,
        params: params.into_iter().collect(),
    }
}

impl ManagerUiState {
    pub fn point(&self, route: &str, key: PointUiKey) -> Option<&PointUiState> {
        self.routes
            .get(&normalize_relative_key(route).ok()?)?
            .iter()
            .find(|point| point.key == key)
    }

    /// Allocate a global stable identity. The caller may change `expanded` before
    /// insertion. Exhaustion is reported without modifying the state.
    pub fn fresh_operation(&mut self, descriptor: &Operation) -> Result<OperationUiState> {
        let maximum = self
            .routes
            .values()
            .flatten()
            .flat_map(|point| &point.operations)
            .map(|operation| operation.id)
            .max()
            .unwrap_or(0);
        let id = self
            .next_id
            .max(maximum.checked_add(1).context("动作界面身份已耗尽")?);
        let next = id.checked_add(1).context("动作界面身份已耗尽")?;
        self.next_id = next;
        Ok(OperationUiState {
            id,
            expanded: true,
            descriptor: canonical_operation(descriptor),
        })
    }

    /// Exact descriptor matching consumes stored duplicates from left to right.
    /// Unmatched operations receive fresh identities and start expanded. This
    /// returns a detached editable vector; call `put_point` after editing it.
    pub fn reconcile_point(
        &mut self,
        route: &str,
        key: PointUiKey,
        operations: &[Operation],
    ) -> Result<Vec<OperationUiState>> {
        normalize_relative_key(route)?;
        ensure!(key.id != 0, "点位界面 ID 必须非零");
        let previous = self
            .point(route, key)
            .map(|point| point.operations.clone())
            .unwrap_or_default();
        let mut consumed = vec![false; previous.len()];
        // Work on a copy so allocation failure never partially advances this state.
        let mut working = self.clone();
        let mut result = Vec::with_capacity(operations.len());
        for operation in operations {
            let descriptor = canonical_operation(operation);
            if let Some(index) = previous
                .iter()
                .enumerate()
                .position(|(index, stored)| !consumed[index] && stored.descriptor == descriptor)
            {
                consumed[index] = true;
                result.push(previous[index].clone());
            } else {
                result.push(working.fresh_operation(&descriptor)?);
            }
        }
        self.next_id = working.next_id;
        Ok(result)
    }

    /// Replace one point while retaining the vector's order and identities.
    /// Parameter edits should update descriptors on these same values, rather
    /// than call reconcile (which intentionally matches only exact content).
    pub fn put_point(
        &mut self,
        route: &str,
        key: PointUiKey,
        operations: Vec<OperationUiState>,
    ) -> Result<()> {
        let route = normalize_relative_key(route)?;
        ensure!(key.id != 0, "点位界面 ID 必须非零");
        let mut identities = HashSet::new();
        for (name, points) in &self.routes {
            for point in points {
                if name != &route || point.key != key {
                    identities.extend(point.operations.iter().map(|operation| operation.id));
                }
            }
        }
        for operation in &operations {
            ensure!(
                operation.id != 0 && identities.insert(operation.id),
                "动作界面身份为零或重复"
            );
            ensure!(operation.id < u64::MAX, "动作界面身份已耗尽");
        }
        let operations: Vec<_> = operations
            .into_iter()
            .map(|mut operation| {
                operation.descriptor = canonical_operation(&operation.descriptor);
                operation
            })
            .collect();
        if let Some(maximum) = operations.iter().map(|operation| operation.id).max() {
            self.next_id = self.next_id.max(maximum + 1);
        }
        let points = self.routes.entry(route).or_default();
        if let Some(point) = points.iter_mut().find(|point| point.key == key) {
            point.operations = operations;
        } else {
            points.push(PointUiState { key, operations });
        }
        points.sort_by_key(|point| point.key);
        Ok(())
    }

    /// Copy presentation choices, but allocate independent action identities.
    pub fn copy_operations(
        &mut self,
        operations: &[OperationUiState],
    ) -> Result<Vec<OperationUiState>> {
        let mut working = self.clone();
        let mut result = Vec::with_capacity(operations.len());
        for operation in operations {
            let mut copied = working.fresh_operation(&operation.descriptor)?;
            copied.expanded = operation.expanded;
            result.push(copied);
        }
        self.next_id = working.next_id;
        Ok(result)
    }

    pub fn remove_point(&mut self, route: &str, key: &PointUiKey) {
        if let Ok(route) = normalize_relative_key(route)
            && let Some(points) = self.routes.get_mut(&route)
        {
            points.retain(|point| &point.key != key);
        }
    }

    pub fn remove_route(&mut self, route: &str) {
        if let Ok(route) = normalize_relative_key(route) {
            self.routes.remove(&route);
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.format == FORMAT, "不支持的管理器界面状态版本");
        ensure!(self.next_id != 0, "动作界面身份计数必须非零");
        let mut routes = HashSet::new();
        let mut identities = HashSet::new();
        for (route, points) in &self.routes {
            ensure!(
                routes.insert(normalize_relative_key(route)?),
                "管理器界面状态包含重复路线"
            );
            let mut keys = HashSet::new();
            for point in points {
                ensure!(
                    point.key.id != 0 && keys.insert(point.key),
                    "管理器界面状态包含无效或重复点位键"
                );
                for operation in &point.operations {
                    ensure!(
                        operation.id != 0 && identities.insert(operation.id),
                        "动作界面身份为零或重复"
                    );
                    ensure!(operation.id < self.next_id, "动作界面身份计数无效");
                    ensure!(
                        !operation.descriptor.params.contains_key("type")
                            && !operation.descriptor.params.contains_key("name"),
                        "动作描述包含保留参数名"
                    );
                }
            }
        }
        Ok(())
    }

    fn normalize(&mut self) -> Result<()> {
        self.validate()?;
        let mut routes = BTreeMap::new();
        for (route, mut points) in std::mem::take(&mut self.routes) {
            for point in &mut points {
                for operation in &mut point.operations {
                    operation.descriptor = canonical_operation(&operation.descriptor);
                }
            }
            points.sort_by_key(|point| point.key);
            routes.insert(normalize_relative_key(&route)?, points);
        }
        self.routes = routes;
        Ok(())
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut state: Self = json_bytes(bytes)?;
        state.normalize()?;
        Ok(state)
    }
}

#[derive(Clone, Debug)]
pub struct ManagerUiFile {
    pub state: ManagerUiState,
    pub snapshot: DiskSnapshot,
}

impl ManagerUiFile {
    /// Missing state is normal. Malformed or unsupported state is never replaced
    /// with a writable empty state: preserve the original and report the error.
    pub fn open(paths: &AppPaths) -> Result<Self> {
        let snapshot = DiskSnapshot::read(&paths.manager_ui_state())?;
        let state = match snapshot.0.as_deref() {
            Some(bytes) => ManagerUiState::parse(bytes).context("管理器界面状态文件无效")?,
            None => ManagerUiState::default(),
        };
        Ok(Self { state, snapshot })
    }

    /// Atomic save with optimistic conflict detection and semantic no-op checks.
    /// A no-op preserves original bytes, BOM, modification time and missing files.
    pub fn save(&mut self, paths: &AppPaths) -> Result<()> {
        let mut state = self.state.clone();
        state.normalize()?;
        let path = paths.manager_ui_state();
        ensure!(
            self.snapshot.unchanged(&path)?,
            "管理器界面状态已被其他程序修改，请重新加载"
        );
        let previous = match self.snapshot.0.as_deref() {
            Some(bytes) => ManagerUiState::parse(bytes).context("原管理器界面状态文件无效")?,
            None => ManagerUiState::default(),
        };
        if previous != state {
            let bytes = serde_json::to_vec_pretty(&state)?;
            self.snapshot = save_atomic(&path, &self.snapshot, &bytes, false)?;
        }
        self.state = state;
        Ok(())
    }
}
