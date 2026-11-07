//! Captured picker points, grouped by the exact window identity.
use crate::storage::{AppPaths, DiskSnapshot, json_bytes, save_atomic};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerTarget {
    pub process: String,
    #[serde(rename = "windowClass")]
    pub window_class: String,
    pub title: String,
}

impl PickerTarget {
    pub fn new(process: String, window_class: String, title: String) -> Self {
        Self {
            process: process.to_ascii_lowercase(),
            window_class,
            title,
        }
    }

    pub fn equivalent(&self, other: &Self) -> bool {
        self.process.eq_ignore_ascii_case(&other.process)
            && self.window_class == other.window_class
            && self.title == other.title
    }

    fn normalize(&mut self) {
        self.process.make_ascii_lowercase();
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            !self.process.is_empty() || !self.window_class.is_empty(),
            "取点目标必须指定进程或窗口类"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapturedPoint {
    pub id: u64,
    pub name: String,
    pub position: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickerGroup {
    pub target: PickerTarget,
    pub points: Vec<CapturedPoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickerState {
    pub format: String,
    pub last_target: Option<PickerTarget>,
    pub targets: Vec<PickerGroup>,
}

impl Default for PickerState {
    fn default() -> Self {
        Self {
            format: "picker/v1".into(),
            last_target: None,
            targets: Vec::new(),
        }
    }
}

impl PickerState {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.format == "picker/v1", "不支持的取点状态版本");
        if let Some(target) = &self.last_target {
            target.validate().context("最近取点目标无效")?;
        }
        for (index, group) in self.targets.iter().enumerate() {
            group.target.validate().context("取点分组目标无效")?;
            ensure!(
                !self.targets[..index]
                    .iter()
                    .any(|previous| previous.target.equivalent(&group.target)),
                "取点状态包含重复目标"
            );
            let mut ids = HashSet::new();
            for point in &group.points {
                ensure!(point.id != 0, "取点 ID 必须非零");
                ensure!(ids.insert(point.id), "同一目标包含重复取点 ID");
                ensure!(
                    point.position.iter().all(|coordinate| {
                        coordinate.is_finite() && (0.0..=1.0).contains(coordinate)
                    }),
                    "取点坐标必须为 0 到 1 之间的有限数值"
                );
            }
        }
        Ok(())
    }

    pub fn group(&self, target: &PickerTarget) -> Option<&PickerGroup> {
        self.targets
            .iter()
            .find(|group| group.target.equivalent(target))
    }

    /// Replace one target's points; an empty list clears only that group.
    /// Validation is deferred until `validate` or an explicit file save.
    pub fn set_points(&mut self, mut target: PickerTarget, points: Vec<CapturedPoint>) {
        target.normalize();
        if let Some(group) = self
            .targets
            .iter_mut()
            .find(|group| group.target.equivalent(&target))
        {
            group.target = target.clone();
            group.points = points;
        } else {
            self.targets.push(PickerGroup {
                target: target.clone(),
                points,
            });
        }
        self.last_target = Some(target);
    }

    fn normalize(&mut self) {
        if let Some(target) = &mut self.last_target {
            target.normalize();
        }
        for group in &mut self.targets {
            group.target.normalize();
        }
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut state: Self = json_bytes(bytes)?;
        state.normalize();
        state.validate()?;
        Ok(state)
    }
}

#[derive(Clone, Debug)]
pub struct PickerFile {
    pub state: PickerState,
    pub snapshot: DiskSnapshot,
}

impl PickerFile {
    /// Opening a missing state is read-only and yields an empty picker.
    /// Invalid existing content is an error, never an empty replacement state.
    pub fn open(paths: &AppPaths) -> Result<Self> {
        let snapshot = DiskSnapshot::read(&paths.picker_state())?;
        let state = match snapshot.0.as_deref() {
            Some(bytes) => PickerState::parse(bytes).context("取点状态文件无效")?,
            None => PickerState::default(),
        };
        Ok(Self { state, snapshot })
    }

    /// Explicit save with conflict detection, no backups, and semantic no-ops.
    /// A no-op preserves the original bytes and mtime, including a missing file.
    pub fn save(&mut self, paths: &AppPaths) -> Result<()> {
        let mut state = self.state.clone();
        state.normalize();
        state.validate()?;
        let path = paths.picker_state();
        ensure!(
            self.snapshot.unchanged(&path)?,
            "取点状态已被其他程序修改，请重新加载"
        );
        let previous = match self.snapshot.0.as_deref() {
            Some(bytes) => PickerState::parse(bytes).context("原取点状态文件无效")?,
            None => PickerState::default(),
        };
        if previous != state {
            let bytes = serde_json::to_vec_pretty(&state)?;
            self.snapshot = save_atomic(&path, &self.snapshot, &bytes, false)?;
        }
        self.state = state;
        Ok(())
    }
}
