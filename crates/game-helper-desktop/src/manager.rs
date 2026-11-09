//! Native route editor. Blocking IO and daemon messages use background jobs.
use anyhow::{Context as _, Result, ensure};
use game_helper_core::{
    document::{EditMode, RouteDocument},
    manager_ui::{
        ManagerUiFile, ManagerUiState, OperationUiState, PointUiKey, PointUiState,
        normalize_route_key,
    },
    route::{OpKind, Operation, Point},
    storage::{self, AppPaths, ConfigFile, CursorState, DiskSnapshot, RouteEntry},
};
use game_helper_windows::daemon::{DaemonClient, DaemonStatus};
use gpui_kit::component::{
    Disableable,
    button::Button,
    input::{EditorState, InputEvent, InputState},
    select::{SelectEvent, SelectItem, SelectState},
};
use gpui_kit::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, Render, ScrollHandle,
    SharedString, Subscription, Task, Window,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

mod interaction;
mod persistence;
mod point_interaction;
mod view;
use interaction::OperationDragState;
use point_interaction::PointDragState;

#[derive(Clone)]
enum Action {
    ToggleLibrary,
    ToggleProperties,
    ToggleFiles,
    ToggleService,
    ToggleNew,
    TogglePointInfo,
    ToggleOp(usize),
    ToggleAllOps,
    Refresh,
    Open(PathBuf),
    New,
    Import,
    ImportText(String, String, bool),
    Export,
    Save(bool, Option<Box<Action>>),
    Delete(bool),
    Switch(EditMode),
    Select(usize),
    AddPoint,
    CopyPoint,
    RemovePoint,
    Move(i32),
    MovePointTo { from: usize, to: usize },
    AddOp(OpKind, Option<usize>),
    RemoveOp(usize),
    MoveOpTo { from: usize, to: usize },
    Start,
    Stop,
    Reload,
    Bind,
    AutoStart,
    RunPoint(usize, bool),
    Picker,
    Reveal,
    Close,
    Discard(Box<Action>),
    Cancel,
}
struct Confirmation {
    message: String,
    choices: Vec<(&'static str, Action)>,
}
#[derive(Clone)]
struct ParameterChoice {
    label: SharedString,
    value: serde_json::Value,
}
impl SelectItem for ParameterChoice {
    type Value = serde_json::Value;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.value
    }
}
fn parameter_choices(key: &str) -> Option<Vec<ParameterChoice>> {
    let values = match key {
        "button" => vec![
            ("左键", serde_json::json!("left")),
            ("右键", serde_json::json!("right")),
            ("中键", serde_json::json!("middle")),
        ],
        "up" => vec![
            ("按下", serde_json::json!(false)),
            ("松开", serde_json::json!(true)),
        ],
        _ => return None,
    };
    Some(
        values
            .into_iter()
            .map(|(label, value)| ParameterChoice {
                label: label.into(),
                value,
            })
            .collect(),
    )
}
enum OperationInput {
    Text(Entity<InputState>, bool),
    Choice(Entity<SelectState<Vec<ParameterChoice>>>),
}
impl OperationInput {
    fn value(&self, cx: &App) -> Result<serde_json::Value> {
        match self {
            Self::Text(input, is_text) => {
                let text = input.read(cx).value().to_string();
                if *is_text {
                    Ok(serde_json::Value::String(text))
                } else {
                    Ok(serde_json::from_str(&text)?)
                }
            }
            Self::Choice(state) => state
                .read(cx)
                .selected_value()
                .cloned()
                .context("请选择参数值"),
        }
    }
}
struct OperationFields {
    ui_id: u64,
    drag_focus: FocusHandle,
    kind: OpKind,
    name: Entity<InputState>,
    expanded: bool,
    toggle_focus: FocusHandle,
    fields: Vec<(String, OperationInput)>,
}

pub struct Manager {
    paths: AppPaths,
    config: Option<ConfigFile>,
    routes: Vec<RouteEntry>,
    document: Option<RouteDocument>,
    search: Entity<InputState>,
    library_open: Option<bool>,
    properties_open: bool,
    files_open: bool,
    service_open: bool,
    new_open: bool,
    status_is_error: bool,
    fields: BTreeMap<&'static str, Entity<InputState>>,
    source: Entity<EditorState>,
    operation_fields: Vec<OperationFields>,
    operation_subscriptions: Vec<Subscription>,
    point_info_open: bool,
    point_info_focus: FocusHandle,
    all_ops_focus: FocusHandle,
    operation_scroll: ScrollHandle,
    point_scroll: ScrollHandle,
    point_drag_focus: Vec<FocusHandle>,
    point_drag: Option<PointDragState>,
    drag: Option<OperationDragState>,
    drag_scroll: gpui_kit::base::AutoScroll,
    drag_epoch: u64,
    move_operation: Option<usize>,
    move_position: Entity<InputState>,
    ui_state: ManagerUiState,
    ui_persisted_state: ManagerUiState,
    ui_baseline: Vec<PointUiState>,
    ui_discard_pending: bool,
    ui_snapshot: Option<DiskSnapshot>,
    ui_error: Option<String>,
    ui_revision: u64,
    ui_saved_revision: u64,
    ui_due: Option<Instant>,
    ui_writing: bool,
    ui_closing: bool,
    ui_job: Option<Task<()>>,
    ui_timer: Option<Task<()>>,
    selected: Option<usize>,
    form_pending: bool,
    source_pending: bool,
    busy: bool,
    opening_route: bool,
    status: SharedString,
    daemon: DaemonStatus,
    runner: Option<std::process::Child>,
    daemon_error: Option<String>,
    cursor: CursorState,
    confirmation: Option<Confirmation>,
    confirmation_focus: gpui_kit::FocusHandle,
    confirmation_visible: bool,
    _subscriptions: Vec<Subscription>,
    job: Option<Task<()>>,
    poll: Option<Task<()>>,
}
impl Manager {
    pub fn new(paths: AppPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut fields = BTreeMap::new();
        let mut subscriptions = Vec::new();
        for key in [
            "stem",
            "name",
            "note",
            "process",
            "class",
            "title",
            "point_name",
            "point_note",
            "hotkey",
            "start",
        ] {
            let placeholder = match key {
                "stem" => "例如：每日采集",
                "name" => "给路线起个名称",
                "note" => "这条路线的用途",
                "process" => "例如：notepad.exe",
                "class" => "按窗口类匹配",
                "title" => "按窗口标题匹配",
                "point_name" => "给点位起个名称",
                "point_note" => "记录这一步的说明",
                "hotkey" => "例如：F9",
                "start" => "1",
                _ => "",
            };
            let field = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            if !matches!(key, "stem" | "hotkey" | "start") {
                subscriptions.push(cx.subscribe(&field, |view, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        view.form_pending = true;
                        cx.notify();
                    }
                }));
            }
            fields.insert(key, field);
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索名称或文件名"));
        subscriptions.push(cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()));
        fields["start"].update(cx, |state, cx| state.set_value("1", window, cx));
        let source = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });
        subscriptions.push(cx.subscribe(&source, |v, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                v.source_pending = true;
                cx.notify();
            }
        }));
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |view, cx| {
                view.gate(Action::Close, window, cx);
                false
            })
            .unwrap_or(true)
        });
        let mut view = Self {
            paths: paths.clone(),
            config: None,
            search,
            library_open: None,
            properties_open: false,
            files_open: false,
            service_open: false,
            new_open: false,
            status_is_error: false,
            routes: Vec::new(),
            document: None,
            fields,
            source,
            operation_fields: Vec::new(),
            operation_subscriptions: Vec::new(),
            point_info_open: false,
            point_info_focus: cx.focus_handle(),
            all_ops_focus: cx.focus_handle(),
            operation_scroll: ScrollHandle::new(),
            point_scroll: ScrollHandle::new(),
            point_drag_focus: Vec::new(),
            point_drag: None,
            drag: None,
            drag_scroll: gpui_kit::base::AutoScroll::default(),
            drag_epoch: 0,
            move_operation: None,
            move_position: cx.new(|cx| InputState::new(window, cx).placeholder("输入目标序号")),
            ui_state: ManagerUiState::default(),
            ui_persisted_state: ManagerUiState::default(),
            ui_baseline: Vec::new(),
            ui_discard_pending: false,
            ui_snapshot: None,
            ui_error: None,
            ui_revision: 0,
            ui_saved_revision: 0,
            ui_due: None,
            ui_writing: false,
            ui_closing: false,
            ui_job: None,
            ui_timer: None,
            selected: None,
            form_pending: false,
            source_pending: false,
            busy: false,
            opening_route: false,
            status: "正在加载路线…".into(),
            daemon: DaemonStatus::default(),
            runner: None,
            daemon_error: None,
            cursor: CursorState::default(),
            confirmation: None,
            confirmation_focus: cx.focus_handle(),
            confirmation_visible: false,
            _subscriptions: subscriptions,
            job: None,
            poll: None,
        };
        view.worker(
            window,
            cx,
            move || {
                Ok((
                    ConfigFile::open(paths.config())?,
                    storage::list_routes(&paths)?,
                    ManagerUiFile::open(&paths),
                ))
            },
            |v, result, window, cx| match result {
                Ok((config, routes, ui)) => {
                    match ui {
                        Ok(file) => {
                            v.ui_persisted_state = file.state.clone();
                            v.ui_state = file.state;
                            v.ui_snapshot = Some(file.snapshot);
                        }
                        Err(error) => {
                            v.ui_error =
                                Some(format!("读取展开状态失败：{error:#}；原文件已保留，可重试"))
                        }
                    }
                    let auto = config.value.auto_start_daemon;
                    v.config = Some(config);
                    v.routes = routes;
                    v.status = "就绪".into();
                    if auto {
                        v.action(Action::Start, window, cx);
                    }
                }
                Err(e) => v.error(e),
            },
        );
        let client = DaemonClient::new(view.paths.clone());
        let paths = view.paths.clone();
        view.poll = Some(cx.spawn(async move |weak, cx| {
            let mut tick = 0;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                tick += 1;
                let client = client.clone();
                let paths = paths.clone();
                let (state, cursor) = cx
                    .background_spawn(async move {
                        (
                            if tick % 2 == 0 {
                                Some(client.status())
                            } else {
                                None
                            },
                            storage::read_cursor(&paths),
                        )
                    })
                    .await;
                if weak
                    .update(cx, |v, cx| {
                        if let Some(child) = v.runner.as_mut() {
                            if let Ok(Some(exit)) = child.try_wait() {
                                v.runner = None;
                                v.status =
                                    format!("runner 进程已退出：{exit}；执行详情见日志").into();
                            }
                        }
                        if let Some(state) = state {
                            match state {
                                Ok(s) => {
                                    v.daemon = s;
                                    v.daemon_error = None;
                                }
                                Err(e) => v.daemon_error = Some(format!("{e:#}")),
                            }
                        }
                        match cursor {
                            Ok(c) => v.cursor = c,
                            Err(e) => v.daemon_error = Some(format!("游标读取失败：{e:#}")),
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
    fn worker<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> Result<T> + Send + 'static,
        apply: impl FnOnce(&mut Self, Result<T>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.job = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = cx.background_spawn(async move { work() }).await;
            let _ = weak.update_in(cx, |v, window, cx| {
                v.busy = false;
                apply(v, result, window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn error(&mut self, e: anyhow::Error) {
        self.status_is_error = true;
        self.status = format!("{e:#}").into();
    }
    fn text(&self, key: &str, cx: &App) -> String {
        self.fields[key].read(cx).value().to_string()
    }
    fn set(&self, key: &str, text: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.fields[key].update(cx, |s, cx| s.set_value(text.into(), window, cx));
    }
    fn dirty(&self) -> bool {
        self.form_pending
            || self.source_pending
            || self.document.as_ref().is_some_and(RouteDocument::dirty)
    }
    fn flush(&mut self, cx: &App) -> Result<()> {
        let Some(mut doc) = self.document.clone() else {
            return Ok(());
        };
        if doc.mode() == EditMode::Source {
            if self.source_pending {
                doc.set_source(self.source.read(cx).value().to_string())?;
            }
        } else if self.form_pending {
            let mut route = doc.route().clone();
            route.name = self.text("name", cx);
            route.note = self.text("note", cx);
            route.target.process = self.text("process", cx);
            route.target.window_class = self.text("class", cx);
            route.target.title = self.text("title", cx);
            if let Some(index) = self.selected {
                let point = route.points.get_mut(index).context("选中的点位已不存在")?;
                point.name = self.text("point_name", cx);
                point.note = self.text("point_note", cx);
                point.operations = self
                    .operation_fields
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let mut params = serde_json::Map::new();
                        for (key, input) in &row.fields {
                            let value = input.value(cx).with_context(|| {
                                format!("operations[{index}].{key}: 参数格式错误")
                            })?;
                            if key != "jitter" || value.as_f64() != Some(0.0) {
                                params.insert(key.clone(), value);
                            }
                        }
                        Ok(Operation {
                            kind: row.kind,
                            name: row.name.read(cx).value().to_string(),
                            params,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
            }
            if &route != doc.route() {
                doc.edit(|r| *r = route)?;
            }
        }
        self.document = Some(doc);
        self.form_pending = false;
        self.source_pending = false;
        self.remember_operation_state()?;
        Ok(())
    }
    fn install(&mut self, doc: RouteDocument, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_list_drag(cx);
        if self.ui_discard_pending {
            self.restore_discarded_ui();
            self.ui_discard_pending = false;
        } else {
            let _ = self.remember_operation_state();
        }
        let target = &doc.route().target;
        self.properties_open =
            target.process.is_empty() && target.window_class.is_empty() && target.title.is_empty();
        self.files_open = false;
        self.new_open = false;
        self.selected = if doc.route().points.is_empty() {
            None
        } else {
            Some(0)
        };
        self.document = Some(doc);
        self.prepare_ui_baseline();
        self.sync(window, cx);
    }
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_list_drag(cx);
        self.reset_point_view();
        self.point_scroll
            .set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)));
        self.point_drag_focus = self
            .document
            .as_ref()
            .map(|doc| {
                doc.route()
                    .points
                    .iter()
                    .map(|_| cx.focus_handle())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(doc) = self.document.as_ref() {
            let r = doc.route();
            for (k, val) in [
                ("name", &r.name),
                ("note", &r.note),
                ("process", &r.target.process),
                ("class", &r.target.window_class),
                ("title", &r.target.title),
            ] {
                self.set(k, val.clone(), window, cx);
            }
            self.set("stem", doc.stem().unwrap_or_default(), window, cx);
            self.source.update(cx, |s, cx| {
                s.set_value(doc.source().to_string(), window, cx)
            });
            let binding = doc
                .stem()
                .ok()
                .and_then(|stem| self.config.as_ref()?.value.binding_for(&stem));
            self.set(
                "hotkey",
                binding.map(|b| b.hotkey.clone()).unwrap_or_default(),
                window,
                cx,
            );
            self.set(
                "start",
                binding.map(|b| b.start_at).unwrap_or(1).to_string(),
                window,
                cx,
            );
        }
        self.sync_point(window, cx);
        self.form_pending = false;
        self.source_pending = false;
    }
    fn reset_point_view(&mut self) {
        self.point_info_open = false;
        // The first layout has no viewport bounds yet; item alignment would
        // subtract its absolute origin from zero and hide the first action.
        self.operation_scroll
            .set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)));
    }
    fn sync_point(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restored = self.restore_operation_states();
        self.operation_fields.clear();
        self.operation_subscriptions.clear();
        if let Some(p) = self
            .document
            .as_ref()
            .and_then(|d| self.selected.and_then(|i| d.route().points.get(i)))
            .cloned()
        {
            self.set("point_name", p.name.clone(), window, cx);
            self.set("point_note", p.note.clone(), window, cx);
            let position = p.action_position();
            for (op_index, op) in p.operations.into_iter().enumerate() {
                let name = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("例如：打开仓库")
                        .default_value(op.name)
                });
                self.operation_subscriptions.push(cx.subscribe(
                    &name,
                    |v, _, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            v.form_pending = true;
                            cx.notify();
                        }
                    },
                ));
                let mut params = Operation::new(op.kind, position).params;
                params.extend(op.params);
                params
                    .entry("jitter".to_string())
                    .or_insert(serde_json::json!(0));
                let mut fields = Vec::new();
                for (key, value) in params {
                    let input = if let Some(choices) = parameter_choices(&key) {
                        let selected = choices
                            .iter()
                            .position(|choice| choice.value == value)
                            .map(gpui_kit::component::IndexPath::new);
                        let state = cx.new(|cx| SelectState::new(choices, selected, window, cx));
                        self.operation_subscriptions.push(cx.subscribe(
                            &state,
                            |v, _, event: &SelectEvent<Vec<ParameterChoice>>, cx| {
                                if matches!(event, SelectEvent::Confirm(Some(_))) {
                                    v.form_pending = true;
                                    cx.notify();
                                }
                            },
                        ));
                        OperationInput::Choice(state)
                    } else {
                        let is_text = value.is_string();
                        let text = value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string());
                        let input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(key.clone())
                                .default_value(text)
                        });
                        self.operation_subscriptions.push(cx.subscribe(
                            &input,
                            |v, _, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    v.form_pending = true;
                                    cx.notify();
                                }
                            },
                        ));
                        OperationInput::Text(input, is_text)
                    };
                    fields.push((key, input));
                }
                self.operation_fields.push(OperationFields {
                    ui_id: restored
                        .get(op_index)
                        .map(|s| s.id)
                        .unwrap_or(op_index as u64 + 1),
                    drag_focus: cx.focus_handle(),
                    kind: op.kind,
                    name,
                    expanded: restored.get(op_index).is_none_or(|s| s.expanded),
                    toggle_focus: cx.focus_handle(),
                    fields,
                });
            }
        }
        self.form_pending = false;
    }
    fn gate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.ui_closing {
            self.status = "正在处理，请稍候".into();
            return;
        }
        if matches!(&action, Action::Open(path) if self.document.as_ref().is_some_and(|doc| &doc.path == path))
        {
            return;
        }
        if self.dirty() {
            self.confirmation = Some(Confirmation {
                message: "当前路线有未保存修改。".into(),
                choices: vec![
                    (
                        "保存后继续",
                        Action::Save(false, Some(Box::new(action.clone()))),
                    ),
                    ("放弃并继续", Action::Discard(Box::new(action))),
                    ("取消", Action::Cancel),
                ],
            });
            cx.notify();
        } else {
            self.action(action, window, cx);
        }
    }
    fn action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.ui_closing {
            return;
        }
        self.confirmation = None;
        self.status_is_error = false;
        let result = self.perform(action, window, cx);
        if let Err(e) = result {
            self.error(e);
        }
        cx.notify();
    }
    fn perform(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        match action {
            Action::ToggleLibrary => {
                let wide = window.viewport_size().width >= gpui_kit::px(1100.);
                self.library_open = Some(!self.library_open.unwrap_or(wide));
            }
            Action::TogglePointInfo => {
                self.point_info_open = !self.point_info_open;
                if self.point_info_open {
                    self.operation_scroll.scroll_to_top_of_item(0);
                    self.fields["point_name"]
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                } else {
                    self.point_info_focus.focus(window, cx);
                }
            }
            Action::ToggleOp(index) => {
                if let Some(op) = self.operation_fields.get_mut(index) {
                    op.expanded = !op.expanded;
                    op.toggle_focus.focus(window, cx);
                    self.operation_scroll
                        .scroll_to_top_of_item(index + self.operation_child_offset());
                }
                self.remember_operation_state()?;
            }
            Action::ToggleAllOps => {
                let expand = !self.operation_fields.iter().any(|op| op.expanded);
                for op in &mut self.operation_fields {
                    op.expanded = expand;
                }
                self.all_ops_focus.focus(window, cx);
                self.operation_scroll.scroll_to_top_of_item(0);
                self.remember_operation_state()?;
            }
            Action::ToggleProperties => self.properties_open = !self.properties_open,
            Action::ToggleFiles => self.files_open = !self.files_open,
            Action::ToggleService => self.service_open = !self.service_open,
            Action::ToggleNew => {
                self.new_open = !self.new_open;
                if self.new_open {
                    self.set("stem", "", window, cx);
                }
                let wide = window.viewport_size().width >= gpui_kit::px(1100.);
                if !self.library_open.unwrap_or(wide) {
                    self.library_open = Some(true);
                }
            }
            Action::Cancel => {}
            Action::Discard(next) => {
                self.ui_discard_pending = true;
                self.action(*next, window, cx);
            }
            Action::Close => self.close_with_ui_state(window, cx),
            Action::Refresh => {
                let paths = self.paths.clone();
                self.worker(
                    window,
                    cx,
                    move || storage::list_routes(&paths),
                    |v, result, _, _| match result {
                        Ok(r) => {
                            v.routes = r;
                            v.status = "路线列表已刷新".into();
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::Open(path) => {
                if self.document.as_ref().is_some_and(|doc| doc.path == path) {
                    return Ok(());
                }
                self.opening_route = true;
                self.confirmation_focus.focus(window, cx);
                self.worker(
                    window,
                    cx,
                    move || RouteDocument::open_for_edit(path),
                    |v, result, w, cx| {
                        v.opening_route = false;
                        match result {
                            Ok(d) => {
                                v.install(d, w, cx);
                                v.status = "路线已打开".into();
                            }
                            Err(e) => {
                                v.ui_discard_pending = false;
                                v.error(e);
                            }
                        }
                    },
                );
            }
            Action::New => {
                let stem = self.text("stem", cx);
                let paths = self.paths.clone();
                self.worker(
                    window,
                    cx,
                    move || RouteDocument::draft(paths.route_path(&stem)?, stem),
                    |v, r, w, cx| match r {
                        Ok(d) => {
                            v.install(d, w, cx);
                            if let Some(path) = v.document.as_ref().map(|doc| doc.path.clone()) {
                                v.clear_route_ui(&path);
                                v.sync_point(w, cx);
                            }
                            v.status = "内存草稿：填写目标后保存".into();
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::Switch(mode) => {
                self.flush(cx)?;
                self.cancel_list_drag(cx);
                let doc = self.document.as_mut().context("请选择路线")?;
                doc.switch(mode)?;
                self.selected = if doc.route().points.is_empty() {
                    None
                } else {
                    Some(self.selected.unwrap_or(0).min(doc.route().points.len() - 1))
                };
                self.sync(window, cx);
            }
            Action::Select(index) => {
                self.flush(cx)?;
                self.cancel_list_drag(cx);
                self.selected = Some(index);
                self.reset_point_view();
                self.sync_point(window, cx);
                self.point_info_focus.focus(window, cx);
            }
            Action::AddPoint
            | Action::CopyPoint
            | Action::RemovePoint
            | Action::Move(_)
            | Action::MovePointTo { .. }
            | Action::AddOp(_, _)
            | Action::RemoveOp(_)
            | Action::MoveOpTo { .. } => self.edit_sequence(action, window, cx)?,
            Action::Save(ack, next) => {
                self.flush(cx)?;
                let doc = self.document.as_ref().context("请选择路线")?;
                if doc.needs_normalization_notice() && !ack {
                    self.confirmation = Some(Confirmation {
                        message:
                            "结构化保存将规范化 YAML，注释可能移除。原文会保存在 routes/.backups。"
                                .into(),
                        choices: vec![
                            ("备份并保存", Action::Save(true, next)),
                            ("取消", Action::Cancel),
                        ],
                    });
                    return Ok(());
                }
                doc.source_for_save()?;
                let mut doc = doc.clone();
                let paths = self.paths.clone();
                let auto_reload = self
                    .config
                    .as_ref()
                    .is_some_and(|c| c.value.auto_reload_on_save);
                self.worker(
                    window,
                    cx,
                    move || {
                        let result = doc.save(&paths, ack);
                        let entries = storage::list_routes(&paths);
                        let notice = if result.is_ok() && auto_reload {
                            let client = DaemonClient::new(paths);
                            match client.status() {
                                Ok(state) if state.running && state.busy => {
                                    Some("路线已保存；执行中未重载，请空闲后重载".to_string())
                                }
                                Ok(state) if state.running => client
                                    .reload()
                                    .err()
                                    .map(|e| format!("路线已保存；自动重载失败：{e:#}")),
                                Err(e) => {
                                    Some(format!("路线已保存；daemon 状态未知，未重载：{e:#}"))
                                }
                                _ => None,
                            }
                        } else {
                            None
                        };
                        Ok((doc, result, notice, entries))
                    },
                    move |v, result, w, cx| match result {
                        Ok((doc, saved, notice, entries)) => {
                            v.document = Some(doc);
                            if let Ok(entries) = entries {
                                v.routes = entries;
                            }
                            match saved {
                                Ok(_) => {
                                    v.prepare_ui_baseline();
                                    v.status = notice
                                        .unwrap_or_else(|| "路线与执行缓存已保存".into())
                                        .into();
                                    if let Some(next) = next {
                                        v.action(*next, w, cx);
                                    }
                                }
                                Err(e) => v.error(e),
                            }
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::Delete(ack) => {
                ensure!(!self.daemon.busy, "执行中不能删除路线");
                if !ack {
                    self.confirmation = Some(Confirmation {
                        message: "删除当前路线、缓存与热键绑定？源文件会备份。".into(),
                        choices: vec![("确认删除", Action::Delete(true)), ("取消", Action::Cancel)],
                    });
                    return Ok(());
                }
                let doc = self.document.as_ref().context("请选择路线")?;
                let path = doc.path.clone();
                let expected = doc.disk_snapshot().clone();
                let cleanup_path = path.clone();
                let paths = self.paths.clone();
                let mut config = self.config.clone().context("配置不可用")?;
                self.worker(
                    window,
                    cx,
                    move || {
                        storage::delete_route(&paths, &mut config, &path, &expected)?;
                        Ok(config)
                    },
                    move |v, r, w, cx| match r {
                        Ok(c) => {
                            v.cancel_list_drag(cx);
                            v.clear_route_ui(&cleanup_path);
                            v.config = Some(c);
                            v.document = None;
                            v.selected = None;
                            v.form_pending = false;
                            v.source_pending = false;
                            v.action(Action::Refresh, w, cx);
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::Start | Action::Stop | Action::Reload => {
                let client = DaemonClient::new(self.paths.clone());
                let config = self.config.clone().context("配置不可用")?;
                self.worker(
                    window,
                    cx,
                    move || {
                        match action {
                            Action::Start => client.start(&config.value)?,
                            Action::Stop => client.stop()?,
                            _ => client.reload()?,
                        };
                        client.status()
                    },
                    |v, r, _, _| match r {
                        Ok(s) => {
                            v.daemon = s;
                            v.status = "daemon 状态已更新".into();
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::Bind | Action::AutoStart => {
                ensure!(!self.daemon.busy, "执行中不能更改绑定或重载");
                let mut config = self.config.clone().context("配置不可用")?;
                if matches!(action, Action::Bind) {
                    self.flush(cx)?;
                    let doc = self.document.as_ref().context("请选择路线")?;
                    ensure!(!doc.dirty(), "请先保存路线，再绑定热键");
                    let start = self
                        .text("start", cx)
                        .parse::<usize>()
                        .context("起始位置须为整数")?;
                    ensure!(
                        start > 0 && start <= doc.route().points.len(),
                        "起始位置超出点位列表"
                    );
                    config
                        .value
                        .set_binding(&doc.stem()?, &self.text("hotkey", cx), start)?;
                } else {
                    config.value.auto_start_daemon = !config.value.auto_start_daemon;
                }
                let client = DaemonClient::new(self.paths.clone());
                let reload = self.daemon.running;
                self.worker(
                    window,
                    cx,
                    move || {
                        config.save()?;
                        let notice = if reload {
                            client
                                .reload()
                                .err()
                                .map(|e| format!("配置已保存；重载失败：{e:#}"))
                        } else {
                            None
                        };
                        Ok((config, notice))
                    },
                    |v, r, _, _| match r {
                        Ok((c, note)) => {
                            v.config = Some(c);
                            v.status = note.unwrap_or_else(|| "配置已保存".into()).into();
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::RunPoint(only, runner_ack) => {
                self.flush(cx)?;
                let doc = self.document.as_ref().context("请选择路线")?;
                ensure!(!doc.dirty(), "有未保存修改，请保存后执行");
                let (route, _) = doc.source_for_save()?;
                let start = self
                    .text("start", cx)
                    .parse::<usize>()
                    .context("起始位置须为整数")?;
                ensure!(
                    start > 0
                        && start <= route.points.len()
                        && only > 0
                        && only <= route.points.len(),
                    "起始或单点位置超出点位列表"
                );
                ensure!(self.runner.is_none(), "独立 runner 仍在运行，不能重复执行");
                ensure!(
                    !self.daemon.busy && self.daemon_error.is_none(),
                    "daemon 忙碌或状态未知，请等待状态恢复"
                );
                if !self.daemon.running && !runner_ack {
                    self.confirmation = Some(Confirmation {
                        message: "daemon 未运行。使用独立 runner 试跑选中点？".into(),
                        choices: vec![
                            ("使用 runner", Action::RunPoint(only, true)),
                            ("取消", Action::Cancel),
                        ],
                    });
                    return Ok(());
                }
                let paths = self.paths.clone();
                let stem = doc.stem()?;
                let path = doc.path.clone();
                let expected = doc.disk_snapshot().clone();
                let config = self.config.clone().context("配置不可用")?;
                self.worker(
                    window,
                    cx,
                    move || {
                        ensure!(expected.unchanged(&path)?, "磁盘路线已变化，请重新打开");
                        let cache = storage::cache_route(&paths, &stem, &route)?;
                        let client = DaemonClient::new(paths);
                        if runner_ack {
                            Ok(Some(client.spawn_runner(
                                &config.value,
                                &cache,
                                start,
                                only,
                            )?))
                        } else {
                            client.run_route(&stem, start, only)?;
                            Ok(None)
                        }
                    },
                    |v, r, _, _| match r {
                        Ok(child) => {
                            v.runner = child;
                            v.status = "执行请求已受理；完成状态请查看目标窗口和日志".into();
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::Picker => {
                let root = self.paths.root.clone();
                self.worker(
                    window,
                    cx,
                    move || {
                        std::process::Command::new(std::env::current_exe()?)
                            .arg("--picker")
                            .arg("--app-root")
                            .arg(root)
                            .spawn()?;
                        Ok(())
                    },
                    |v, r, _, _| {
                        if let Err(e) = r {
                            v.error(e);
                        }
                    },
                );
            }
            Action::Reveal => {
                if let Some(doc) = &self.document {
                    cx.reveal_path(&doc.path);
                }
            }
            Action::Import => {
                let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: None,
                });
                self.busy = true;
                self.job = Some(cx.spawn_in(window, async move |weak, cx| {
                    let result = receiver.await;
                    let path = match result {
                        Ok(Ok(Some(paths))) => paths.into_iter().next(),
                        _ => None,
                    };
                    let result = if let Some(path) = path {
                        Some(
                            cx.background_spawn(async move {
                                let text = std::fs::read_to_string(&path)?;
                                let stem = path
                                    .file_stem()
                                    .context("导入文件名无效")?
                                    .to_string_lossy()
                                    .into_owned();
                                game_helper_core::route::Route::parse(&text)?;
                                Ok::<_, anyhow::Error>((stem, text))
                            })
                            .await,
                        )
                    } else {
                        None
                    };
                    let _ = weak.update_in(cx, |v, w, cx| {
                        v.busy = false;
                        if let Some(r) = result {
                            match r {
                                Ok((stem, text)) => {
                                    v.action(Action::ImportText(stem, text, false), w, cx)
                                }
                                Err(e) => v.error(e),
                            }
                        }
                        cx.notify();
                    });
                }));
            }
            Action::ImportText(stem, text, overwrite) => {
                let paths = self.paths.clone();
                self.worker(
                    window,
                    cx,
                    move || {
                        let path = paths.route_path(&stem)?;
                        let exists = path.exists();
                        if exists && !overwrite {
                            return Ok(Err((stem, text)));
                        }
                        Ok(Ok(RouteDocument::from_source(path, &text, overwrite)?))
                    },
                    |v, r, w, cx| match r {
                        Ok(Ok(d)) => {
                            v.install(d, w, cx);
                            if let Some(path) = v.document.as_ref().map(|doc| doc.path.clone()) {
                                v.clear_route_ui(&path);
                                v.sync_point(w, cx);
                            }
                            v.status = "导入内容已载入，保存后生效".into();
                        }
                        Ok(Err((stem, text))) => {
                            v.confirmation = Some(Confirmation {
                                message: format!("{stem}.yaml 已存在。"),
                                choices: vec![
                                    (
                                        "覆盖（保存时备份）",
                                        Action::ImportText(stem.clone(), text.clone(), true),
                                    ),
                                    (
                                        "另存副本",
                                        Action::ImportText(
                                            format!(
                                                "{stem}-导入-{}",
                                                std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .unwrap_or_default()
                                                    .as_millis()
                                            ),
                                            text,
                                            false,
                                        ),
                                    ),
                                    ("取消", Action::Cancel),
                                ],
                            });
                        }
                        Err(e) => v.error(e),
                    },
                );
            }
            Action::Export => {
                self.flush(cx)?;
                let text = self
                    .document
                    .as_ref()
                    .context("请选择路线")?
                    .export_source()?;
                let receiver = cx.prompt_for_new_path(&self.paths.routes(), Some("导出路线.yaml"));
                self.busy = true;
                self.job = Some(cx.spawn_in(window, async move |weak, cx| {
                    let result = match receiver.await {
                        Ok(Ok(Some(path))) => Some(
                            cx.background_spawn(async move {
                                ensure!(!path.exists(), "导出目标已存在，请选择新文件名");
                                storage::save_atomic(
                                    &path,
                                    &DiskSnapshot(None),
                                    text.as_bytes(),
                                    true,
                                )?;
                                Ok::<_, anyhow::Error>(())
                            })
                            .await,
                        ),
                        _ => None,
                    };
                    let _ = weak.update_in(cx, |v, _, cx| {
                        v.busy = false;
                        if let Some(r) = result {
                            match r {
                                Ok(()) => v.status = "导出已保存".into(),
                                Err(e) => v.error(e),
                            }
                        }
                        cx.notify();
                    });
                }));
            }
        }
        Ok(())
    }
    fn controls_busy(&self) -> bool {
        self.ui_closing || (self.busy && !self.opening_route)
    }
    fn button(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        label: impl Into<SharedString>,
        action: Action,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        use gpui_kit::Styled as _;
        Button::new(id)
            .label(label)
            .h(gpui_kit::px(32.))
            .disabled(self.controls_busy() || disabled || self.confirmation.is_some())
            .on_click(cx.listener(move |v, _, w, cx| {
                if matches!(
                    action,
                    Action::Open(_) | Action::New | Action::Import | Action::Close
                ) {
                    v.gate(action.clone(), w, cx);
                } else {
                    v.action(action.clone(), w, cx);
                }
            }))
    }
}

impl Render for Manager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.schedule_ui_persistence(window, cx);
        let showing = self.confirmation.is_some();
        if (showing
            || self.busy
            || self.opening_route
            || self.ui_closing
            || !window.is_window_active())
            && (self.drag.is_some() || self.point_drag.is_some())
        {
            self.cancel_list_drag(cx);
        }
        if showing && !self.confirmation_visible {
            self.confirmation_focus.focus(window, cx);
        }
        self.confirmation_visible = showing;
        self.render_workspace(window, cx)
    }
}
