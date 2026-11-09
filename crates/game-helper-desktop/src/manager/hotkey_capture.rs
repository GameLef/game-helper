//! Keyboard capture owns a short-lived service lease and never changes route documents.
use super::*;
use game_helper_core::config::{Config, hotkey_display, hotkey_to_ahk};
use game_helper_windows::{hotkey, window as win};
use gpui_kit::{KeyDownEvent, Modifiers};

struct CaptureLease {
    client: DaemonClient,
    owner: isize,
}
impl Drop for CaptureLease {
    fn drop(&mut self) {
        let client = self.client.clone();
        let owner = self.owner;
        // Drop also runs when a window closes. Never block the UI on IPC;
        // the daemon's lease watchdog covers process exit and failed sends.
        let _ = std::thread::Builder::new()
            .name("hotkey-capture-release".into())
            .spawn(move || {
                let _ = client.capture_end(owner);
            });
    }
}

pub(super) struct HotkeyCapture {
    pub(super) stem: String,
    pub(super) candidate: Option<String>,
    pub(super) error: Option<String>,
    pub(super) saving: bool,
    pub(super) armed: bool,
    pub(super) modifiers: Modifiers,
    lease: Option<CaptureLease>,
}

struct ConfigChange {
    config: ConfigFile,
    state: Option<DaemonStatus>,
    error: Option<String>,
    committed: bool,
}

/// If a reload fails, restore the previous settings using the *new* disk
/// snapshot. A concurrent external editor is still protected by ConfigFile.
fn save_config_change(
    paths: AppPaths,
    mut config: ConfigFile,
    previous: Config,
) -> Result<ConfigChange> {
    let client = DaemonClient::new(paths);
    let state = client.status()?;
    ensure!(!state.busy, "执行服务正在回放，请空闲后修改快捷键");
    config.save()?;
    if state.running
        && let Err(error) = client.reload()
    {
        let desired = config.value.clone();
        config.value = previous;
        let recovery = match config.save() {
            Ok(()) => match client.reload() {
                Ok(()) => "已恢复原配置及服务注册".to_string(),
                Err(e) => format!("原配置已恢复，服务状态未知：{e:#}；请检查并重载服务"),
            },
            Err(e) => {
                config.value = desired;
                format!("恢复配置失败：{e:#}；请重新加载配置并检查服务")
            }
        };
        return Ok(ConfigChange {
            config,
            state: None,
            error: Some(format!("应用快捷键失败：{error:#}；{recovery}")),
            committed: false,
        });
    }
    // A status refresh failure must not turn an already committed save into a
    // discarded ConfigFile snapshot. Retain it and make uncertainty explicit.
    let (state, error) = match client.status() {
        Ok(state) => (Some(state), None),
        Err(e) => (None, Some(format!("配置已保存，服务状态未知：{e:#}"))),
    };
    Ok(ConfigChange {
        config,
        state,
        error,
        committed: true,
    })
}

impl Manager {
    pub(super) fn persist_hotkey_config(
        &mut self,
        config: ConfigFile,
        message: String,
        close_capture: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self
            .config
            .as_ref()
            .expect("config validated by caller")
            .value
            .clone();
        let paths = self.paths.clone();
        self.worker(
            window,
            cx,
            move || save_config_change(paths, config, previous),
            move |view, result, window, cx| match result {
                Ok(change) => {
                    view.config = Some(change.config);
                    if let Some(state) = change.state {
                        view.daemon = state;
                    }
                    view.daemon_error = change.error.clone();
                    if let Some(error) = change.error {
                        if let Some(capture) = view.capture.as_mut() {
                            capture.error = Some(error.clone());
                            capture.saving = false;
                            capture.candidate = None;
                        }
                        view.error(anyhow::anyhow!(error));
                    } else {
                        view.status = message.into();
                    }
                    if close_capture && change.committed {
                        view.finish_hotkey_capture(window, cx);
                    }
                }
                Err(error) => {
                    if let Some(capture) = view.capture.as_mut() {
                        capture.error = Some(format!("{error:#}"));
                        capture.saving = false;
                        capture.candidate = None;
                    }
                    view.error(error);
                }
            },
        );
    }

    fn hotkey_mutation_allowed(&self) -> Result<()> {
        ensure!(self.config.is_some(), "配置不可用");
        ensure!(
            !self.daemon.busy && self.daemon_error.is_none(),
            "执行服务忙碌或状态未知，请稍后重试"
        );
        Ok(())
    }

    pub(super) fn toggle_route_hotkey(
        &mut self,
        stem: &str,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.hotkey_mutation_allowed()?;
        let mut config = self.config.clone().context("配置不可用")?;
        config.value.set_binding_enabled(stem, enabled)?;
        let entry = self
            .routes
            .iter()
            .find(|entry| entry.stem == stem)
            .context("路线不存在")?;
        let paths = self.paths.clone();
        let path = entry.path.clone();
        let stem = stem.to_owned();
        let start_at = config
            .value
            .binding_for(&stem)
            .context("请先设置快捷键")?
            .start_at;
        let previous = self.config.as_ref().unwrap().value.clone();
        self.worker(
            window,
            cx,
            move || {
                if enabled {
                    let snapshot = DiskSnapshot::read(&path)?;
                    let document = RouteDocument::open(path.clone())?;
                    let (route, _) = document.source_for_save()?;
                    ensure!(
                        start_at > 0 && start_at <= route.points.len(),
                        "快捷键起始点超出路线点位，请先调整起始点"
                    );
                    ensure!(snapshot.unchanged(&path)?, "磁盘路线已变化，请重试");
                    storage::cache_route(&paths, &stem, &route)?;
                }
                save_config_change(paths, config, previous)
            },
            move |view, result, _, _| match result {
                Ok(change) => {
                    view.config = Some(change.config);
                    if let Some(state) = change.state {
                        view.daemon = state;
                    }
                    view.daemon_error = change.error.clone();
                    if let Some(error) = change.error {
                        view.error(anyhow::anyhow!(error));
                    } else {
                        view.status = if enabled {
                            "快捷键已开启，同键其他路线已关闭"
                        } else {
                            "快捷键已关闭"
                        }
                        .into();
                    }
                }
                Err(error) => view.error(error),
            },
        );
        Ok(())
    }

    pub(super) fn save_start_at(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.hotkey_mutation_allowed()?;
        self.flush(cx)?;
        let doc = self.document.as_ref().context("请选择路线")?;
        ensure!(!doc.dirty(), "请先保存路线，再调整起始点");
        let stem = doc.stem()?;
        let start = self
            .text("start", cx)
            .parse::<usize>()
            .context("起始点须为整数")?;
        ensure!(
            start > 0 && start <= doc.route().points.len(),
            "起始点超出路线点位"
        );
        let mut config = self.config.clone().context("配置不可用")?;
        let key = config
            .value
            .binding_for(&stem)
            .context("请先设置快捷键")?
            .hotkey
            .clone();
        config.value.set_binding(&stem, &key, start)?;
        self.persist_hotkey_config(config, "起始点已保存".into(), false, window, cx);
        Ok(())
    }

    pub(super) fn clear_hotkey(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.hotkey_mutation_allowed()?;
        let stem = self.document.as_ref().context("请选择路线")?.stem()?;
        let mut config = self.config.clone().context("配置不可用")?;
        config.value.set_binding(&stem, "", 1)?;
        self.persist_hotkey_config(config, "快捷键已清除".into(), false, window, cx);
        Ok(())
    }

    pub(super) fn begin_hotkey_capture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.hotkey_mutation_allowed()?;
        self.flush(cx)?;
        let doc = self.document.as_ref().context("请选择路线")?;
        ensure!(!doc.dirty(), "请先保存路线，再设置快捷键");
        ensure!(!doc.route().points.is_empty(), "请先添加并保存至少一个点位");
        let stem = doc.stem()?;
        let owner = win::foreground_window()
            .filter(|info| info.pid == std::process::id())
            .context("请先激活管理器窗口")?
            .hwnd;
        let client = DaemonClient::new(self.paths.clone());
        self.worker(
            window,
            cx,
            move || {
                let status = client.status()?;
                ensure!(!status.busy, "执行服务正在回放，请稍后设置快捷键");
                let lease = if status.running {
                    client.capture_begin(owner)?;
                    Some(CaptureLease { client, owner })
                } else {
                    None
                };
                Ok((stem, lease))
            },
            |view, result, window, cx| match result {
                Ok((stem, lease)) => {
                    if !window.is_window_active() {
                        return;
                    }
                    view.cancel_list_drag(cx);
                    view.capture = Some(HotkeyCapture {
                        stem,
                        candidate: None,
                        error: None,
                        saving: false,
                        armed: false,
                        modifiers: window.modifiers(),
                        lease,
                    });
                    view.capture_focus.focus(window, cx);
                    view.capture_task = Some(cx.spawn_in(window, async move |weak, cx| {
                        let mut last_renew = Instant::now();
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(40))
                                .await;
                            let tick = weak
                                .update_in(cx, |view, window, cx| {
                                    view.capture.as_ref()?;
                                    if !window.is_window_active() {
                                        view.cancel_hotkey_capture(window, cx);
                                        return None;
                                    }
                                    let capture = view.capture.as_mut().unwrap();
                                    capture.modifiers = window.modifiers();
                                    if !capture.saving && hotkey::keys_released() {
                                        if capture.candidate.is_some() {
                                            view.commit_captured_hotkey(window, cx);
                                        } else {
                                            capture.armed = true;
                                        }
                                    }
                                    cx.notify();
                                    Some(
                                        view.capture
                                            .as_ref()
                                            .and_then(|capture| capture.lease.as_ref())
                                            .map(|lease| (lease.client.clone(), lease.owner)),
                                    )
                                })
                                .ok()
                                .flatten();
                            let Some(lease) = tick else {
                                break;
                            };
                            if last_renew.elapsed() >= Duration::from_secs(1) {
                                last_renew = Instant::now();
                                if let Some((client, owner)) = lease {
                                    let result = cx
                                        .background_spawn(
                                            async move { client.capture_renew(owner) },
                                        )
                                        .await;
                                    if let Err(error) = result {
                                        let _ = weak.update_in(cx, |view, window, cx| {
                                            view.cancel_hotkey_capture(window, cx);
                                            view.error(error);
                                            cx.notify();
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                    }));
                }
                Err(error) => view.error(error),
            },
        );
        Ok(())
    }

    pub(super) fn finish_hotkey_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.capture.take();
        // The capture task exits after seeing None. Do not drop it from inside
        // its own callback while it is driving a save or cancellation.
        if window.is_window_active() {
            self.hotkey_button_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn cancel_hotkey_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.capture.as_ref().is_some_and(|capture| capture.saving) {
            return;
        }
        self.finish_hotkey_capture(window, cx);
        self.status = "已取消快捷键设置，原绑定保持不变".into();
    }

    pub(super) fn capture_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        window.prevent_default();
        cx.stop_propagation();
        if capture.saving || !capture.armed || event.is_held {
            return;
        }
        if event.keystroke.key == "escape" {
            self.cancel_hotkey_capture(window, cx);
            return;
        }
        if event.keystroke.key == "tab"
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
            && !event.keystroke.modifiers.platform
        {
            self.capture_focus.focus(window, cx);
            return;
        }
        if capture.candidate.is_some() {
            return;
        }
        let key = hotkey::pressed_key().unwrap_or_else(|| event.keystroke.key.clone());
        match capture_binding(&key, window.modifiers()) {
            Ok(Some(binding)) => {
                capture.candidate = Some(binding);
                capture.error = None;
            }
            Ok(None) => {}
            Err(error) => capture.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    fn commit_captured_hotkey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> Result<(ConfigFile, String)> {
            let capture = self.capture.as_ref().context("录制已结束")?;
            let key = capture.candidate.as_ref().context("尚未捕获按键")?;
            let mut config = self.config.clone().context("配置不可用")?;
            let start = config
                .value
                .binding_for(&capture.stem)
                .map(|binding| binding.start_at)
                .unwrap_or(1);
            config.value.set_binding(&capture.stem, key, start)?;
            let enabled = config
                .value
                .binding_for(&capture.stem)
                .is_some_and(|binding| binding.enabled);
            let message = format!(
                "快捷键已设置为 {}{}",
                hotkey_display(key),
                if enabled {
                    "，同键其他路线已关闭"
                } else {
                    "，开启路线开关后生效"
                }
            );
            Ok((config, message))
        })();
        match result {
            Ok((config, message)) => {
                self.capture.as_mut().unwrap().saving = true;
                self.persist_hotkey_config(config, message, true, window, cx);
            }
            Err(error) => {
                if let Some(capture) = self.capture.as_mut() {
                    capture.candidate = None;
                    capture.error = Some(format!("{error:#}"));
                }
            }
        }
    }
}

fn capture_binding(key: &str, modifiers: Modifiers) -> Result<Option<String>> {
    let key = match key.to_ascii_lowercase().as_str() {
        "control" | "ctrl" | "alt" | "shift" | "super" | "win" | "lwin" | "rwin" | "lshift"
        | "rshift" | "lcontrol" | "rcontrol" | "lalt" | "ralt" => return Ok(None),
        "escape" | "esc" => "Esc".to_string(),
        "enter" | "return" => "Enter".to_string(),
        "space" | " " => "Space".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Delete".to_string(),
        "insert" => "Insert".to_string(),
        "pageup" => "PgUp".to_string(),
        "pagedown" => "PgDn".to_string(),
        "up" | "down" | "left" | "right" | "home" | "end" | "tab" | "pause" | "printscreen"
        | "scrolllock" | "capslock" | "numlock" => key.to_owned(),
        value if value.len() == 1 && value.is_ascii() => value.to_string(),
        value
            if value.strip_prefix('f').is_some_and(|number| {
                number.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n))
            }) =>
        {
            value.to_string()
        }
        value if value.starts_with("vk") && u8::from_str_radix(&value[2..], 16).is_ok() => {
            value.to_string()
        }
        _ => anyhow::bail!("该按键暂不支持，请使用常用键或组合键"),
    };
    ensure!(!modifiers.function, "无法录制 Fn，请使用其他修饰键");
    ensure!(
        !(modifiers.alt && key.eq_ignore_ascii_case("f4")),
        "Alt+F4 用于关闭窗口，请选择其他快捷键"
    );
    let mut binding = String::new();
    for (active, prefix) in [
        (modifiers.control, '^'),
        (modifiers.alt, '!'),
        (modifiers.shift, '+'),
        (modifiers.platform, '#'),
    ] {
        if active {
            binding.push(prefix);
        }
    }
    binding.push_str(&key);
    Ok(Some(hotkey_to_ahk(&binding)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_combinations_and_existing_apostrophe() {
        assert_eq!(
            capture_binding("'", Modifiers::default())
                .unwrap()
                .as_deref(),
            Some("'")
        );
        assert_eq!(
            capture_binding(
                "f9",
                Modifiers {
                    control: true,
                    alt: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .as_deref(),
            Some("^!f9")
        );
        assert_eq!(
            capture_binding(
                "=",
                Modifiers {
                    shift: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .as_deref(),
            Some("+=")
        );
        assert_eq!(capture_binding("ctrl", Modifiers::default()).unwrap(), None);
        assert!(capture_binding("中", Modifiers::default()).is_err());
        assert!(
            capture_binding(
                "f4",
                Modifiers {
                    alt: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
