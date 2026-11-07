//! Native Windows capability probe. Does not read/write routes or launch AHK.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app_root;
mod application;
mod manager;
mod picker;
mod workspace_theme;
fn main() {
    application::run();
}

use game_helper_windows::{hotkey::HotkeyService, window as win};
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme, IconName,
    button::Button,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    table::{Column, DataTable, TableDelegate, TableState},
};
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, PathPromptOptions, Render,
    SharedString, Styled, Subscription, Task, Window, WindowOptions, application, assets, div,
    init, open_window, px,
};

struct ProbeRows {
    names: Vec<Entity<InputState>>,
}

impl TableDelegate for ProbeRows {
    fn columns_count(&self, _: &App) -> usize {
        2
    }
    fn rows_count(&self, _: &App) -> usize {
        self.names.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        match index {
            0 => Column::new("id", "序号").width(80.),
            _ => Column::new("name", "可编辑点名（验证重绘后保留）").width(420.),
        }
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if col == 0 {
            div().child((row + 1).to_string()).into_any_element()
        } else {
            Input::new(&self.names[row]).into_any_element()
        }
    }
}

struct NativeProbe {
    input: Entity<InputState>,
    editor: Entity<EditorState>,
    table: Entity<TableState<ProbeRows>>,
    status: SharedString,
    changes: usize,
    _subscriptions: Vec<Subscription>,
    dialog_task: Option<Task<()>>,
    platform_task: Option<Task<()>>,
    hotkey_active: bool,
    coordinates: SharedString,
    captured: usize,
}

impl NativeProbe {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("输入中文，点击回显检查内容"));
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
                .default_value("format: route/v1\nname: 原生界面验证\npoints: []\n")
        });
        let names = (1..=2)
            .map(|i| cx.new(|cx| InputState::new(window, cx).default_value(format!("示例点 {i}"))))
            .collect();
        let table = cx.new(|cx| TableState::new(ProbeRows { names }, window, cx));
        let input_subscription = cx.subscribe(&input, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.changes += 1;
                cx.notify();
            }
        });
        let editor_subscription = cx.observe(&editor, |_, _, cx| cx.notify());
        Self {
            input,
            editor,
            table,
            status: "验证窗口：尚未连接路线与执行器".into(),
            changes: 0,
            _subscriptions: vec![input_subscription, editor_subscription],
            dialog_task: None,
            platform_task: None,
            hotkey_active: false,
            coordinates: "全局采点测试尚未启用".into(),
            captured: 0,
        }
    }

    fn toggle_platform(&mut self, cx: &mut Context<Self>) {
        if self.hotkey_active {
            self.platform_task = None;
            self.hotkey_active = false;
            self.coordinates = "全局采点测试已停止，热键已释放".into();
            cx.notify();
            return;
        }
        self.hotkey_active = true;
        self.platform_task = Some(cx.spawn(async move |view, cx| {
            let service = match cx.background_spawn(async { HotkeyService::start() }).await {
                Ok(service) => service,
                Err(error) => {
                    let _ = view.update(cx, |view, cx| {
                        view.hotkey_active = false;
                        view.coordinates = error.to_string().into();
                        cx.notify();
                    });
                    return;
                }
            };
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let mut count = 0;
                while service.try_recv().is_ok() {
                    count += 1;
                }
                let point = win::cursor_position();
                let captured = if count > 0 {
                    point.and_then(|(x, y)| {
                        let target = win::window_from_point(x, y)?;
                        let area = target.client_area().ok()?;
                        let (nx, ny) = area.normalize_screen(x, y)?;
                        Some(format!("{} · ({nx:.6}, {ny:.6})", target.process))
                    })
                } else {
                    None
                };
                if view
                    .update(cx, |view, cx| {
                        view.coordinates = format!(
                            "屏幕坐标 {point:?} · Ctrl+Alt+P 已触发 {} 次",
                            view.captured + count
                        )
                        .into();
                        if count > 0 {
                            view.captured += count;
                            view.status = captured
                                .unwrap_or_else(|| "采点位置不在有效客户区内".into())
                                .into();
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let result = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        self.status = "等待文件选择（仅显示路径，不读取文件）".into();
        self.dialog_task = Some(cx.spawn(async move |view, cx| {
            let result = result.await;
            let status = match result {
                Ok(Ok(Some(paths))) => paths
                    .first()
                    .map(|p| format!("已选择：{}", p.display()))
                    .unwrap_or_else(|| "未选择文件".into()),
                Ok(Ok(None)) => "已取消文件选择".into(),
                Ok(Err(error)) => format!("文件对话框失败：{error}"),
                Err(error) => format!("文件对话框已关闭：{error}"),
            };
            eprintln!("Native dialog result: {status}");
            let _ = view.update(cx, |view, cx| {
                view.status = status.into();
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for NativeProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("Game Helper · GPUI Kit 原生验证")
            .child(Input::new(&self.input).id("probe-input"))
            .child(format!(
                "输入变更 {} 次 · YAML {} 字",
                self.changes,
                self.editor.read(cx).value().chars().count()
            ))
            .child(div().h(px(130.)).child(DataTable::new(&self.table)))
            .child(Editor::new(&self.editor).h(px(200.)))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("echo")
                            .icon(IconName::Check)
                            .label("回显输入")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.status =
                                    format!("输入内容：{}", view.input.read(cx).value()).into();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("open")
                            .label("打开文件…")
                            .on_click(cx.listener(|view, _, _, cx| view.open_dialog(cx))),
                    ),
            )
            .child(
                Button::new("hotkey-probe")
                    .label(if self.hotkey_active {
                        "停止全局采点测试"
                    } else {
                        "启用 Ctrl+Alt+P 测试"
                    })
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_platform(cx))),
            )
            .child(self.coordinates.clone())
            .child(self.status.clone())
    }
}

pub fn run_probe() {
    game_helper_windows::dpi::try_set_per_monitor_v2()
        .expect("无法启用物理像素坐标所需的 DPI 感知");
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| NativeProbe::new(window, cx))
        })
        .expect("无法创建 GPUI 原生验证窗口");
    });
}
