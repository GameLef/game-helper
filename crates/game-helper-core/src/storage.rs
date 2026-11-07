//! Explicit saves with optimistic conflict detection and same-directory replacement.
use crate::{config::Config, route::Route};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub root: PathBuf,
}
impl AppPaths {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root =
            dunce::canonicalize(root.as_ref()).context("无法定位应用目录，请指定 --app-root")?;
        ensure!(
            root.join("ahk/daemon.ahk").is_file() && root.join("config.json").is_file(),
            "应用目录必须包含 ahk/daemon.ahk 和 config.json"
        );
        Ok(Self { root })
    }
    pub fn routes(&self) -> PathBuf {
        self.root.join("routes")
    }
    pub fn cache(&self) -> PathBuf {
        self.routes().join("_cache")
    }
    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }
    pub fn cursor(&self) -> PathBuf {
        self.root.join("state/cursor.json")
    }
    pub fn picker_state(&self) -> PathBuf {
        self.root.join("state/picker.json")
    }
    pub fn route_path(&self, stem: &str) -> Result<PathBuf> {
        validate_stem(stem)?;
        ensure!(
            !self.routes().join(format!("{stem}.yml")).exists(),
            "同名 .yml 路线已存在，请选择其他文件名"
        );
        Ok(self.routes().join(format!("{stem}.yaml")))
    }
    pub fn cache_path(&self, stem: &str) -> Result<PathBuf> {
        validate_stem(stem)?;
        Ok(self.cache().join(format!("{stem}.json")))
    }
}

pub fn validate_stem(stem: &str) -> Result<()> {
    ensure!(
        !stem.is_empty() && stem != "." && stem != ".." && !stem.ends_with(['.', ' ']),
        "路线文件名为空或末尾无效"
    );
    ensure!(
        !stem
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)),
        "路线文件名包含非法字符"
    );
    let base = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    ensure!(
        !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(1..=9).any(|i| base == format!("COM{i}") || base == format!("LPT{i}")),
        "Windows 保留的文件名"
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskSnapshot(pub Option<Vec<u8>>);
impl DiskSnapshot {
    pub fn read(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => Ok(Self(Some(bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self(None)),
            Err(error) => Err(error).with_context(|| format!("读取 {} 失败", path.display())),
        }
    }
    pub fn unchanged(&self, path: &Path) -> Result<bool> {
        Ok(Self::read(path)? == *self)
    }
}

/// Best-effort copy of the previous content into `<parent>/.backups`, unique
/// by timestamp nanoseconds plus a process-unique suffix.
fn backup_original(parent: &Path, filename: &str, suffix: &str, original: &[u8]) -> Result<()> {
    let backups = parent.join(".backups");
    fs::create_dir_all(&backups)?;
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(backups.join(format!("{filename}.{epoch}.{suffix}.bak")))?;
    file.write_all(original)?;
    file.sync_all()?;
    Ok(())
}

/// Snapshot comparison does not lock arbitrary external editors. The UI also
/// limits this client's writers to one instance and refuses detected conflicts.
pub fn save_atomic(
    path: &Path,
    expected: &DiskSnapshot,
    bytes: &[u8],
    backup: bool,
) -> Result<DiskSnapshot> {
    ensure!(
        expected.unchanged(path)?,
        "文件已被其他程序修改，请重新加载后保存：{}",
        path.display()
    );
    if expected.0.as_deref() == Some(bytes) {
        return Ok(expected.clone());
    }
    let parent = path.parent().context("保存路径缺少父目录")?;
    fs::create_dir_all(parent)?;
    let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    let filename = path
        .file_name()
        .context("保存路径缺少文件名")?
        .to_string_lossy();
    let suffix = format!("{}-{id}", std::process::id());
    let temporary = parent.join(format!(".{filename}.{suffix}.tmp"));
    let operation = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if backup && let Some(original) = expected.0.as_ref() {
            backup_original(parent, &filename, &suffix, original)?;
        }
        ensure!(
            expected.unchanged(path)?,
            "保存前检测到磁盘冲突：{}",
            path.display()
        );
        fs::rename(&temporary, path)
            .with_context(|| format!("替换 {} 失败，原文件保留", path.display()))?;
        Ok(())
    })();
    if operation.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    operation?;
    Ok(DiskSnapshot(Some(bytes.to_vec())))
}

pub fn json_bytes<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
        .context("JSON 格式错误")
}

/// Read `state/cursor.json` (written by the AHK daemon). A missing file is the
/// normal "daemon has not stepped yet" state, not an error.
pub fn read_cursor(paths: &AppPaths) -> Result<CursorState> {
    match DiskSnapshot::read(&paths.cursor())? {
        DiskSnapshot(None) => Ok(CursorState::default()),
        DiskSnapshot(Some(bytes)) => json_bytes(&bytes),
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CursorState {
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, RouteCursor>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteCursor {
    pub last_done: usize,
    pub total: usize,
    pub start_at: usize,
}

#[derive(Clone, Debug)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub value: Config,
    snapshot: DiskSnapshot,
}
impl ConfigFile {
    pub fn open(path: PathBuf) -> Result<Self> {
        let snapshot = DiskSnapshot::read(&path)?;
        let value = Config::parse(snapshot.0.as_deref().context("未找到 config.json")?)?;
        Ok(Self {
            path,
            value,
            snapshot,
        })
    }
    /// Byte snapshot of config.json as last read/written by this client.
    pub fn disk_snapshot(&self) -> &DiskSnapshot {
        &self.snapshot
    }
    pub fn save(&mut self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.value)?;
        // No-op saves must not touch the file: repeated UI refreshes would
        // otherwise keep rewriting mtime and backups.
        ensure!(
            self.snapshot.unchanged(&self.path)?,
            "配置已被其他程序修改，请重新加载"
        );
        if self.snapshot.0.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        if let Some(original) = self.snapshot.0.as_deref() {
            let previous = Config::parse(original)?;
            if serde_json::to_value(previous)? == serde_json::to_value(&self.value)? {
                return Ok(());
            }
        }
        self.snapshot = save_atomic(&self.path, &self.snapshot, &bytes, true)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RouteEntry {
    pub path: PathBuf,
    pub stem: String,
    pub name: String,
    pub points: usize,
    pub error: Option<String>,
}
pub fn list_routes(paths: &AppPaths) -> Result<Vec<RouteEntry>> {
    let mut entries = Vec::new();
    if !paths.routes().exists() {
        return Ok(entries);
    }
    // One unreadable directory entry must not hide the whole list; each file
    // degrades to its own error message instead.
    for file in fs::read_dir(paths.routes())? {
        let entry = match file {
            Ok(entry) => entry,
            Err(error) => {
                entries.push(RouteEntry {
                    path: paths.routes().clone(),
                    stem: String::new(),
                    name: String::new(),
                    points: 0,
                    error: Some(format!("目录项读取失败: {error}")),
                });
                continue;
            }
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if !matches!(extension.as_deref(), Some("yaml" | "yml")) {
            continue;
        }
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        let stem = path
            .file_stem()
            .context("路线文件名无效")?
            .to_string_lossy()
            .into_owned();
        let result = fs::read_to_string(&path)
            .context("读取路线失败")
            .and_then(|s| Route::parse(&s));
        let (name, points, error) = match result {
            Ok(route) => (route.name, route.points.len(), None),
            Err(error) => (format!("{stem}（无效）"), 0, Some(format!("{error:#}"))),
        };
        entries.push(RouteEntry {
            path,
            stem,
            name,
            points,
            error,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

pub fn cache_route(paths: &AppPaths, stem: &str, route: &Route) -> Result<PathBuf> {
    let path = paths.cache_path(stem)?;
    let bytes = route.cache_json()?;
    save_atomic(&path, &DiskSnapshot::read(&path)?, bytes.as_bytes(), false)?;
    Ok(path)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeletionReport {
    pub source_deleted: bool,
    pub cache_deleted: bool,
    pub binding_removed: bool,
    pub backup_path: Option<PathBuf>,
    /// Partial failures so far; the operation stops at the first hard error.
    pub errors: Vec<String>,
}
impl std::fmt::Display for DeletionReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "源文件已删={} 缓存已删={} 绑定已移除={} 备份目录={}",
            self.source_deleted,
            self.cache_deleted,
            self.binding_removed,
            self.backup_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "无".into())
        )?;
        for error in &self.errors {
            write!(formatter, "\n  {error}")?;
        }
        Ok(())
    }
}

/// Delete one route: source YAML, executable cache, then config binding.
///
/// `expected` must be the snapshot the UI saw when it asked for deletion; an
/// external modification since then aborts before anything is removed. The
/// caller passes the UI's authoritative path — busy/running checks live there.
/// The stem is derived from `path` so cache/binding cleanup can never target a
/// different route than the deleted file.
///
/// Safety rails: the path must be the canonical `<root>/routes/<stem>.yaml`
/// (stopping same-stem `.yml` siblings and any path outside `routes/` from
/// being touched), and the original bytes are backed up to `routes/.backups`
/// before removal. Steps run in the order source → cache → binding, stopping
/// at the first failure; everything already done is reported in the returned
/// error alongside the failure, never silently.
pub fn delete_route(
    paths: &AppPaths,
    config: &mut ConfigFile,
    path: &Path,
    expected: &DiskSnapshot,
) -> Result<()> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("路线文件名无效")?
        .to_string();
    validate_stem(&stem)?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    ensure!(
        path.parent() == Some(paths.routes().as_path()) && matches!(extension, "yaml" | "yml"),
        "只允许删除 routes/ 下的 YAML 路线文件：{}",
        path.display()
    );
    let sibling = paths.routes().join(format!(
        "{stem}.{}",
        if extension == "yaml" { "yml" } else { "yaml" }
    ));
    ensure!(
        !sibling.exists(),
        "存在同 stem 的 YAML/YML 文件，共用缓存与绑定；请先解决文件名冲突再删除"
    );
    ensure!(
        expected.unchanged(path)?,
        "文件已被其他程序修改，请重新加载后删除：{}",
        path.display()
    );
    let mut report = DeletionReport::default();
    let original = expected.0.clone();
    // 1. Source file first: if this fails, nothing was touched.
    if let Some(bytes) = original.as_ref() {
        // A backup before deletion is the only recovery path left afterwards.
        if let Err(error) = backup_original(
            path.parent().context("路线路径缺少父目录")?,
            &format!("{stem}.yaml"),
            &std::process::id().to_string(),
            bytes,
        ) {
            report
                .errors
                .push(format!("删除前备份失败（未删除任何文件）: {error:#}"));
            return Err(anyhow::anyhow!("{report:#}").context("删除路线中止"));
        }
        report.backup_path = Some(path.parent().unwrap().join(".backups"));
    }
    match fs::remove_file(path) {
        Ok(()) => report.source_deleted = true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => report.source_deleted = true,
        Err(error) => {
            report
                .errors
                .push(format!("删除路线文件失败（未动缓存与绑定）: {error}"));
            return Err(anyhow::anyhow!("{report:#}").context("删除路线未完成"));
        }
    }
    // 2. Cache: tolerate a missing cache, report a real failure but continue
    //    to unbinding — a leftover cache alone would resurrect the route for
    //    the daemon only until reload, while a leftover binding persists.
    let cache = paths.cache_path(&stem)?;
    match fs::remove_file(&cache) {
        Ok(()) => report.cache_deleted = true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => report.cache_deleted = true,
        Err(error) => report.errors.push(format!(
            "删除执行缓存失败（路线文件已删）: {}：{error}",
            cache.display()
        )),
    }
    // 3. Binding removal is an explicit config save; a failure here is real.
    if let Err(error) = config
        .value
        .set_binding(&stem, "", 1)
        .and_then(|_| config.save())
    {
        report
            .errors
            .push(format!("源文件已删除，但解绑配置保存失败：{error:#}"));
        return Err(anyhow::anyhow!("{report:#}").context("删除路线部分完成"));
    }
    report.binding_removed = true;
    if report.errors.is_empty() {
        return Ok(());
    }
    Err(anyhow::anyhow!("{report:#}").context("路线已删除，但部分清理未完成"))
}
