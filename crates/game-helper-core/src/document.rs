//! Two editing views with one explicitly chosen source of truth per document.
use crate::{
    route::Route,
    storage::{AppPaths, DiskSnapshot, cache_route, save_atomic},
};
use anyhow::{Context, Result, ensure};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    Form,
    Source,
}

#[derive(Clone, Debug)]
pub struct RouteDocument {
    pub path: PathBuf,
    mode: EditMode,
    route: Route,
    source: String,
    saved_source: String,
    /// Form edits exist that `source` does not reflect yet. Survives the
    /// Form→Source export (the generated text merely mirrors the model) and is
    /// cleared only by re-syncing: user-authored `set_source`, the Source→Form
    /// parse, or a successful save.
    structured_edit: bool,
    /// The current `source` text was machine-generated, not loaded or typed.
    /// Drives the normalization notice after Form→Source→Form round trips.
    source_generated: bool,
    snapshot: DiskSnapshot,
}
impl RouteDocument {
    pub fn open(path: PathBuf) -> Result<Self> {
        let snapshot = DiskSnapshot::read(&path)?;
        let source = String::from_utf8(snapshot.0.clone().context("路线文件已不存在")?)
            .context("路线不是 UTF-8")?;
        let route = Route::parse(&source)?;
        Ok(Self {
            path,
            mode: EditMode::Form,
            route,
            saved_source: source.clone(),
            source,
            structured_edit: false,
            source_generated: false,
            snapshot,
        })
    }
    pub fn draft(path: PathBuf, name: String) -> Result<Self> {
        let snapshot = DiskSnapshot::read(&path)?;
        ensure!(snapshot.0.is_none(), "同名路线已存在");
        let route = Route::draft(name);
        let source = yaml_serde::to_string(&route)?;
        Ok(Self {
            path,
            mode: EditMode::Form,
            route,
            source,
            saved_source: String::new(),
            structured_edit: true,
            source_generated: true,
            snapshot,
        })
    }
    /// Import a validated source into a route path. The source must parse and
    /// validate; nothing is written until an explicit `save`.
    ///
    /// The imported text is user-provided original text: a save commits it
    /// verbatim (source-save semantics), so no normalization acknowledgement
    /// is needed until the user starts editing in the form view.
    ///
    /// When the target already exists on disk, `overwrite_acknowledged == false`
    /// aborts so the UI can offer cancel / save-as / overwrite first. With
    /// acknowledgement the current target bytes are recorded as this document's
    /// snapshot: a save then replaces exactly those bytes with backup + conflict
    /// detection instead of clobbering an unknown file state.
    pub fn from_source(path: PathBuf, source: &str, overwrite_acknowledged: bool) -> Result<Self> {
        let route = Route::parse(source).context("导入的路线内容无效")?;
        let snapshot = DiskSnapshot::read(&path)?;
        if snapshot.0.is_some() {
            ensure!(
                overwrite_acknowledged,
                "同名路线已存在：{}。请确认覆盖、另存或取消",
                path.display()
            );
        }
        // Model and source are already in sync; `saved_source` stays empty so
        // the document counts as dirty until its first explicit save.
        Ok(Self {
            path,
            mode: EditMode::Form,
            route,
            source: source.into(),
            saved_source: String::new(),
            structured_edit: false,
            source_generated: false,
            snapshot,
        })
    }
    /// Open broken YAML in source mode so it can be repaired without losing input.
    pub fn open_for_edit(path: PathBuf) -> Result<Self> {
        match Self::open(path.clone()) {
            Ok(document) => Ok(document),
            Err(_) => {
                let snapshot = DiskSnapshot::read(&path)?;
                let source = String::from_utf8(snapshot.0.clone().context("路线文件已不存在")?)?;
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                Ok(Self {
                    path,
                    mode: EditMode::Source,
                    route: Route::draft(name),
                    saved_source: source.clone(),
                    source,
                    structured_edit: false,
                    source_generated: false,
                    snapshot,
                })
            }
        }
    }
    pub fn disk_snapshot(&self) -> &DiskSnapshot {
        &self.snapshot
    }
    pub fn route(&self) -> &Route {
        &self.route
    }
    pub fn mode(&self) -> EditMode {
        self.mode
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn stem(&self) -> Result<String> {
        Ok(self
            .path
            .file_stem()
            .context("文件名无效")?
            .to_str()
            .context("文件名不是 Unicode")?
            .into())
    }
    pub fn dirty(&self) -> bool {
        self.structured_edit || self.source != self.saved_source
    }
    pub fn edit(&mut self, edit: impl FnOnce(&mut Route)) -> Result<()> {
        ensure!(self.mode == EditMode::Form, "请先切换到点位编辑视图");
        edit(&mut self.route);
        self.structured_edit = true;
        Ok(())
    }
    /// A broken source remains in memory until fixed or explicitly discarded.
    /// User typing marks the source as authored; a stale generated export no
    /// longer claims normalization on the next Form switch.
    pub fn set_source(&mut self, source: String) -> Result<()> {
        ensure!(self.mode == EditMode::Source, "当前不在 YAML 源码视图");
        self.source = source;
        self.source_generated = false;
        Ok(())
    }
    pub fn switch(&mut self, mode: EditMode) -> Result<()> {
        if mode == self.mode {
            return Ok(());
        }
        match mode {
            EditMode::Form => {
                let parsed = Route::parse(&self.source)?;
                self.route = parsed;
                // One source of truth: a successful parse means the source
                // fully describes the model; only sync clears the edit flag.
                // `source_generated` stays: the source text itself is still
                // the machine export and still differs from disk.
                self.structured_edit = false;
            }
            EditMode::Source => {
                if self.structured_edit {
                    self.source = self.route.yaml()?;
                    self.source_generated = true;
                }
            }
        }
        self.mode = mode;
        Ok(())
    }
    /// True when saving now would replace on-disk YAML with text that differs
    /// from the file (layout/comments/unknown fields lost to regeneration).
    /// Derived, not remembered: it stays correct after Form→Source→Form round
    /// trips, and clears only when the text to save matches disk (user typed
    /// it back, source was saved, or generation was identical).
    pub fn needs_normalization_notice(&self) -> bool {
        let text = match self.pending_save_text() {
            Some(text) => text,
            None => return false,
        };
        let on_disk = self.snapshot.0.as_ref().map(|bytes| {
            String::from_utf8_lossy(bytes)
                .trim_start_matches('\u{feff}')
                .to_string()
        });
        !matches!(on_disk, Some(disk) if disk == text)
    }
    /// The text the next save would write, or None when a save would commit
    /// user-authored text verbatim (no normalization to acknowledge).
    fn pending_save_text(&self) -> Option<String> {
        match self.mode {
            // Generated export still on screen in Source view.
            EditMode::Source if self.source_generated => Some(self.source.clone()),
            EditMode::Source => None,
            // Form edits regenerate from the model at save time.
            EditMode::Form if self.structured_edit => Some(self.route.yaml().ok()?),
            // Form after a generated round trip: the screen text is still the
            // machine export; save commits it verbatim, comments lost.
            EditMode::Form if self.source_generated => Some(self.source.clone()),
            EditMode::Form => None,
        }
    }
    pub fn source_for_save(&self) -> Result<(Route, String)> {
        let (route, source) = match self.mode {
            EditMode::Source => (Route::parse(&self.source)?, self.source.clone()),
            EditMode::Form if self.structured_edit => (self.route.clone(), self.route.yaml()?),
            EditMode::Form => (self.route.clone(), self.source.clone()),
        };
        route.validate()?;
        Ok((route, source))
    }
    pub fn save(&mut self, paths: &AppPaths, normalization_acknowledged: bool) -> Result<PathBuf> {
        ensure!(
            !self.needs_normalization_notice() || normalization_acknowledged,
            "结构化保存会规范化 YAML 并可能移除注释，请确认；原文将备份"
        );
        let (route, source) = self.source_for_save()?;
        let stem = self.stem()?;
        let snapshot = save_atomic(&self.path, &self.snapshot, source.as_bytes(), true)?;
        // Source save has committed. Preserve this snapshot even if cache IO fails,
        // so retry does not misclassify this client's own save as an external edit.
        self.snapshot = snapshot;
        self.saved_source = source.clone();
        self.source = source;
        self.route = route;
        self.structured_edit = false;
        self.source_generated = false;
        cache_route(paths, &stem, &self.route)
            .context("路线已保存，但执行缓存更新失败；请重试缓存生成")
    }
    pub fn export_source(&self) -> Result<String> {
        self.source_for_save().map(|(_, text)| text)
    }
}
