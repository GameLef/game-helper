//! Application root resolution shared by the manager and the picker.
//!
//! Resolution order: explicit `--app-root` first, then the portable exe
//! directory, then the development walk-up looking for `ahk/daemon.ahk` +
//! `config.json`. No data is created on failure; the caller shows a clear
//! error and may offer a directory picker.

use game_helper_core::storage::AppPaths;
use std::path::{Path, PathBuf};

/// A resolved application root, or the reason resolution failed.
#[derive(Clone, Debug)]
pub enum ResolvedRoot {
    /// Canonical paths rooted at a valid application directory.
    Ok(AppPaths),
    /// Nothing matched; carries every candidate that was tried.
    Failed(Vec<PathBuf>),
}

impl ResolvedRoot {
    /// Validate one candidate directory: it must contain both markers.
    fn candidate(path: PathBuf) -> Option<PathBuf> {
        if path.join("ahk/daemon.ahk").is_file() && path.join("config.json").is_file() {
            Some(path)
        } else {
            None
        }
    }

    /// Resolve the root for the current process. `explicit` is the parsed
    /// `--app-root` value; `exe_dir` is `current_exe().parent()`.
    pub fn resolve(explicit: Option<&Path>, exe_dir: Option<&Path>) -> Self {
        let mut tried = Vec::new();
        if let Some(root) = explicit {
            tried.push(root.to_path_buf());
            if let Some(found) = Self::candidate(root.to_path_buf()) {
                return match AppPaths::new(&found) {
                    Ok(paths) => Self::Ok(paths),
                    Err(error) => {
                        tried.push(PathBuf::from(format!("AppPaths 校验失败: {error:#}")));
                        Self::Failed(tried)
                    }
                };
            }
        }
        if let Some(dir) = exe_dir {
            tried.push(dir.to_path_buf());
            if let Some(found) = Self::candidate(dir.to_path_buf()) {
                if let Ok(paths) = AppPaths::new(&found) {
                    return Self::Ok(paths);
                }
            }
            // Development layout: target/debug/game-helper → walk up to the repo.
            let mut parent = dir.parent();
            while let Some(ancestor) = parent {
                tried.push(ancestor.to_path_buf());
                if let Some(found) = Self::candidate(ancestor.to_path_buf()) {
                    if let Ok(paths) = AppPaths::new(&found) {
                        return Self::Ok(paths);
                    }
                }
                parent = ancestor.parent();
            }
        }
        Self::Failed(tried)
    }

    /// The resolved paths, when resolution succeeded.
    pub fn paths(&self) -> Option<&AppPaths> {
        match self {
            Self::Ok(paths) => Some(paths),
            Self::Failed(_) => None,
        }
    }

    /// A user-facing failure message listing the attempted candidates.
    pub fn error_message(&self) -> String {
        match self {
            Self::Ok(_) => String::new(),
            Self::Failed(tried) => {
                let mut message = String::from(
                    "未能定位应用目录：需要包含 ahk/daemon.ahk 与 config.json。\n\
                     可使用 --app-root <目录> 指定，或将 exe 放在应用目录中。\n已尝试：",
                );
                for (index, path) in tried.iter().enumerate() {
                    if index > 0 {
                        message.push_str("；");
                    }
                    message.push_str(&path.display().to_string());
                }
                message
            }
        }
    }
}
