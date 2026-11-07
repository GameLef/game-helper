//! Independent native picker. RegisterHotKey is owned by this window only.
mod view;

use anyhow::{Context as _, Result, ensure};
use game_helper_core::{
    route::{OpKind, Operation, Point, Route, Target},
    storage::{self, AppPaths, DiskSnapshot},
};
use game_helper_windows::{hotkey::HotkeyService, window as win};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, FocusHandle, IntoElement, Render, ScrollHandle,
    SharedString, Subscription, Task, Window,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

gpui_kit::actions!(picker, [ModalNext, ModalPrevious]);

struct Captured {
    point: Point,
    position: [f64; 2],
    name: Entity<InputState>,
}
struct PendingSave {
    path: PathBuf,
    snapshot: DiskSnapshot,
    text: String,
    route: Route,
    cache: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FeedbackKind {
    Info,
    Success,
    Error,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirmation {
    Overwrite,
    Clear,
    Close,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CountdownKind {
    Select,
    Capture,
}
#[derive(Clone, Copy, Debug)]
struct Countdown {
    kind: CountdownKind,
    deadline: Instant,
}
impl Countdown {
    fn remaining(self, now: Instant) -> u64 {
        self.deadline
            .saturating_duration_since(now)
            .as_millis()
            .div_ceil(1000) as u64
    }
}

type Sample = ((i32, i32), (i32, i32), [f64; 2]);
enum SampleState {
    Unselected,
    Unavailable(SharedString),
    Valid(Sample),
}

pub struct Picker {
    paths: AppPaths,
    windows: Vec<win::WindowInfo>,
    target: Option<win::WindowInfo>,
    points: Vec<Captured>,
    stem: Entity<InputState>,
    name: Entity<InputState>,
    search: Entity<InputState>,
    status: SharedString,
    status_kind: FeedbackKind,
    coordinates: SharedString,
    sample_state: SampleState,
    hotkey: Option<HotkeyService>,
    countdown: Option<Countdown>,
    // Incremented whenever an interaction invalidates an in-flight sample/command.
    interaction_epoch: u64,
    busy: bool,
    dirty: bool,
    confirmation: Option<Confirmation>,
    close_after_save: bool,
    pending: Option<PendingSave>,
    saved: Option<(PathBuf, DiskSnapshot)>,
    library_open: Option<bool>,
    point_scroll: ScrollHandle,
    confirm_focus: FocusHandle,
    confirm_primary_focus: FocusHandle,
    confirm_discard_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
    point_subscriptions: Vec<Subscription>,
    job: Option<Task<()>>,
    poll: Option<Task<()>>,
}
impl Picker {
    pub fn new(paths: AppPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            gpui_kit::KeyBinding::new("tab", ModalNext, Some("PickerConfirmation")),
            gpui_kit::KeyBinding::new("shift-tab", ModalPrevious, Some("PickerConfirmation")),
        ]);
        let stem = cx.new(|cx| InputState::new(window, cx).default_value("新采集路线"));
        let name = cx.new(|cx| InputState::new(window, cx).default_value("新采集路线"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索标题或进程"));
        let mut subscriptions = Vec::new();
        for field in [&stem, &name] {
            subscriptions.push(cx.subscribe(field, |v, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    v.dirty = true;
                    cx.notify();
                }
            }));
        }
        subscriptions.push(cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()));
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |w, cx| {
            weak.update(cx, |v, cx| {
                if v.busy {
                    v.feedback(FeedbackKind::Info, "正在处理，请完成后再关闭");
                    cx.notify();
                    false
                } else if v.confirmation.is_some() {
                    v.confirm_focus.focus(w, cx);
                    false
                } else if v.dirty {
                    v.begin_confirmation(Confirmation::Close, w, cx);
                    false
                } else {
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
            search,
            status: "选择目标窗口，然后开始采点".into(),
            status_kind: FeedbackKind::Info,
            coordinates: "未选择目标窗口".into(),
            sample_state: SampleState::Unselected,
            hotkey: None,
            countdown: None,
            interaction_epoch: 0,
            busy: false,
            dirty: false,
            confirmation: None,
            close_after_save: false,
            pending: None,
            saved: None,
            library_open: None,
            point_scroll: ScrollHandle::new(),
            confirm_focus: cx.focus_handle(),
            confirm_primary_focus: cx.focus_handle(),
            confirm_discard_focus: cx.focus_handle(),
            return_focus: None,
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
                let Ok((target, capture, selecting, epoch, countdown_due)) =
                    weak.update_in(cx, |v, _, _| {
                        let mut events = 0;
                        // Always drain the queue, including while confirmation/save is active.
                        if let Some(service) = &v.hotkey {
                            while service.try_recv().is_ok() {
                                events += 1;
                            }
                        }
                        // The poll awaits this one background sample before dispatching
                        // another. Keep the timer visible/cancellable until its result
                        // commits, including the interval after the deadline.
                        let due = v
                            .countdown
                            .filter(|c| Instant::now() >= c.deadline)
                            .map(|c| c.kind);
                        let selecting = due == Some(CountdownKind::Select);
                        if due == Some(CountdownKind::Capture) {
                            events += 1;
                        }
                        if v.locked()
                            || selecting
                            || v.countdown.is_some_and(|c| c.kind == CountdownKind::Select)
                        {
                            events = 0;
                        }
                        (
                            v.target.clone(),
                            events,
                            selecting && !v.locked(),
                            v.interaction_epoch,
                            due.is_some(),
                        )
                    })
                else {
                    break;
                };
                let expected_target = target_key(target.as_ref());
                let result = cx
                    .background_spawn(async move {
                        let position = win::cursor_position();
                        let picked = if selecting {
                            position.and_then(|(x, y)| win::window_from_point(x, y))
                        } else {
                            None
                        };
                        (picked, sample(target.as_ref(), position))
                    })
                    .await;
                if weak
                    .update_in(cx, |v, w, cx| {
                        // A cancelled countdown, dialog, save or new target must invalidate
                        // events that were already dispatched to the background executor.
                        if !accept_sample(
                            epoch,
                            v.interaction_epoch,
                            expected_target,
                            target_key(v.target.as_ref()),
                        ) {
                            return;
                        }
                        if countdown_due {
                            take_due(&mut v.countdown, Instant::now());
                        }
                        if selecting && !v.locked() {
                            if let Some(target) = result.0 {
                                v.choose(target, cx);
                            } else {
                                v.feedback(FeedbackKind::Error, "未找到可用窗口，请重试准星选窗");
                            }
                            cx.notify();
                            return;
                        }
                        v.coordinates = match &result.1 {
                            Ok((screen, relative, pos)) => format!(
                                "屏幕 ({}, {}) · 客户区 ({}, {}) · 归一化 ({:.6}, {:.6})",
                                screen.0, screen.1, relative.0, relative.1, pos[0], pos[1]
                            )
                            .into(),
                            Err(e) => format!("不可采点：{e:#}").into(),
                        };
                        if capture > 0 && !v.locked() {
                            match &result.1 {
                                Ok((_, _, pos)) => {
                                    for _ in 0..capture {
                                        v.append(*pos, w, cx);
                                    }
                                }
                                Err(e) => {
                                    v.feedback(FeedbackKind::Error, format!("采点失败：{e:#}"))
                                }
                            }
                        }
                        v.sample_state = if v.target.is_none() {
                            SampleState::Unselected
                        } else {
                            match result.1 {
                                Ok(sample) => SampleState::Valid(sample),
                                Err(e) => SampleState::Unavailable(format!("{e:#}").into()),
                            }
                        };
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

    fn feedback(&mut self, kind: FeedbackKind, message: impl Into<SharedString>) {
        self.status_kind = kind;
        self.status = message.into();
    }
    fn locked(&self) -> bool {
        self.busy || self.confirmation.is_some() || self.pending.is_some()
    }
    fn invalidate_commands(&mut self) {
        self.countdown = None;
        self.interaction_epoch = self.interaction_epoch.wrapping_add(1);
        // Discard commands queued before a lock began/ended, even if the next
        // poll happens only after a very quick operation has already completed.
        if let Some(service) = &self.hotkey {
            while service.try_recv().is_ok() {}
        }
    }
    fn start_countdown(&mut self, kind: CountdownKind, cx: &mut Context<Self>) {
        if self.locked()
            || self.countdown.is_some()
            || (kind == CountdownKind::Capture && self.target.is_none())
        {
            return;
        }
        self.invalidate_commands();
        let seconds = if kind == CountdownKind::Select { 3 } else { 2 };
        self.countdown = Some(Countdown {
            kind,
            deadline: Instant::now() + Duration::from_secs(seconds),
        });
        self.feedback(
            FeedbackKind::Info,
            if kind == CountdownKind::Select {
                "将光标移到目标窗口，3 秒后选择；可取消"
            } else {
                "将光标移到目标客户区，2 秒后采点；可取消"
            },
        );
        cx.notify();
    }
    fn cancel_countdown(&mut self, cx: &mut Context<Self>) {
        // Also invalidate a due command whose background sampling has started.
        self.invalidate_commands();
        self.feedback(FeedbackKind::Info, "已取消倒计时");
        cx.notify();
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.invalidate_commands();
        self.busy = true;
        self.feedback(FeedbackKind::Info, "正在刷新窗口列表…");
        self.job = Some(cx.spawn_in(window, async |weak, cx| {
            let result = cx.background_spawn(async { win::list_windows() }).await;
            let _ = weak.update_in(cx, |v, _, cx| {
                v.invalidate_commands();
                v.busy = false;
                match result {
                    Ok(windows) => {
                        v.windows = windows
                            .into_iter()
                            .filter(|w| w.pid != std::process::id())
                            .collect();
                        v.feedback(
                            FeedbackKind::Info,
                            format!(
                                "找到 {} 个窗口，可从列表选择或使用准星选窗",
                                v.windows.len()
                            ),
                        );
                    }
                    Err(e) => v.feedback(FeedbackKind::Error, format!("窗口枚举失败：{e}")),
                }
                cx.notify();
            });
        }));
    }
    fn choose(&mut self, target: win::WindowInfo, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        if target.pid == std::process::id() {
            self.feedback(FeedbackKind::Error, "请选择其他应用的窗口");
            cx.notify();
            return;
        }
        let changed = target_key(self.target.as_ref()) != target_key(Some(&target));
        if changed && !self.points.is_empty() {
            self.feedback(
                FeedbackKind::Error,
                "已有点位属于当前目标。请先保存需要的点位，再清空后更换目标",
            );
            cx.notify();
            return;
        }
        if !target.is_alive() {
            self.feedback(FeedbackKind::Error, "目标窗口已关闭，请刷新后重新选择");
            cx.notify();
            return;
        }
        self.invalidate_commands();
        self.feedback(
            FeedbackKind::Success,
            format!("已选择 {}，将光标移入目标客户区开始采点", target.title),
        );
        self.target = Some(target);
        self.sample_state = SampleState::Unavailable("将光标移入目标客户区".into());
        self.coordinates = "将光标移入目标客户区".into();
        self.dirty |= changed;
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
        let mut point = Point::new(id);
        point.operations.push(Operation::new(OpKind::Click, pos));
        let name = cx.new(|cx| InputState::new(window, cx).default_value(point.name.clone()));
        self.point_subscriptions
            .push(cx.subscribe(&name, |v, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    v.dirty = true;
                    cx.notify();
                }
            }));
        self.points.push(Captured {
            point,
            position: pos,
            name,
        });
        self.point_scroll.scroll_to_bottom();
        self.dirty = true;
        self.feedback(
            FeedbackKind::Success,
            format!("已采集 {} 个点位", self.points.len()),
        );
    }
    fn copy_point(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        if let Some(row) = self.points.get(index) {
            let text = format!(
                "id: {}\nname: {}\noperations:\n  - type: click\n    pos: [{:.6}, {:.6}]",
                row.point.id,
                serde_json::json!(row.name.read(cx).value().to_string()),
                row.position[0],
                row.position[1]
            );
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.feedback(
                FeedbackKind::Success,
                format!("已复制第 {} 个点位", index + 1),
            );
            cx.notify();
        }
    }
    fn remove_point(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.locked() || index >= self.points.len() {
            return;
        }
        self.points.remove(index);
        drop(self.point_subscriptions.remove(index));
        self.dirty = true;
        self.feedback(
            FeedbackKind::Info,
            format!("已删除点位，剩余 {} 个", self.points.len()),
        );
        cx.notify();
    }
    fn request_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.locked() && !self.points.is_empty() {
            self.begin_confirmation(Confirmation::Clear, window, cx);
        }
    }
    fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.confirmation != Some(Confirmation::Clear) {
            return;
        }
        self.points.clear();
        self.point_subscriptions.clear();
        self.dirty = true;
        self.confirmation = None;
        self.invalidate_commands();
        self.restore_focus(window, cx);
        self.feedback(FeedbackKind::Info, "已清空采点，可以继续采点或更换目标");
        cx.notify();
    }
    fn begin_confirmation(
        &mut self,
        kind: Confirmation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.confirmation.is_none() {
            self.return_focus = window.focused(cx);
        }
        self.invalidate_commands();
        self.confirmation = Some(kind);
        self.confirm_focus.focus(window, cx);
        cx.notify();
    }
    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.return_focus.take() {
            focus.focus(window, cx);
        }
    }
    fn cancel_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.confirmation.is_none() {
            return;
        }
        self.confirmation = None;
        self.pending = None;
        self.close_after_save = false;
        self.invalidate_commands();
        self.restore_focus(window, cx);
        self.feedback(FeedbackKind::Info, "已取消操作，编辑内容已保留");
        cx.notify();
    }
    fn cycle_confirmation(&self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut handles = vec![&self.confirm_primary_focus];
        if self.confirmation == Some(Confirmation::Close) {
            handles.push(&self.confirm_discard_focus);
        }
        handles.push(&self.confirm_focus);
        let current = handles.iter().position(|h| h.is_focused(window));
        let next = match (current, backwards) {
            (Some(i), true) => (i + handles.len() - 1) % handles.len(),
            (Some(i), false) => (i + 1) % handles.len(),
            (None, _) => handles.len() - 1,
        };
        handles[next].focus(window, cx);
    }
    fn save_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.confirmation != Some(Confirmation::Close) {
            return;
        }
        self.confirmation = None;
        self.restore_focus(window, cx);
        self.close_after_save = true;
        self.save(false, window, cx);
    }
    fn discard_and_close(&mut self, window: &mut Window, _: &mut Context<Self>) {
        if !self.busy && self.confirmation == Some(Confirmation::Close) {
            self.invalidate_commands();
            window.remove_window();
        }
    }
    fn toggle_hotkey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.invalidate_commands();
        if self.hotkey.take().is_some() {
            self.feedback(FeedbackKind::Info, "全局采点热键已释放");
            cx.notify();
            return;
        }
        self.busy = true;
        self.feedback(FeedbackKind::Info, "正在注册 Ctrl+Alt+P…");
        self.job = Some(cx.spawn_in(window, async |weak, cx| {
            let result = cx.background_spawn(async { HotkeyService::start() }).await;
            let _ = weak.update_in(cx, |v, _, cx| {
                v.invalidate_commands();
                v.busy = false;
                match result {
                    Ok(service) => {
                        v.hotkey = Some(service);
                        v.invalidate_commands();
                        v.feedback(FeedbackKind::Success, "Ctrl+Alt+P 已启用，失焦也可采点");
                    }
                    Err(e) => v.feedback(
                        FeedbackKind::Error,
                        format!("热键注册失败：{e}；可使用延时采点按钮"),
                    ),
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
        if self.locked() {
            return;
        }
        self.invalidate_commands();
        let prepared = self
            .route(cx)
            .and_then(|route| route.yaml().map(|text| (route, text)));
        let (route, text) = match prepared {
            Ok(prepared) => prepared,
            Err(e) => {
                self.close_after_save = false;
                self.feedback(FeedbackKind::Error, format!("保存失败：{e:#}"));
                cx.notify();
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
        self.feedback(FeedbackKind::Info, "正在保存路线…");
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
                                "目标文件已被外部修改，请更换文件名或导出到其他文件"
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
                v.invalidate_commands();
                v.busy = false;
                match result {
                    Some(Ok((pending, exists))) => {
                        let known = v.saved.as_ref().is_some_and(|(p, _)| p == &pending.path);
                        v.pending = Some(pending);
                        if exists && !known {
                            v.begin_confirmation(Confirmation::Overwrite, w, cx);
                        } else {
                            v.commit(w, cx);
                        }
                    }
                    Some(Err(e)) => {
                        v.close_after_save = false;
                        v.feedback(FeedbackKind::Error, format!("保存失败：{e:#}"));
                    }
                    None => {
                        v.close_after_save = false;
                        v.feedback(FeedbackKind::Info, "已取消导出，编辑内容已保留");
                    }
                }
                cx.notify();
            });
        }));
    }
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.confirmation = None;
        self.restore_focus(window, cx);
        self.invalidate_commands();
        self.busy = true;
        self.feedback(FeedbackKind::Info, "正在写入路线与执行缓存…");
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
                            .map(|e| {
                                format!("路线已保存；执行缓存更新失败：{e:#}。请再次保存以重试")
                            })
                    } else {
                        None
                    };
                    Ok::<_, anyhow::Error>((pending.path, snapshot, cache_error))
                })
                .await;
            let _ = weak.update_in(cx, |v, w, cx| {
                v.invalidate_commands();
                v.busy = false;
                let close_requested = std::mem::take(&mut v.close_after_save);
                match result {
                    Ok((path, snapshot, note)) => {
                        v.saved = Some((path.clone(), snapshot));
                        v.dirty = note.is_some();
                        let close = should_close_after_save(close_requested, true, note.is_none());
                        if let Some(note) = note {
                            v.feedback(FeedbackKind::Error, note);
                        } else {
                            v.feedback(
                                FeedbackKind::Success,
                                format!("已保存：{}", path.display()),
                            );
                        }
                        if close {
                            w.remove_window();
                        }
                    }
                    Err(e) => v.feedback(FeedbackKind::Error, format!("保存失败：{e:#}")),
                }
                cx.notify();
            });
        }));
    }
}
fn target_key(target: Option<&win::WindowInfo>) -> Option<(isize, u32)> {
    target.map(|t| (t.hwnd, t.pid))
}
fn accept_sample(
    expected_epoch: u64,
    current_epoch: u64,
    expected_target: Option<(isize, u32)>,
    current_target: Option<(isize, u32)>,
) -> bool {
    expected_epoch == current_epoch && expected_target == current_target
}
fn take_due(countdown: &mut Option<Countdown>, now: Instant) -> Option<CountdownKind> {
    if countdown.is_some_and(|c| now >= c.deadline) {
        countdown.take().map(|c| c.kind)
    } else {
        None
    }
}
fn should_close_after_save(requested: bool, source_saved: bool, cache_saved: bool) -> bool {
    requested && source_saved && cache_saved
}
fn sample(target: Option<&win::WindowInfo>, screen: Option<(i32, i32)>) -> Result<Sample> {
    let target = target.context("未选择目标窗口")?;
    let (x, y) = screen.context("无法读取光标位置")?;
    let area = target
        .client_area()
        .context("目标已失效、最小化或客户区不可用，请还原窗口或重新选择")?;
    let (nx, ny) = area
        .normalize_screen(x, y)
        .context("光标在目标客户区外，请移入目标窗口")?;
    Ok(((x, y), (x - area.screen_x, y - area.screen_y), [nx, ny]))
}
impl Render for Picker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_workspace(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_runs_once_and_cancellation_removes_due_event() {
        let now = Instant::now();
        let timer = Countdown {
            kind: CountdownKind::Capture,
            deadline: now + Duration::from_secs(2),
        };
        assert_eq!(timer.remaining(now), 2);
        assert_eq!(timer.remaining(now + Duration::from_millis(1001)), 1);
        let mut pending = Some(timer);
        assert_eq!(
            take_due(&mut pending, now + Duration::from_millis(1999)),
            None
        );
        pending = None; // Cancel just before expiry.
        assert_eq!(take_due(&mut pending, now + Duration::from_secs(3)), None);
        pending = Some(timer);
        assert_eq!(
            take_due(&mut pending, timer.deadline),
            Some(CountdownKind::Capture)
        );
        assert_eq!(take_due(&mut pending, timer.deadline), None);
    }
    #[test]
    fn in_flight_sample_is_rejected_after_cancel_dialog_or_target_change() {
        let original = Some((101, 12));
        assert!(accept_sample(4, 4, original, original));
        assert!(!accept_sample(4, 5, original, original));
        assert!(!accept_sample(4, 4, original, Some((102, 12))));
        assert!(!accept_sample(4, 4, original, Some((101, 13))));
    }
    #[test]
    fn close_intent_requires_successful_source_and_cache_save() {
        assert!(should_close_after_save(true, true, true));
        assert!(!should_close_after_save(false, true, true));
        assert!(!should_close_after_save(true, false, true));
        assert!(!should_close_after_save(true, true, false));
    }
}
