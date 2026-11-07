//! Native picker workspace. State transitions and file operations stay in the parent.
use super::*;
use crate::workspace_theme::*;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    input::Input,
};
use gpui_kit::{
    Div, FontWeight, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, div, px,
};

fn column() -> Div {
    div().flex().flex_col().min_w_0()
}

fn row() -> Div {
    div().flex().items_center().gap_2().min_w_0()
}

fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(color(MUTED))
        .child(text.into())
}

fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

fn ellipsis(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

fn control(id: impl Into<SharedString>, label: impl Into<SharedString>, disabled: bool) -> Button {
    let id = id.into();
    let label = label.into();
    Button::new(id.clone())
        .accessibility_id(id)
        .accessibility_label(label.clone())
        .label(label)
        .disabled(disabled)
        .tab_stop(!disabled)
}

fn coordinate_column(id: &'static str, label: &'static str, x: String, y: String) -> Div {
    column().flex_1().gap_2().child(muted(label)).child(
        column()
            .id(id)
            .accessibility_id(id)
            .role(gpui_kit::Role::Group)
            .aria_label(format!("{label}：x {x}，y {y}"))
            .gap_1()
            .font_family("Cascadia Mono")
            .text_size(px(14.))
            .child(ellipsis(format!("x  {x}")))
            .child(ellipsis(format!("y  {y}"))),
    )
}

impl Picker {
    fn window_library(&self, cx: &Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let locked = self.locked();
        let timing = self.countdown.is_some();
        let mut entries = column().gap_1();
        let mut visible = 0;
        for target in &self.windows {
            if !query.is_empty()
                && !target.title.to_lowercase().contains(&query)
                && !target.process.to_lowercase().contains(&query)
            {
                continue;
            }
            visible += 1;
            let selected = self
                .target
                .as_ref()
                .is_some_and(|t| t.hwnd == target.hwnd && t.pid == target.pid);
            let title = if target.title.is_empty() {
                "无标题窗口"
            } else {
                &target.title
            };
            let full_name = format!(
                "{}，{}{}",
                title,
                target.process,
                if selected { "，已选择" } else { "" }
            );
            let mut title_row = row().justify_between().child(
                ellipsis(title.to_owned())
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD),
            );
            if selected {
                title_row = title_row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(color(TEAL))
                        .child("已选择"),
                );
            }
            let target = target.clone();
            entries = entries.child(
                control(format!("target-{}", target.hwnd), "", locked || timing)
                    .accessibility_label(full_name.clone())
                    .tooltip(full_name)
                    .selected(selected)
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(color(if selected { TINT } else { PAPER }))
                            .foreground(color(INK))
                            .hover(color(if selected { TINT } else { FOG }))
                            .active(color(TINT)),
                    )
                    .w_full()
                    .h_auto()
                    .p_3()
                    .border_l_2()
                    .border_color(color(if selected { TEAL } else { PAPER }))
                    .child(
                        column()
                            .flex_1()
                            .gap_1()
                            .overflow_hidden()
                            .child(title_row)
                            .child(
                                ellipsis(target.process.clone())
                                    .text_size(px(12.))
                                    .text_color(color(MUTED)),
                            ),
                    )
                    .on_click(cx.listener(move |v, _, w, cx| v.choose(target.clone(), w, cx))),
            );
        }
        if visible == 0 {
            entries = entries.child(
                column()
                    .gap_2()
                    .px_3()
                    .py_5()
                    .child(heading(if self.windows.is_empty() {
                        "没有可选窗口"
                    } else {
                        "没有匹配窗口"
                    }))
                    .child(muted(if self.windows.is_empty() {
                        "打开目标应用后刷新，或使用准星选择窗口。"
                    } else {
                        "尝试其他标题或进程名，或清空搜索。"
                    })),
            );
        }
        column()
            .w(px(280.))
            .flex_shrink_0()
            .min_h_0()
            .bg(color(PAPER))
            .border_r_1()
            .border_color(color(LINE))
            .child(
                column()
                    .gap_3()
                    .p_4()
                    .flex_shrink_0()
                    .child(
                        row().justify_between().child(heading("窗口库")).child(
                            control("refresh-windows", "刷新窗口", locked || timing)
                                .ghost()
                                .small()
                                .on_click(cx.listener(|v, _, w, cx| v.refresh(w, cx))),
                        ),
                    )
                    .child(
                        Input::new(&self.search)
                            .id("picker-search")
                            .accessibility_id("picker-search")
                            .aria_label("搜索窗口标题或进程名")
                            .disabled(locked)
                            .w_full(),
                    )
                    .child(
                        control("crosshair", "准星选窗 · 3 秒", locked || timing)
                            .secondary()
                            .w_full()
                            .tooltip("点击后将光标移到目标窗口，3 秒后自动选择")
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.start_countdown(CountdownKind::Select, cx)
                            })),
                    )
                    .child(muted(format!("{} 个窗口", visible))),
            )
            .child(
                div()
                    .id("picker-targets")
                    .accessibility_id("picker-targets")
                    .role(gpui_kit::Role::Group)
                    .aria_label("可选目标窗口")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pb_3()
                    .child(entries),
            )
            .child(
                muted("先选窗口，再将光标移入目标客户区采点。")
                    .p_4()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(color(LINE)),
            )
    }

    fn target_header(&self, library_visible: bool, cx: &Context<Self>) -> Div {
        let (title, detail, full_name) = match &self.target {
            Some(target) => {
                let title = if target.title.is_empty() {
                    "无标题窗口".to_owned()
                } else {
                    target.title.clone()
                };
                let detail = format!("{}  ·  HWND {}", target.process, target.hwnd);
                let full_name = format!(
                    "目标窗口：{}，{}，HWND {}",
                    title, target.process, target.hwnd
                );
                (title, detail, full_name)
            }
            None => match &self.active {
                Some(target) => (
                    target.title.clone(),
                    format!("{}  ·  已恢复点位，尚未连接窗口", target.process),
                    format!(
                        "已恢复目标：{}，{}，请重新选择窗口后采点",
                        target.title, target.process
                    ),
                ),
                None => (
                    "选择目标窗口".into(),
                    "从窗口库选择，或使用准星定位目标。".into(),
                    "尚未选择目标窗口".into(),
                ),
            },
        };
        column()
            .gap_3()
            .p_4()
            .flex_shrink_0()
            .bg(color(PAPER))
            .border_b_1()
            .border_color(color(LINE))
            .child(
                row()
                    .items_start()
                    .gap_3()
                    .child(
                        column()
                            .id("target-summary")
                            .accessibility_id("target-summary")
                            .role(gpui_kit::Role::Group)
                            .aria_label(full_name)
                            .flex_1()
                            .gap_1()
                            .overflow_hidden()
                            .child(muted("取点工具 / 当前目标"))
                            .child(
                                ellipsis(title)
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(ellipsis(detail).text_size(px(12.)).text_color(color(MUTED))),
                    )
                    .child(
                        control(
                            "toggle-targets",
                            if library_visible {
                                "收起窗口库"
                            } else {
                                "显示窗口库"
                            },
                            self.locked(),
                        )
                        .ghost()
                        .small()
                        .selected(library_visible)
                        .on_click(cx.listener(|v, _, w, cx| {
                            let visible = v
                                .library_open
                                .unwrap_or(w.viewport_size().width >= px(1000.));
                            v.library_open = Some(!visible);
                            cx.notify();
                        })),
                    ),
            )
            .child(
                row()
                    .justify_between()
                    .child(
                        row()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(color(if self.hotkey.is_some() { TEAL } else { MUTED })),
                            )
                            .child(muted(if self.hotkey.is_some() {
                                "全局采点热键已启用"
                            } else {
                                "全局采点热键未启用"
                            })),
                    )
                    .child(
                        control(
                            "hotkey",
                            if self.hotkey.is_some() {
                                "释放 Ctrl+Alt+P"
                            } else {
                                "启用 Ctrl+Alt+P"
                            },
                            self.locked() || self.countdown.is_some(),
                        )
                        .small()
                        .selected(self.hotkey.is_some())
                        .tooltip("启用后，在其他窗口中按 Ctrl+Alt+P 也可采点")
                        .on_click(cx.listener(|v, _, w, cx| v.toggle_hotkey(w, cx))),
                    ),
            )
    }

    fn coordinate_panel(&self, cx: &Context<Self>) -> Div {
        let (screen_x, screen_y, client_x, client_y, nx, ny, hint, valid) = match &self.sample_state
        {
            SampleState::Valid((screen, client, normalized)) => (
                screen.0.to_string(),
                screen.1.to_string(),
                client.0.to_string(),
                client.1.to_string(),
                format!("{:.6}", normalized[0]),
                format!("{:.6}", normalized[1]),
                SharedString::from("光标位于目标客户区内，可以采点。"),
                true,
            ),
            SampleState::Unselected => (
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                SharedString::from("选择目标窗口后显示实时坐标。"),
                false,
            ),
            SampleState::Unavailable(reason) => (
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                reason.clone(),
                false,
            ),
        };
        let mut panel = column()
            .gap_3()
            .p_4()
            .flex_shrink_0()
            .border_b_1()
            .border_color(color(LINE))
            .bg(color(FOG))
            .child(
                row().justify_between().child(heading("实时坐标")).child(
                    control(
                        "capture",
                        "2秒后采点",
                        self.locked() || self.target.is_none() || self.countdown.is_some(),
                    )
                    .primary()
                    .small()
                    .tooltip("点击后将光标移入目标客户区，2 秒后记录位置")
                    .on_click(
                        cx.listener(|v, _, _, cx| v.start_countdown(CountdownKind::Capture, cx)),
                    ),
                ),
            )
            .child(
                row()
                    .id("coordinates")
                    .accessibility_id("coordinates")
                    .role(gpui_kit::Role::Group)
                    .aria_label(self.coordinates.clone())
                    .items_start()
                    .gap_4()
                    .child(coordinate_column(
                        "screen-coordinate",
                        "屏幕坐标 / px",
                        screen_x,
                        screen_y,
                    ))
                    .child(coordinate_column(
                        "client-coordinate",
                        "客户区坐标 / px",
                        client_x,
                        client_y,
                    ))
                    .child(coordinate_column(
                        "normalized-coordinate",
                        "归一化坐标",
                        nx,
                        ny,
                    )),
            );
        if let Some(countdown) = &self.countdown {
            let seconds = countdown.remaining(Instant::now());
            let message = match countdown.kind {
                CountdownKind::Select => format!("{seconds} 秒后选择光标下的窗口，请移动光标。"),
                CountdownKind::Capture => format!("{seconds} 秒后采点，请将光标移入目标客户区。"),
            };
            panel = panel.child(
                row()
                    .id("picker-countdown")
                    .accessibility_id("picker-countdown")
                    .role(gpui_kit::Role::Status)
                    .aria_label(message.clone())
                    .justify_between()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(color(TINT))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(color(TEAL))
                            .child(message),
                    )
                    .child(
                        control("cancel-countdown", "取消倒计时", self.locked())
                            .ghost()
                            .small()
                            .on_click(cx.listener(|v, _, _, cx| v.cancel_countdown(cx))),
                    ),
            );
        } else {
            panel = panel.child(
                ellipsis(hint)
                    .text_size(px(12.))
                    .text_color(color(if valid { TEAL } else { MUTED })),
            );
        }
        panel
    }

    fn captured_points(&self, cx: &Context<Self>) -> Div {
        let mut points = column().gap_2();
        for (index, captured) in self.points.iter().enumerate() {
            let point_id = captured.point.id;
            let name_id = format!("point-name-{point_id}");
            let name = captured.name.read(cx).value().to_string();
            points = points.child(
                row()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .bg(color(PAPER))
                    .border_1()
                    .border_color(color(LINE))
                    .rounded(px(6.))
                    .child(
                        div()
                            .w(px(26.))
                            .flex_shrink_0()
                            .text_center()
                            .text_color(color(TEAL))
                            .font_family("Bahnschrift")
                            .text_size(px(17.))
                            .child(format!("{:02}", index + 1)),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&captured.name)
                                .id(name_id.clone())
                                .accessibility_id(name_id)
                                .aria_label(format!("第 {} 个点位的名称", index + 1))
                                .disabled(self.locked())
                                .w_full(),
                        ),
                    )
                    .child(
                        control(format!("copy-coordinate-{point_id}"), "", self.locked())
                            .ghost()
                            .h_auto()
                            .w(px(120.))
                            .flex_shrink_0()
                            .accessibility_label(format!(
                                "复制第 {} 个点位坐标 {}",
                                index + 1,
                                coordinate_text(captured.point.position)
                            ))
                            .tooltip("复制坐标，可直接粘贴到路线管理")
                            .child(
                                column()
                                    .gap_1()
                                    .font_family("Cascadia Mono")
                                    .text_size(px(12.))
                                    .text_color(color(MUTED))
                                    .child(format!("nx  {:.6}", captured.point.position[0]))
                                    .child(format!("ny  {:.6}", captured.point.position[1])),
                            )
                            .on_click(cx.listener(move |v, _, _, cx| v.copy_point(index, cx))),
                    )
                    .child(
                        row()
                            .flex_shrink_0()
                            .gap_1()
                            .child(
                                control(format!("copy-point-{point_id}"), "复制", self.locked())
                                    .ghost()
                                    .small()
                                    .accessibility_label(format!(
                                        "复制第 {} 个点位 {}",
                                        index + 1,
                                        name
                                    ))
                                    .tooltip("复制坐标，可直接粘贴到路线管理")
                                    .on_click(
                                        cx.listener(move |v, _, _, cx| v.copy_point(index, cx)),
                                    ),
                            )
                            .child(
                                control(
                                    format!("remove-point-{point_id}"),
                                    "删除",
                                    self.locked() || self.countdown.is_some(),
                                )
                                .ghost()
                                .small()
                                .text_color(color(RED))
                                .accessibility_label(format!(
                                    "删除第 {} 个点位 {}",
                                    index + 1,
                                    name
                                ))
                                .on_click(
                                    cx.listener(move |v, _, _, cx| v.remove_point(index, cx)),
                                ),
                            ),
                    ),
            );
        }
        if self.points.is_empty() {
            points = points.child(
                column()
                    .gap_3()
                    .py_5()
                    .px_4()
                    .items_start()
                    .child(heading("还没有采集点位"))
                    .child(muted(
                        "选择目标窗口后，使用「2秒后采点」或 Ctrl+Alt+P 记录位置。",
                    ))
                    .child(muted(
                        "按应用和完整窗口标题自动保存；点击坐标可复制到路线管理。",
                    )),
            );
        }
        column()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                row()
                    .justify_between()
                    .px_4()
                    .py_3()
                    .flex_shrink_0()
                    .child(heading("已采集点位"))
                    .child(
                        row()
                            .gap_3()
                            .child(muted(format!(
                                "{} 个 · {}",
                                self.points.len(),
                                if self.revision == self.saved_revision {
                                    "已自动保存"
                                } else {
                                    "等待自动保存"
                                }
                            )))
                            .child(
                                control(
                                    "clear-picker",
                                    "清空采点",
                                    self.locked()
                                        || self.points.is_empty()
                                        || self.countdown.is_some(),
                                )
                                .ghost()
                                .small()
                                .text_color(color(RED))
                                .on_click(cx.listener(|v, _, w, cx| v.request_clear(w, cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .id("picker-points")
                    .accessibility_id("picker-points")
                    .role(gpui_kit::Role::Group)
                    .aria_label("已采集点位列表")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.point_scroll)
                    .px_4()
                    .pb_4()
                    .child(points),
            )
    }

    fn picker_confirmation(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut choices = row().justify_end().gap_2();
        let (title, message) = match self.confirmation.as_ref().expect("picker confirmation") {
            Confirmation::Clear => {
                choices = choices.child(
                    control("confirm-clear", "确认清空", self.busy)
                        .danger()
                        .track_focus(&self.confirm_primary_focus)
                        .on_click(cx.listener(|v, _, w, cx| v.confirm_clear(w, cx))),
                );
                (
                    "清空当前窗口采点？",
                    format!(
                        "将移除当前窗口的 {} 个点位并自动保存。其他窗口的点位保留，此操作无法撤销。",
                        self.points.len()
                    ),
                )
            }
            Confirmation::Close => {
                choices = choices
                    .child(
                        control("retry-close", "重试并关闭", self.persisting)
                            .primary()
                            .track_focus(&self.confirm_primary_focus)
                            .on_click(cx.listener(|v, _, w, cx| v.retry_and_close(w, cx))),
                    )
                    .child(
                        control("discard-close", "放弃未保存修改并关闭", self.persisting)
                            .danger()
                            .track_focus(&self.confirm_discard_focus)
                            .on_click(cx.listener(|v, _, w, cx| v.discard_and_close(w, cx))),
                    );
                (
                    "点位自动保存失败",
                    "最新点位仍在内存中。可以重试后关闭，或继续编辑并复制需要的坐标。放弃未保存修改将丢失尚未写入磁盘的更改。".into(),
                )
            }
        };
        choices = choices.child(
            control(
                "cancel-confirmation",
                if matches!(self.confirmation, Some(Confirmation::Close)) {
                    "继续编辑"
                } else {
                    "取消"
                },
                self.busy,
            )
            .ghost()
            .track_focus(&self.confirm_focus)
            .on_click(cx.listener(|v, _, w, cx| v.cancel_confirmation(w, cx))),
        );
        div()
            .id("picker-confirmation-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .bg(gpui_kit::rgba(0x173F4A66))
            .flex()
            .items_center()
            .justify_center()
            .p_5()
            .child(
                column()
                    .id("picker-confirmation")
                    .accessibility_id("picker-confirmation")
                    .role(gpui_kit::Role::Dialog)
                    .aria_label(title)
                    .occlude()
                    .w_full()
                    .max_w(px(560.))
                    .gap_4()
                    .p_5()
                    .bg(color(PAPER))
                    .border_1()
                    .border_color(color(LINE))
                    .rounded(px(8.))
                    .child(heading(title))
                    .child(div().text_size(px(14.)).line_height(px(24.)).child(message))
                    .child(choices),
            )
    }

    pub(super) fn render_workspace(
        &self,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let show_library = self
            .library_open
            .unwrap_or(window.viewport_size().width >= px(1000.));
        let workspace = column()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(self.target_header(show_library, cx))
            .child(self.coordinate_panel(cx))
            .child(self.captured_points(cx));
        let mut body = row().items_stretch().gap_0().flex_1().min_h_0().min_w_0();
        if show_library {
            body = body.child(self.window_library(cx));
        }
        body = body.child(workspace);
        let status = self
            .persistence_error
            .clone()
            .unwrap_or_else(|| self.status.clone());
        let status_tooltip = status.clone();
        let status_color = match if self.persistence_error.is_some() {
            FeedbackKind::Error
        } else {
            self.status_kind
        } {
            FeedbackKind::Info => MUTED,
            FeedbackKind::Success => TEAL,
            FeedbackKind::Error => RED,
        };
        let mut root = column()
            .relative()
            .size_full()
            .min_w(px(900.))
            .min_h(px(640.))
            .bg(color(FOG))
            .text_color(color(INK))
            .text_size(px(14.))
            .child(body)
            .child(
                div()
                    .id("picker-status")
                    .accessibility_id("picker-status")
                    .role(gpui_kit::Role::Status)
                    .aria_label(status.clone())
                    .flex_shrink_0()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(color(LINE))
                    .text_size(px(12.))
                    .text_color(color(status_color))
                    .bg(color(if self.status_kind == FeedbackKind::Error {
                        0xF9EBEE
                    } else {
                        FOG
                    }))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(status_tooltip.clone())
                            .build(window, cx)
                    })
                    .child(row().child(ellipsis(status).flex_1()).children(
                        self.persistence_error.as_ref().map(|_| {
                            control(
                                "retry-persistence",
                                if self.file.is_none() {
                                    "重新读取"
                                } else {
                                    "重试"
                                },
                                self.busy
                                    || self.persisting
                                    || self.closing
                                    || self.confirmation.is_some(),
                            )
                            .small()
                            .on_click(cx.listener(|v, _, w, cx| v.retry_persistence(w, cx)))
                        }),
                    )),
            );
        if self.confirmation.is_some() {
            root = root.child(gpui_kit::deferred(self.picker_confirmation(cx)).with_priority(2));
        }
        root.id("picker-root")
            .accessibility_id("picker-root")
            .role(gpui_kit::Role::Group)
            .aria_label("取点工作台")
            .key_context(if self.confirmation.is_some() {
                "PickerConfirmation"
            } else {
                "Picker"
            })
            .on_action(cx.listener(|v, _: &ModalNext, w, cx| {
                if v.confirmation.is_some() {
                    v.cycle_confirmation(false, w, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|v, _: &ModalPrevious, w, cx| {
                if v.confirmation.is_some() {
                    v.cycle_confirmation(true, w, cx);
                } else {
                    cx.propagate();
                }
            }))
            .capture_key_down(cx.listener(|v, event: &gpui_kit::KeyDownEvent, w, cx| {
                if event.keystroke.key == "escape"
                    && (v.confirmation.is_some() || v.countdown.is_some())
                {
                    if v.confirmation.is_some() {
                        v.cancel_confirmation(w, cx);
                    } else {
                        v.cancel_countdown(cx);
                    }
                    w.prevent_default();
                    cx.stop_propagation();
                } else if v.confirmation.is_some() && event.keystroke.key == "tab" {
                    v.cycle_confirmation(event.keystroke.modifiers.shift, w, cx);
                    w.prevent_default();
                    cx.stop_propagation();
                }
            }))
    }
}
