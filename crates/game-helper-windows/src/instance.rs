//! Process-lifetime named mutex; manager and picker have separate identities.
use anyhow::{Result, bail};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
    },
    core::PCWSTR,
};

pub struct InstanceGuard(HANDLE);
impl InstanceGuard {
    pub fn acquire(root: &Path, picker: bool) -> Result<Self> {
        let mut hash = DefaultHasher::new();
        root.to_string_lossy().to_lowercase().hash(&mut hash);
        let name = format!(
            "Local\\GameHelper-Rust-{:016x}-{}",
            hash.finish(),
            if picker { "picker" } else { "manager" }
        );
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: Input name lives through the call. This guard uniquely owns
        // each successfully returned handle; no mutex ownership is requested.
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr()))? };
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if exists {
            unsafe {
                let _ = CloseHandle(handle);
            }
            bail!(
                "此目录的{}已在运行",
                if picker { "取点工具" } else { "管理器" }
            );
        }
        Ok(Self(handle))
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: The guard closes its owned handle once. No ReleaseMutex is
        // needed because CreateMutex was called with initial ownership false.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
