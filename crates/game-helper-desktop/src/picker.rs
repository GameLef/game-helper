//! Independent native picker. RegisterHotKey is owned by this window only.
mod view;

use anyhow::{Context as _, Result};
use game_helper_core::{
    picker::{CapturedPoint, PickerFile, PickerTarget},
    storage::AppPaths,
};
use game_helper_windows::{hotkey::HotkeyService, window as win};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, FocusHandle, IntoElement, Render, ScrollHandle,
    SharedString, Subscription, Task, Window,
};
use std::time::{Duration, Instant};

gpui_kit::actions!(picker, [ModalNext, ModalPrevious]);

struct Captured {
    point: CapturedPoint,
    name: Entity<InputState>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FeedbackKind {
    Info,
    Success,
    Error,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirmation {
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
    active: Option<PickerTarget>,
    points: Vec<Captured>,
    search: Entity<InputState>,
    status: SharedString,
    status_kind: FeedbackKind,
    coordinates: SharedString,
    sample_state: SampleState,
    hotkey: Option<HotkeyService>,
    countdown: Option<Countdown>,
    interaction_epoch: u64,
    busy: bool,
    file: Option<PickerFile>,
    revision: u64,
    saved_revision: u64,
    persist_due: Option<Instant>,
    persisting: bool,
    persistence_error: Option<SharedString>,
    closing: bool,
    confirmation: Option<Confirmation>,
    library_open: Option<bool>,
    point_scroll: ScrollHandle,
    confirm_focus: FocusHandle,
    confirm_primary_focus: FocusHandle,
    confirm_discard_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
    point_subscriptions: Vec<Subscription>,
    job: Option<Task<()>>,
    persist_job: Option<Task<()>>,
    clipboard_job: Option<Task<()>>,
    poll: Option<Task<()>>,
}
impl Picker {
    pub fn new(paths: AppPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            gpui_kit::KeyBinding::new("tab", ModalNext, Some("PickerConfirmation")),
            gpui_kit::KeyBinding::new("shift-tab", ModalPrevious, Some("PickerConfirmation")),
        ]);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索标题或进程"));
        let subscriptions = vec![cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify())];
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |w, cx| {
            weak.update(cx, |v, cx| v.request_close(w, cx))
                .unwrap_or(true)
        });
        let mut view = Self {
            paths,
            windows: Vec::new(),
            target: None,
            active: None,
            points: Vec::new(),
            search,
            status: "正在读取已采集点位…".into(),
            status_kind: FeedbackKind::Info,
            coordinates: "未选择目标窗口".into(),
            sample_state: SampleState::Unselected,
            hotkey: None,
            countdown: None,
            interaction_epoch: 0,
            busy: false,
            file: None,
            revision: 0,
            saved_revision: 0,
            persist_due: None,
            persisting: false,
            persistence_error: None,
            closing: false,
            confirmation: None,
            library_open: None,
            point_scroll: ScrollHandle::new(),
            confirm_focus: cx.focus_handle(),
            confirm_primary_focus: cx.focus_handle(),
            confirm_discard_focus: cx.focus_handle(),
            return_focus: None,
            _subscriptions: subscriptions,
            point_subscriptions: Vec::new(),
            job: None,
            persist_job: None,
            clipboard_job: None,
            poll: None,
        };
        view.load(window, cx);
        view.poll = Some(cx.spawn_in(window, async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let Ok((target, capture, selecting, epoch, countdown_due)) =
                    weak.update_in(cx, |v, w, cx| {
                        if v.persist_due.is_some_and(|due| Instant::now() >= due) {
                            v.persist(w, cx);
                        }
                        let mut events = 0;
                        if let Some(service) = &v.hotkey {
                            while service.try_recv().is_ok() {
                                events += 1;
                            }
                        }
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
                                v.choose(target, w, cx);
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
        self.busy || self.file.is_none() || self.closing || self.confirmation.is_some()
    }
    fn invalidate_commands(&mut self) {
        self.countdown = None;
        self.interaction_epoch = self.interaction_epoch.wrapping_add(1);
        if let Some(service) = &self.hotkey {
            while service.try_recv().is_ok() {}
        }
    }
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.file.is_some() {
            return;
        }
        self.busy = true;
        self.persistence_error = None;
        let paths = self.paths.clone();
        self.job = Some(cx.spawn_in(window, async move |weak, cx| {
            let (file, windows) = cx
                .background_spawn(async move { (PickerFile::open(&paths), win::list_windows()) })
                .await;
            let _ = weak.update_in(cx, |v, w, cx| {
                v.busy = false;
                match file {
                    Ok(file) => {
                        v.active = file.state.last_target.clone();
                        v.file = Some(file);
                        v.restore_points(w, cx);
                        v.feedback(
                            FeedbackKind::Info,
                            if v.active.is_some() {
                                "已恢复点位，重新选择对应窗口后可继续采点"
                            } else {
                                "选择目标窗口，然后开始采点；点位自动保存"
                            },
                        );
                    }
                    Err(e) => {
                        v.persistence_error = Some(
                            format!("读取点位失败：{e:#}。原文件已保留，请修复后重新读取").into(),
                        );
                    }
                }
                match windows {
                    Ok(windows) => {
                        v.windows = windows
                            .into_iter()
                            .filter(|w| w.pid != std::process::id())
                            .collect()
                    }
                    Err(e) => v.feedback(FeedbackKind::Error, format!("窗口枚举失败：{e}")),
                }
                cx.notify();
            });
        }));
    }
    fn restore_points(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.point_subscriptions.clear();
        self.points.clear();
        let points = self
            .active
            .as_ref()
            .and_then(|target| self.file.as_ref()?.state.group(target))
            .map(|g| g.points.clone())
            .unwrap_or_default();
        for point in points {
            self.add_row(point, window, cx);
        }
        self.point_scroll.scroll_to_bottom();
    }
    fn add_row(&mut self, point: CapturedPoint, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(point.name.clone()));
        self.point_subscriptions
            .push(cx.subscribe(&name, |v, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) && !v.locked() {
                    v.changed(Duration::from_millis(300), cx);
                    cx.notify();
                }
            }));
        self.points.push(Captured { point, name });
    }
    fn sync_points(&mut self, cx: &Context<Self>) {
        if let (Some(file), Some(target)) = (&mut self.file, &self.active) {
            let points = self
                .points
                .iter()
                .map(|row| CapturedPoint {
                    name: row.name.read(cx).value().to_string(),
                    ..row.point.clone()
                })
                .collect();
            file.state.set_points(target.clone(), points);
        }
    }
    fn changed(&mut self, delay: Duration, cx: &Context<Self>) {
        self.sync_points(cx);
        self.revision = self.revision.wrapping_add(1);
        self.persist_due = Some(Instant::now() + delay);
    }
    fn persist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.persisting || self.revision == self.saved_revision || self.file.is_none() {
            return;
        }
        self.sync_points(cx);
        let current = self.file.as_ref().unwrap();
        let mut file = PickerFile {
            state: current.state.clone(),
            snapshot: current.snapshot.clone(),
        };
        let paths = self.paths.clone();
        let revision = self.revision;
        self.persist_due = None;
        self.persisting = true;
        self.persist_job = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = cx.background_spawn(async move { file.save(&paths).map(|_| file.snapshot) }).await;
            let _ = weak.update_in(cx, |v, w, cx| {
                v.persisting = false;
                match result {
                    Ok(snapshot) => {
                        v.file.as_mut().unwrap().snapshot = snapshot;
                        v.saved_revision = revision;
                        v.persistence_error = None;
                        if v.revision != v.saved_revision {
                            // A newer mutation arrived while this snapshot was being written.
                            v.persist_due = Some(Instant::now());
                        } else if v.closing { w.remove_window(); }
                    }
                    Err(e) => {
                        v.persist_due = None;
                        v.persistence_error = Some(format!("自动保存失败：{e:#}。点位仍在内存中，可复制或重试；如文件被外部修改，请保留坐标后重新打开").into());
                        if v.closing {
                            v.closing = false;
                            v.begin_confirmation(Confirmation::Close, w, cx);
                        }
                    }
                }
                cx.notify();
            });
        }));
    }
    fn retry_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file.is_none() {
            self.load(window, cx);
        } else {
            self.persist(window, cx);
        }
    }
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.confirmation.is_some() {
            self.confirm_focus.focus(window, cx);
            return false;
        }
        self.invalidate_commands();
        if self.revision == self.saved_revision {
            return true;
        }
        self.closing = true;
        self.feedback(FeedbackKind::Info, "正在保存最新点位，完成后关闭…");
        self.persist(window, cx);
        cx.notify();
        false
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
    fn choose(&mut self, target: win::WindowInfo, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        if target.pid == std::process::id() {
            self.feedback(FeedbackKind::Error, "请选择其他应用的窗口");
            cx.notify();
            return;
        }
        if !target.is_alive() {
            self.feedback(FeedbackKind::Error, "目标窗口已关闭，请刷新后重新选择");
            cx.notify();
            return;
        }
        if target.process.is_empty() && target.class.is_empty() {
            self.feedback(
                FeedbackKind::Error,
                "无法识别目标应用及窗口类型，请选择其他窗口",
            );
            cx.notify();
            return;
        }
        let key = PickerTarget::new(
            target.process.clone(),
            target.class.clone(),
            target.title.clone(),
        );
        self.invalidate_commands();
        self.sync_points(cx);
        self.active = Some(key);
        self.restore_points(window, cx);
        self.target = Some(target);
        self.changed(Duration::ZERO, cx);
        self.feedback(
            FeedbackKind::Success,
            format!(
                "已选择 {}，恢复 {} 个点位；将光标移入客户区可继续采点",
                self.active.as_ref().unwrap().title,
                self.points.len()
            ),
        );
        self.sample_state = SampleState::Unavailable("将光标移入目标客户区".into());
        self.coordinates = "将光标移入目标客户区".into();
        cx.notify();
    }
    fn append(&mut self, position: [f64; 2], window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self
            .points
            .iter()
            .map(|p| p.point.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            self.feedback(FeedbackKind::Error, "点位编号已达上限");
            return;
        };
        self.add_row(
            CapturedPoint {
                id,
                name: format!("点位 {id}"),
                position,
            },
            window,
            cx,
        );
        self.point_scroll.scroll_to_bottom();
        self.changed(Duration::ZERO, cx);
        self.feedback(
            FeedbackKind::Success,
            format!("已采集 {} 个点位", self.points.len()),
        );
    }
    fn copy_point(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let Some(row) = self.points.get(index) else {
            return;
        };
        let text = coordinate_text(row.point.position);
        // The Windows GPUI clipboard API silently returns when OpenClipboard
        // fails. A short asynchronous retry avoids losing a click to another
        // clipboard reader, and a newer copy cancels this task.
        self.clipboard_job = None;
        self.feedback(FeedbackKind::Info, "正在复制坐标…");
        self.clipboard_job = Some(cx.spawn(async move |weak, cx| {
            for attempt in 0..9 {
                if attempt > 0 {
                    cx.background_executor()
                        .timer(Duration::from_millis(40))
                        .await;
                }
                let Ok(done) = weak.update(cx, |v, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    let copied = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .as_deref()
                        == Some(text.as_str());
                    if copied {
                        v.feedback(
                            FeedbackKind::Success,
                            format!("已复制第 {} 个点位的坐标，可粘贴到路线管理", index + 1),
                        );
                    } else if attempt == 8 {
                        v.feedback(FeedbackKind::Error, "复制失败：剪贴板正被占用，请重试");
                    }
                    cx.notify();
                    copied
                }) else {
                    return;
                };
                if done {
                    return;
                }
            }
        }));
        cx.notify();
    }
    fn move_point(&mut self, point_id: u64, delta: isize, cx: &mut Context<Self>) {
        if self.locked() || self.countdown.is_some() || !matches!(delta, -1 | 1) {
            return;
        }
        let Some(index) = self.points.iter().position(|row| row.point.id == point_id) else {
            return;
        };
        let Some(next) = index
            .checked_add_signed(delta)
            .filter(|next| *next < self.points.len())
        else {
            return;
        };
        self.points.swap(index, next);
        // Keep the subscription paired with its name entity for later removal.
        self.point_subscriptions.swap(index, next);
        self.point_scroll.scroll_to_item(next);
        self.changed(Duration::ZERO, cx);
        self.feedback(
            FeedbackKind::Info,
            format!("已将点位移至第 {} 个，顺序自动保存", next + 1),
        );
        cx.notify();
    }
    fn remove_point(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.locked() || index >= self.points.len() {
            return;
        }
        self.points.remove(index);
        drop(self.point_subscriptions.remove(index));
        self.changed(Duration::ZERO, cx);
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
        self.changed(Duration::ZERO, cx);
        self.confirmation = None;
        self.invalidate_commands();
        self.restore_focus(window, cx);
        self.feedback(
            FeedbackKind::Info,
            "已清空当前窗口的采点，其他窗口点位已保留",
        );
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
        self.closing = false;
        self.invalidate_commands();
        self.restore_focus(window, cx);
        self.feedback(FeedbackKind::Info, "已取消操作，点位内容已保留");
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
    fn retry_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirmation != Some(Confirmation::Close) {
            return;
        }
        self.confirmation = None;
        self.restore_focus(window, cx);
        self.request_close(window, cx);
    }
    fn discard_and_close(&mut self, window: &mut Window, _: &mut Context<Self>) {
        if !self.persisting && self.confirmation == Some(Confirmation::Close) {
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
}
fn coordinate_text(position: [f64; 2]) -> String {
    format!("[{:.6}, {:.6}]", position[0], position[1])
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
    fn copied_coordinates_are_directly_usable_action_parameters() {
        for pos in [[0.123456789, 0.987654321], [0.0, 1.0]] {
            let text = coordinate_text(pos);
            let parsed: [f64; 2] = serde_json::from_str(&text).unwrap();
            for i in 0..2 {
                assert!((parsed[i] - pos[i]).abs() <= 0.000001);
            }
            let mut route = game_helper_core::route::Route::draft("复制坐标");
            route.target.process = "game.exe".into();
            let mut point = game_helper_core::route::Point::new(1);
            point
                .operations
                .push(game_helper_core::route::Operation::new(
                    game_helper_core::route::OpKind::Click,
                    parsed,
                ));
            route.points.push(point);
            route.validate().unwrap();
        }
        assert_eq!(coordinate_text([0.0, 1.0]), "[0.000000, 1.000000]");
    }
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
        pending = None;
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
}
