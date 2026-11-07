//! Independent native picker. RegisterHotKey is owned by this window only.
use anyhow::{Context as _, Result, ensure};
use game_helper_core::{
    route::{Point, Route, Target},
    storage::{self, AppPaths, DiskSnapshot},
};
use game_helper_windows::{hotkey::HotkeyService, window as win};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Task, Window, div, px,
};
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Captured {
    point: Point,
    name: Entity<InputState>,
}
struct PendingSave {
    path: PathBuf,
    snapshot: DiskSnapshot,
    text: String,
    route: Route,
    cache: bool,
}
pub struct Picker {
    paths: AppPaths,
    windows: Vec<win::WindowInfo>,
    target: Option<win::WindowInfo>,
    points: Vec<Captured>,
    stem: Entity<InputState>,
    name: Entity<InputState>,
    status: SharedString,
    coordinates: SharedString,
    hotkey: Option<HotkeyService>,
    selecting: Option<Instant>,
    capture_at: Option<Instant>,
    busy: bool,
    dirty: bool,
    close_requested: bool,
    pending: Option<PendingSave>,
    saved: Option<(PathBuf, DiskSnapshot)>,
    _subscriptions: Vec<Subscription>,
    point_subscriptions: Vec<Subscription>,
    job: Option<Task<()>>,
    poll: Option<Task<()>>,
}
impl Picker {
    pub fn new(paths: AppPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let stem = cx.new(|cx| InputState::new(window, cx).default_value("新采集路线"));
        let name = cx.new(|cx| InputState::new(window, cx).default_value("新采集路线"));
        let mut subscriptions = Vec::new();
        for field in [&stem, &name] {
            subscriptions.push(cx.subscribe(field, |v, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    v.dirty = true;
                    cx.notify();
                }
            }));
        }
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |w, cx| {
            weak.update(cx, |v, cx| {
                if v.busy || v.dirty {
                    v.close_requested = true;
                    v.status = "尚有未保存采点，可保存后关闭或放弃关闭".into();
                    cx.notify();
                    false
                } else {
                    let _ = w;
                    true
                }
            })
            .unwrap_or(true)
        });
        let mut view = Self {
            paths,
            windows: Vec::new(),
            target: None,
            points: Vec::new(),
            stem,
            name,
            status: "选择目标窗口后启用采点".into(),
            coordinates: "未选择目标窗口".into(),
            hotkey: None,
            selecting: None,
            capture_at: None,
            busy: false,
            dirty: false,
            close_requested: false,
            pending: None,
            saved: None,
            _subscriptions: subscriptions,
            point_subscriptions: Vec::new(),
            job: None,
            poll: None,
        };
        view.refresh(window, cx);
        view.poll = Some(cx.spawn_in(window, async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let Ok((target, capture, selecting, locked)) = weak.update_in(cx, |v, _, _| {
                    let now = Instant::now();
                    let mut events = 0;
                    if let Some(service) = &v.hotkey {
                        while service.try_recv().is_ok() {
                            events += 1;
                        }
                    }
                    if v.capture_at.is_some_and(|at| now >= at) {
                        v.capture_at = None;
                        events += 1;
                    }
                    let selecting = v.selecting.is_some_and(|at| now >= at);
                    if selecting {
                        v.selecting = None;
                    }
                    (
                        v.target.clone(),
                        events,
                        selecting,
                        v.busy || v.pending.is_some(),
                    )
                }) else {
                    break;
                };
                let result = cx
                    .background_spawn(async move {
                        let position = win::cursor_position();
                        let picked = if selecting {
                            position.and_then(|(x, y)| win::window_from_point(x, y))
                        } else {
                            None
                        };
                        let sample = sample(target.as_ref(), position);
                        (picked, sample)
                    })
                    .await;
                if weak
                    .update_in(cx, |v, w, cx| {
                        if let Some(target) = result.0 {
                            v.choose(target, cx);
                        }
                        v.coordinates = match &result.1 {
                            Ok((screen, relative, pos)) => format!(
                                "屏幕 ({}, {}) · 客户区 ({}, {}) · 归一化 ({:.6}, {:.6})",
                                screen.0, screen.1, relative.0, relative.1, pos[0], pos[1]
                            )
                            .into(),
                            Err(e) => format!("不可采点：{e:#}").into(),
                        };
                        if capture > 0 && !locked {
                            match result.1 {
                                Ok((_, _, pos)) => {
                                    for _ in 0..capture {
                                        v.append(pos, w, cx);
                                    }
                                }
                                Err(e) => v.status = format!("采点失败：{e:#}").into(),
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        view
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.job = Some(cx.spawn_in(window, async |weak, cx| {
            let result = cx.background_spawn(async { win::list_windows() }).await;
            let _ = weak.update_in(cx, |v, _, cx| {
                v.busy = false;
                match result {
                    Ok(windows) => v.windows = windows,
                    Err(e) => v.status = format!("窗口枚举失败：{e}").into(),
                }
                cx.notify();
            });
        }));
    }
    fn choose(&mut self, target: win::WindowInfo, cx: &mut Context<Self>) {
        if target.pid == std::process::id() {
            self.status = "请选择其他应用的窗口".into();
            return;
        }
        if !self.points.is_empty()
            && self
                .target
                .as_ref()
                .is_some_and(|old| old.hwnd != target.hwnd || old.pid != target.pid)
        {
            self.status = "当前点位属于原目标，请保存或清空点位后更换目标".into();
            return;
        }
        self.status = format!("目标：{} · {}", target.process, target.title).into();
        self.target = Some(target);
        self.dirty = true;
        cx.notify();
    }
    fn append(&mut self, pos: [f64; 2], window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .points
            .iter()
            .map(|p| p.point.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let point = Point::new(id, pos);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(point.name.clone()));
        self.point_subscriptions
            .push(cx.subscribe(&name, |v, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    v.dirty = true;
                    cx.notify();
                }
            }));
        self.points.push(Captured { point, name });
        self.dirty = true;
        self.status = format!("已采集 {} 个点位", self.points.len()).into();
    }
    fn toggle_hotkey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hotkey.take().is_some() {
            self.status = "全局采点热键已释放".into();
            return;
        }
        if self.busy {
            return;
        }
        self.busy = true;
        self.job = Some(cx.spawn_in(window, async |weak, cx| {
            let result = cx.background_spawn(async { HotkeyService::start() }).await;
            let _ = weak.update_in(cx, |v, _, cx| {
                v.busy = false;
                match result {
                    Ok(service) => {
                        v.hotkey = Some(service);
                        v.status = "Ctrl+Alt+P 已启用，失焦也可采点".into();
                    }
                    Err(e) => v.status = format!("热键注册失败：{e}；可使用延时采点按钮").into(),
                }
                cx.notify();
            });
        }));
    }
    fn route(&self, cx: &Context<Self>) -> Result<Route> {
        let target = self.target.as_ref().context("请先选择目标窗口")?;
        ensure!(target.is_alive(), "目标窗口已失效，请重新选择");
        let mut route = Route::draft(self.name.read(cx).value().to_string());
        route.target = Target {
            process: target.process.clone(),
            window_class: target.class.clone(),
            title: target.title.clone(),
        };
        route.points = self
            .points
            .iter()
            .map(|row| {
                let mut p = row.point.clone();
                p.name = row.name.read(cx).value().to_string();
                p
            })
            .collect();
        route.validate()?;
        Ok(route)
    }
    fn save(&mut self, export: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let route = match self.route(cx) {
            Ok(r) => r,
            Err(e) => {
                self.status = format!("{e:#}").into();
                return;
            }
        };
        let text = match route.yaml() {
            Ok(s) => s,
            Err(e) => {
                self.status = format!("{e:#}").into();
                return;
            }
        };
        let paths = self.paths.clone();
        let stem = self.stem.read(cx).value().to_string();
        let saved = self.saved.clone();
        let receiver = if export {
            Some(cx.prompt_for_new_path(&paths.routes(), Some("采集路线.yaml")))
        } else {
            None
        };
        self.busy = true;
        self.job = Some(cx.spawn_in(window, async move |weak, cx| {
            let path = if let Some(receiver) = receiver {
                match receiver.await {
                    Ok(Ok(Some(p))) => Ok(Some(p)),
                    Ok(Ok(None)) => Ok(None),
                    _ => Err(anyhow::anyhow!("导出对话框失败")),
                }
            } else {
                paths.route_path(&stem).map(Some)
            };
            let result = match path {
                Ok(Some(path)) => Some(
                    cx.background_spawn(async move {
                        let snapshot = if let Some((old_path, old_snapshot)) =
                            saved.filter(|(p, _)| p == &path)
                        {
                            ensure!(
                                old_snapshot.unchanged(&old_path)?,
                                "目标文件已被外部修改，请另存"
                            );
                            old_snapshot
                        } else {
                            DiskSnapshot::read(&path)?
                        };
                        let exists = snapshot.0.is_some();
                        Ok::<_, anyhow::Error>((
                            PendingSave {
                                path,
                                snapshot,
                                text,
                                route,
                                cache: !export,
                            },
                            exists,
                        ))
                    })
                    .await,
                ),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            };
            let _ = weak.update_in(cx, |v, w, cx| {
                v.busy = false;
                match result {
                    Some(Ok((pending, exists))) => {
                        let known = v.saved.as_ref().is_some_and(|(p, _)| p == &pending.path);
                        v.pending = Some(pending);
                        if exists && !known {
                            v.status =
                                "目标已存在：确认覆盖会备份原文，或取消选择其他文件名".into();
                        } else {
                            v.commit(w, cx);
                        }
                    }
                    Some(Err(e)) => v.status = format!("{e:#}").into(),
                    None => {}
                }
                cx.notify();
            });
        }));
    }
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.busy = true;
        let paths = self.paths.clone();
        self.job = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = cx
                .background_spawn(async move {
                    let snapshot = storage::save_atomic(
                        &pending.path,
                        &pending.snapshot,
                        pending.text.as_bytes(),
                        true,
                    )?;
                    let cache_error = if pending.cache {
                        pending
                            .path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .and_then(|stem| {
                                storage::cache_route(&paths, stem, &pending.route).err()
                            })
                            .map(|e| format!("源码已保存；缓存更新失败：{e:#}"))
                    } else {
                        None
                    };
                    Ok::<_, anyhow::Error>((pending.path, snapshot, cache_error))
                })
                .await;
            let _ = weak.update_in(cx, |v, _, cx| {
                v.busy = false;
                match result {
                    Ok((path, snapshot, note)) => {
                        v.saved = Some((path.clone(), snapshot));
                        v.dirty = false;
                        v.status = note
                            .unwrap_or_else(|| format!("已保存：{}", path.display()))
                            .into();
                    }
                    Err(e) => v.status = format!("{e:#}").into(),
                }
                cx.notify();
            });
        }));
    }
}
type Sample = ((i32, i32), (i32, i32), [f64; 2]);
fn sample(target: Option<&win::WindowInfo>, screen: Option<(i32, i32)>) -> Result<Sample> {
    let target = target.context("未选择目标窗口")?;
    let (x, y) = screen.context("无法读取光标位置")?;
    let area = target
        .client_area()
        .context("目标失效、最小化或客户区不可用")?;
    let (nx, ny) = area
        .normalize_screen(x, y)
        .context("光标在目标客户区外或客户区太小")?;
    Ok(((x, y), (x - area.screen_x, y - area.screen_y), [nx, ny]))
}
impl Render for Picker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut targets = div()
            .id("picker-targets")
            .h(px(180.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for (i, target) in self.windows.iter().enumerate() {
            let target = target.clone();
            targets = targets.child(
                Button::new(("target", i))
                    .label(format!("{} · {}", target.process, target.title))
                    .disabled(self.busy || self.pending.is_some())
                    .on_click(cx.listener(move |v, _, _, cx| v.choose(target.clone(), cx))),
            );
        }
        let mut points = div()
            .id("picker-points")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for (i, row) in self.points.iter().enumerate() {
            points = points.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(format!(
                        "{} · ({:.6}, {:.6})",
                        i + 1,
                        row.point.pos[0],
                        row.point.pos[1]
                    ))
                    .child(
                        Input::new(&row.name)
                            .disabled(self.busy || self.pending.is_some())
                            .w(px(300.)),
                    )
                    .child(Button::new(("copy", i)).label("复制").on_click(cx.listener(
                        move |v, _, _, cx| {
                            if let Some(row) = v.points.get(i) {
                                let text = format!(
                                    "name: {}\npos: [{:.6}, {:.6}]",
                                    row.name.read(cx).value(),
                                    row.point.pos[0],
                                    row.point.pos[1]
                                );
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        },
                    )))
                    .child(
                        Button::new(("remove", i))
                            .label("删除")
                            .disabled(self.busy || self.pending.is_some())
                            .on_click(cx.listener(move |v, _, _, cx| {
                                if i < v.points.len() {
                                    v.points.remove(i);
                                    drop(v.point_subscriptions.remove(i));
                                    v.dirty = true;
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        let target = self
            .target
            .as_ref()
            .map(|t| format!("{} · {} · HWND {}", t.process, t.title, t.hwnd))
            .unwrap_or_else(|| "未选择".into());
        let mut root = div()
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("Game Helper · 取点工具")
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("refresh-windows")
                            .label("刷新窗口")
                            .disabled(self.busy || self.pending.is_some())
                            .on_click(cx.listener(|v, _, w, cx| v.refresh(w, cx))),
                    )
                    .child(
                        Button::new("crosshair")
                            .label("准星选择（3秒后取光标下窗口）")
                            .disabled(self.busy || self.pending.is_some())
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.selecting = Some(Instant::now() + Duration::from_secs(3));
                                v.status = "将光标移到目标窗口，3秒后选择".into();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("hotkey")
                            .label(if self.hotkey.is_some() {
                                "释放 Ctrl+Alt+P"
                            } else {
                                "启用 Ctrl+Alt+P"
                            })
                            .disabled(self.busy || self.pending.is_some())
                            .on_click(cx.listener(|v, _, w, cx| v.toggle_hotkey(w, cx))),
                    )
                    .child(
                        Button::new("capture")
                            .label("2秒后采点")
                            .disabled(self.busy || self.target.is_none())
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.capture_at = Some(Instant::now() + Duration::from_secs(2));
                                v.status = "将光标移到目标客户区，2秒后采点".into();
                                cx.notify();
                            })),
                    ),
            )
            .child(targets)
            .child(format!("已选目标：{target}"))
            .child(
                div()
                    .id("coordinates")
                    .role(gpui_kit::Role::Status)
                    .accessibility_id("coordinates")
                    .aria_label(self.coordinates.clone())
                    .child(self.coordinates.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child("文件名")
                    .child(Input::new(&self.stem).disabled(self.busy || self.pending.is_some()))
                    .child("路线名")
                    .child(Input::new(&self.name).disabled(self.busy || self.pending.is_some())),
            )
            .child(points)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("save-picker")
                            .label("保存路线")
                            .disabled(self.busy || self.pending.is_some())
                            .on_click(cx.listener(|v, _, w, cx| v.save(false, w, cx))),
                    )
                    .child(
                        Button::new("export-picker")
                            .label("导出…")
                            .disabled(self.busy || self.pending.is_some())
                            .on_click(cx.listener(|v, _, w, cx| v.save(true, w, cx))),
                    )
                    .child(
                        Button::new("clear-picker")
                            .label("清空采点")
                            .disabled(self.busy || self.pending.is_some())
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.points.clear();
                                v.point_subscriptions.clear();
                                v.dirty = true;
                                cx.notify();
                            })),
                    ),
            );
        if self.pending.is_some() {
            root =
                root.child(
                    div()
                        .flex()
                        .gap_2()
                        .child("覆盖将保留备份")
                        .child(
                            Button::new("confirm-overwrite")
                                .label("确认覆盖")
                                .on_click(cx.listener(|v, _, w, cx| v.commit(w, cx))),
                        )
                        .child(Button::new("cancel-overwrite").label("取消").on_click(
                            cx.listener(|v, _, _, cx| {
                                v.pending = None;
                                cx.notify();
                            }),
                        )),
                );
        }
        if self.close_requested {
            root =
                root.child(
                    div()
                        .flex()
                        .gap_2()
                        .child("采点尚未保存")
                        .child(
                            Button::new("discard-close")
                                .label("放弃并关闭")
                                .disabled(self.busy || self.pending.is_some())
                                .on_click(cx.listener(|_, _, w, _| w.remove_window())),
                        )
                        .child(Button::new("cancel-close").label("继续编辑").on_click(
                            cx.listener(|v, _, _, cx| {
                                v.close_requested = false;
                                cx.notify();
                            }),
                        )),
                );
        }
        root.child(
            div()
                .id("picker-status")
                .role(gpui_kit::Role::Status)
                .accessibility_id("picker-status")
                .aria_label(self.status.clone())
                .child(self.status.clone()),
        )
    }
}
