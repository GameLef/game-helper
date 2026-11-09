//! Route workspace presentation. All document mutations stay in the parent actions.
use super::*;
use crate::workspace_theme::*;
use gpui_kit::base::{h_resizable, resizable_panel};
use gpui_kit::component::{
    Selectable, Sizable,
    button::{ButtonCustomVariant, ButtonVariants},
    input::{Editor, Input},
    menu::{DropdownMenu, PopupMenuItem},
    select::Select,
    switch::Switch,
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
fn list_detail(text: impl Into<SharedString>, selected: bool) -> Div {
    muted(text).text_color(color(if selected { LIST_SELECTED_MUTED } else { MUTED }))
}
fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}
fn empty(title: &str, detail: &str) -> Div {
    column()
        .gap_2()
        .p_5()
        .child(
            heading(title.to_owned())
                .id("empty-state-title")
                .role(gpui_kit::Role::Status)
                .aria_label(title.to_owned()),
        )
        .child(muted(detail.to_owned()))
}
fn op_name(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Click => "点击",
        OpKind::Key => "按下 / 松开",
        OpKind::Press => "按住按键",
        OpKind::Wheel => "滚轮",
        OpKind::Drag => "拖动",
        OpKind::Wait => "等待",
        OpKind::DragNative => "原生拖动",
    }
}
fn parameter_name(key: &str) -> &str {
    match key {
        "pos" => "点击坐标 [x, y]",
        "button" => "鼠标按键",
        "clicks" => "点击次数",
        "key" => "按键",
        "up" => "按键动作",
        "ms" => "时长（ms）",
        "delta" => "滚动量",
        "from" => "起点 [x, y]",
        "to" => "终点 [x, y]",
        "duration" => "拖动时长（ms）",
        "speed" => "移动速度（0～100）",
        "pre_drag_distance" => "预拖动距离（物理像素）",
        "pre_drag_speed" => "预拖动速度（1～100）",
        "pre_drag_delay" => "预拖动后等待（ms）",
        "jitter" => "随机偏移",
        _ => key,
    }
}

// Defer the edit until PopupMenu has dismissed and restored its trigger;
// otherwise dismiss would steal focus from the newly inserted name input.
fn enqueue_operation_add(
    manager: &gpui_kit::WeakEntity<Manager>,
    kind: OpKind,
    position: Option<usize>,
    epoch: u64,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = manager.update(cx, |_, cx| {
        cx.defer_in(window, move |view, window, cx| {
            if view.drag_epoch == epoch
                && !view.busy
                && !view.opening_route
                && !view.ui_closing
                && view.confirmation.is_none()
            {
                view.action(Action::AddOp(kind, position), window, cx);
            }
        });
    });
}

impl Manager {
    fn field(&self, key: &'static str, label: &'static str) -> Div {
        column()
            .gap_1()
            .flex_1()
            .min_w(px(140.))
            .child(muted(label))
            .child(
                Input::new(&self.fields[key])
                    .accessibility_id(key)
                    .aria_label(label)
                    .disabled(self.controls_busy() || self.confirmation.is_some())
                    .w_full(),
            )
    }

    fn operation_add_menu(
        &self,
        id: String,
        label: &'static str,
        position: Option<usize>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let manager = cx.weak_entity();
        let epoch = self.drag_epoch;
        Button::new(id.clone())
            .accessibility_id(id)
            .label(label)
            .small()
            .h(px(28.))
            .disabled(
                self.busy || self.opening_route || self.ui_closing || self.confirmation.is_some(),
            )
            .dropdown_menu(move |mut menu, _, popup_cx| {
                let popup = popup_cx.weak_entity();
                for kind in OpKind::ALL {
                    let button_manager = manager.clone();
                    let item_manager = manager.clone();
                    let popup = popup.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            let manager = button_manager.clone();
                            let popup = popup.clone();
                            Button::new(format!("operation-add-{}", kind.label()))
                                .accessibility_id(format!("operation-add-{}", kind.label()))
                                .accessibility_label(op_name(kind))
                                .label(op_name(kind))
                                .ghost()
                                .small()
                                .h(px(28.))
                                .w_full()
                                .justify_start()
                                .on_click(move |_, window, cx| {
                                    // A direct UI Automation invocation does not
                                    // bubble to PopupMenu's outer menu item.
                                    let _ =
                                        popup.update(cx, |_, cx| cx.emit(gpui_kit::DismissEvent));
                                    enqueue_operation_add(
                                        &manager, kind, position, epoch, window, cx,
                                    );
                                    cx.stop_propagation();
                                })
                        })
                        .on_click(move |_, window, cx| {
                            enqueue_operation_add(&item_manager, kind, position, epoch, window, cx);
                        }),
                    );
                }
                menu.min_w(px(150.))
            })
    }

    fn ui_persistence_warning(&self, cx: &Context<Self>) -> Option<Div> {
        let message = self.ui_error.as_ref()?;
        Some(
            row()
                .gap_2()
                .px_3()
                .py_1()
                .flex_shrink_0()
                .bg(color(ERROR_BG))
                .child(
                    div()
                        .id("manager-ui-warning")
                        .accessibility_id("manager-ui-warning")
                        .role(gpui_kit::Role::Status)
                        .aria_label(message.clone())
                        .text_size(px(12.))
                        .text_color(color(DANGER))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(message.clone()),
                )
                .child(
                    Button::new("manager-ui-retry")
                        .accessibility_id("manager-ui-retry")
                        .label("重试")
                        .small()
                        .h(px(28.))
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.retry_ui_persistence(window, cx);
                        })),
                )
                .children(self.ui_closing.then(|| {
                    Button::new("manager-ui-close-anyway")
                        .accessibility_id("manager-ui-close-anyway")
                        .label("直接关闭")
                        .small()
                        .h(px(28.))
                        .on_click(|_, window, _| window.remove_window())
                })),
        )
    }

    fn route_library(&self, cx: &Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let mut entries = column().gap(px(2.));
        let mut visible = 0;
        for (i, entry) in self.routes.iter().enumerate() {
            if !query.is_empty()
                && !entry.name.to_lowercase().contains(&query)
                && !entry.stem.to_lowercase().contains(&query)
            {
                continue;
            }
            visible += 1;
            let selected = self.document.as_ref().is_some_and(|d| d.path == entry.path);
            let route_binding = self
                .config
                .as_ref()
                .and_then(|c| c.value.binding_for(&entry.stem));
            let binding = route_binding
                .map(|b| game_helper_core::config::hotkey_display(&b.hotkey))
                .unwrap_or_else(|| "未绑定".into());
            let enabled = route_binding.is_some_and(|b| b.enabled);
            let file = format!("{}.yaml", entry.stem);
            let detail = format!(
                "{}\n{}\n{} 个点位 · {}{}",
                entry.name,
                file,
                entry.points,
                binding,
                entry
                    .error
                    .as_ref()
                    .map(|e| format!("\n格式有误，打开修复：{e}"))
                    .unwrap_or_default(),
            );
            let tooltip = detail.clone();
            let summary = if entry.error.is_some() {
                list_detail("格式有误", selected).text_color(color(DANGER))
            } else {
                list_detail(format!("{} 点", entry.points), selected)
            };
            let title = column()
                .w_full()
                .gap(px(2.))
                .child(
                    row()
                        .w_full()
                        .line_height(px(20.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(entry.name.clone()),
                        )
                        .child(summary.flex_shrink_0().whitespace_nowrap()),
                )
                .child(
                    row()
                        .w_full()
                        .line_height(px(16.))
                        .child(
                            list_detail(file, selected)
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap(),
                        )
                        .child(
                            list_detail(binding.to_owned(), selected)
                                .flex_shrink_0()
                                .max_w(px(88.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap(),
                        ),
                );
            let variant = ButtonCustomVariant::new(cx)
                .color(color(SURFACE))
                .foreground(color(TEXT))
                .hover(color(CANVAS))
                .active(color(LIST_SELECTED));
            let stem = entry.stem.clone();
            let switch_disabled = self.controls_busy()
                || self.config.is_none()
                || self.confirmation.is_some()
                || self.capture.is_some()
                || self.daemon.busy
                || self.daemon_error.is_some()
                || route_binding.is_none()
                || (entry.error.is_some() && !enabled);
            entries = entries.child(
                row()
                    .gap(px(4.))
                    .w_full()
                    .child(
                        self.button(
                            ("route", i),
                            "",
                            Action::Open(entry.path.clone()),
                            false,
                            cx,
                        )
                        .accessibility_id(format!("route-{}", entry.stem))
                        .accessibility_label(format!(
                            "{} {}",
                            entry.stem,
                            detail.replace('\n', "，")
                        ))
                        .tooltip(tooltip)
                        .custom(variant)
                        .selected(selected)
                        .w_full()
                        .h(px(48.))
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .border_l_2()
                        .border_color(color(if selected { EMPHASIS } else { SURFACE }))
                        .flex_1()
                        .min_w_0()
                        .child(title),
                    )
                    .child(
                        div()
                            .id(format!("route-hotkey-{}", entry.stem))
                            .accessibility_id(format!("route-hotkey-{}", entry.stem))
                            .role(gpui_kit::Role::Group)
                            .aria_label(format!("{} 快捷键设置", entry.name))
                            .flex_shrink_0()
                            .px_1()
                            .child(
                                Switch::new(("route-switch", i))
                                    .small()
                                    .checked(enabled)
                                    .disabled(switch_disabled)
                                    .accessibility_label(format!("{} 快捷键开关", entry.name))
                                    .tooltip(if route_binding.is_none() {
                                        "请先设置快捷键"
                                    } else if entry.error.is_some() && !enabled {
                                        "请先修复并保存路线"
                                    } else if enabled {
                                        "关闭此路线的快捷键"
                                    } else {
                                        "开启此快捷键，并关闭其他同键路线"
                                    })
                                    .on_change(cx.listener(
                                        move |view, checked: &bool, window, cx| {
                                            view.action(
                                                Action::ToggleHotkey(stem.clone(), *checked),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                            ),
                    ),
            );
        }
        if visible == 0 {
            entries = entries.child(if self.routes.is_empty() {
                empty("路线库还是空的", "新建路线，或从 YAML 文件导入。")
            } else {
                empty("没有匹配的路线", "试试其他名称，或清空搜索。")
            });
        }
        let mut tools = column()
            .flex_shrink_0()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                row()
                    .justify_between()
                    .child(heading("路线库"))
                    .child(muted(format!("{} 条", self.routes.len()))),
            )
            .child(Styled::h(
                Input::new(&self.search)
                    .small()
                    .disabled(self.controls_busy() || self.confirmation.is_some())
                    .w_full(),
                px(28.),
            ))
            .child(
                row()
                    .flex_wrap()
                    .child(
                        self.button("new-toggle", "新建路线", Action::ToggleNew, false, cx)
                            .secondary()
                            .small()
                            .h(px(28.)),
                    )
                    .child(
                        self.button("import", "导入", Action::Import, false, cx)
                            .ghost()
                            .small()
                            .h(px(28.)),
                    )
                    .child(
                        self.button("refresh", "刷新", Action::Refresh, false, cx)
                            .ghost()
                            .small()
                            .h(px(28.)),
                    ),
            );
        if self.new_open {
            tools = tools.child(
                column()
                    .gap_2()
                    .p_3()
                    .bg(color(CANVAS))
                    .rounded(px(6.))
                    .child(self.field("stem", "文件名（不含扩展名）"))
                    .child(
                        self.button("new", "创建草稿", Action::New, false, cx)
                            .primary()
                            .border_1()
                            .border_color(color(EMPHASIS))
                            .small(),
                    ),
            );
        }
        column()
            .size_full()
            .bg(color(SURFACE))
            .border_r_1()
            .border_color(color(BORDER))
            .child(tools)
            .child(
                div()
                    .id("route-list")
                    .accessibility_id("route-list")
                    .role(gpui_kit::Role::Group)
                    .aria_label("路线列表")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pb_3()
                    .child(entries),
            )
            .child(
                muted("路线仅在点击保存后写入文件")
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(color(BORDER)),
            )
    }

    fn workspace_header(&self, source_mode: bool, cx: &Context<Self>) -> impl IntoElement {
        let title = self
            .document
            .as_ref()
            .map(|_| self.text("name", cx))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "未命名路线".into());
        let file = self
            .document
            .as_ref()
            .map(|d| {
                d.path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_default();
        let saved = if self.dirty() {
            "有未保存修改"
        } else {
            "已保存"
        };
        let title_tooltip = title.clone();
        let file_tooltip = file.clone();
        let modes = row()
            .gap_1()
            .flex_shrink_0()
            .child(
                self.button(
                    "form",
                    "点位编辑",
                    Action::Switch(EditMode::Form),
                    false,
                    cx,
                )
                .ghost()
                .small()
                .h(px(28.))
                .selected(!source_mode),
            )
            .child(
                self.button(
                    "yaml",
                    "YAML 源码",
                    Action::Switch(EditMode::Source),
                    false,
                    cx,
                )
                .ghost()
                .small()
                .h(px(28.))
                .selected(source_mode),
            );
        let commands = row()
            .gap_1()
            .flex_shrink_0()
            .child(
                self.button(
                    "properties-toggle",
                    "路线属性",
                    Action::ToggleProperties,
                    source_mode,
                    cx,
                )
                .ghost()
                .small()
                .h(px(28.))
                .selected(self.properties_open && !source_mode),
            )
            .child(
                self.button("files-toggle", "文件操作", Action::ToggleFiles, false, cx)
                    .ghost()
                    .small()
                    .h(px(28.))
                    .selected(self.files_open),
            )
            .child(
                self.button("save", "保存", Action::Save(false, None), false, cx)
                    .primary()
                    .small()
                    .h(px(28.))
                    .border_1()
                    .border_color(color(EMPHASIS)),
            );
        let mut header = column()
            .id("workspace-header")
            .accessibility_id("workspace-header")
            .role(gpui_kit::Role::Group)
            .aria_label("路线工具栏")
            .flex_shrink_0()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(color(BORDER))
            .bg(color(SURFACE))
            .child(
                row()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        column()
                            .flex_1()
                            .min_w(px(160.))
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("route-context-name")
                                    .text_size(px(17.))
                                    .line_height(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .tooltip(move |w, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            title_tooltip.clone(),
                                        )
                                        .build(w, cx)
                                    })
                                    .child(title),
                            )
                            .child(
                                row()
                                    .gap(px(8.))
                                    .line_height(px(16.))
                                    .child(
                                        muted(file)
                                            .id("route-context-file")
                                            .accessibility_id("route-context-file")
                                            .role(gpui_kit::Role::Label)
                                            .aria_label(file_tooltip.clone())
                                            .flex_shrink_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .tooltip(move |w, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(
                                                    file_tooltip.clone(),
                                                )
                                                .build(w, cx)
                                            }),
                                    )
                                    .child(
                                        div()
                                            .id("route-save-state")
                                            .accessibility_id("route-save-state")
                                            .role(gpui_kit::Role::Label)
                                            .aria_label(saved)
                                            .flex_shrink_0()
                                            .text_size(px(12.))
                                            .text_color(color(if self.dirty() {
                                                WARNING
                                            } else {
                                                MUTED
                                            }))
                                            .child(saved),
                                    ),
                            ),
                    )
                    .child(row().gap_2().flex_wrap().child(modes).child(commands)),
            );
        if self.files_open {
            header = header.child(
                row()
                    .id("file-operations")
                    .accessibility_id("file-operations")
                    .role(gpui_kit::Role::Group)
                    .aria_label("文件操作")
                    .flex_wrap()
                    .gap_2()
                    .p_2()
                    .bg(color(CANVAS))
                    .rounded(px(4.))
                    .child(
                        self.button("export", "导出", Action::Export, false, cx)
                            .small()
                            .h(px(32.))
                            .border_1()
                            .border_color(color(CONTROL_BORDER)),
                    )
                    .child(
                        self.button("reveal", "定位文件", Action::Reveal, false, cx)
                            .small()
                            .h(px(32.))
                            .border_1()
                            .border_color(color(CONTROL_BORDER)),
                    )
                    .child(
                        self.button(
                            "delete",
                            "删除路线",
                            Action::Delete(false),
                            self.daemon.busy,
                            cx,
                        )
                        .small()
                        .h(px(32.))
                        .border_1()
                        .border_color(color(CONTROL_BORDER))
                        .text_color(color(DANGER)),
                    ),
            );
        }
        if self.properties_open && !source_mode {
            header = header.child(
                column()
                    .id("route-properties")
                    .accessibility_id("route-properties")
                    .role(gpui_kit::Role::Group)
                    .aria_label("路线属性")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .flex_shrink_0()
                    .gap_2()
                    .p_2()
                    .bg(color(CANVAS))
                    .rounded(px(4.))
                    .child(
                        row()
                            .flex_wrap()
                            .items_start()
                            .gap_3()
                            .child(self.field("name", "路线名称"))
                            .child(self.field("note", "路线备注")),
                    )
                    .child(
                        row()
                            .flex_wrap()
                            .items_start()
                            .gap_3()
                            .child(self.field("process", "目标进程，如 notepad.exe"))
                            .child(self.field("class", "窗口类（可选）"))
                            .child(self.field("title", "窗口标题（可选）")),
                    )
                    .child(muted(
                        "至少填写一个目标条件；多个条件同时填写时需全部匹配。",
                    )),
            );
        }
        header
    }

    fn point_sequence(&self, cx: &Context<Self>) -> impl IntoElement {
        let points = &self
            .document
            .as_ref()
            .expect("document workspace")
            .route()
            .points;
        let mut sequence = column()
            .id("points")
            .accessibility_id("points")
            .role(gpui_kit::Role::Group)
            .aria_label("点位列表")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.point_scroll)
            .gap(px(2.))
            .px_2()
            .on_drop(cx.listener(
                |view, payload: &super::point_interaction::PointDragPayload, window, cx| {
                    view.finish_point_drag(payload, window, cx);
                },
            ));
        let number_width = px(28. + points.len().to_string().len().saturating_sub(3) as f32 * 8.);
        for (i, point) in points.iter().enumerate() {
            let selected = self.selected == Some(i);
            let name = if selected {
                self.text("point_name", cx)
            } else {
                point.name.clone()
            };
            let detail = format!(
                "第 {} 点 {}，{} 个动作",
                i + 1,
                name,
                point.operations.len()
            );
            let tooltip = detail.clone();
            let handle = self.point_drag_handle(i, &name, cx);
            let mut point_row = row()
                .id(format!("point-row-{i}"))
                .accessibility_id(format!("point-row-{i}"))
                .role(gpui_kit::Role::Group)
                .aria_label(format!("点位 {} 排序与选择", i + 1))
                .relative()
                .w_full()
                .h(px(36.))
                .flex_shrink_0()
                .gap_0()
                .child(handle)
                .child(
                    self.button(("point", i), "", Action::Select(i), false, cx)
                        .accessibility_id(format!("point-{i}"))
                        .accessibility_label(detail)
                        .tooltip(tooltip)
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(color(SURFACE))
                                .foreground(color(TEXT))
                                .hover(color(CANVAS))
                                .active(color(LIST_SELECTED)),
                        )
                        .selected(selected)
                        .flex_1()
                        .min_w_0()
                        .h(px(36.))
                        .flex_shrink_0()
                        .border_l_2()
                        .border_color(color(if selected { EMPHASIS } else { SURFACE }))
                        .px_2()
                        .py_1()
                        .child(
                            row()
                                .w_full()
                                .gap_2()
                                .line_height(px(20.))
                                .child(
                                    div()
                                        .w(number_width)
                                        .flex_shrink_0()
                                        .text_right()
                                        .text_color(color(if selected { EMPHASIS } else { MUTED }))
                                        .font_family("Bahnschrift")
                                        .text_size(px(13.))
                                        .child((i + 1).to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(name),
                                )
                                .child(
                                    list_detail(
                                        format!("{} 动作", point.operations.len()),
                                        selected,
                                    )
                                    .flex_shrink_0()
                                    .whitespace_nowrap(),
                                ),
                        ),
                );
            if let Some(indicator) = self.point_drop_indicator(i) {
                point_row = point_row.child(
                    indicator
                        .top_0()
                        .id("point-drop-indicator")
                        .accessibility_id("point-drop-indicator")
                        .role(gpui_kit::Role::Status)
                        .aria_label(format!("插入到点位 {} 前", i + 1)),
                );
            }
            if i + 1 == points.len()
                && let Some(indicator) = self.point_drop_indicator(i + 1)
            {
                point_row = point_row.child(
                    indicator
                        .bottom_0()
                        .id("point-drop-indicator")
                        .accessibility_id("point-drop-indicator")
                        .role(gpui_kit::Role::Status)
                        .aria_label("插入到点位列表末尾"),
                );
            }
            sequence = sequence.child(point_row);
        }
        if points.is_empty() {
            sequence = sequence.child(empty("还没有点位", "新增点位开始编排，或使用取点工具。"));
        }
        let index = self.selected;
        column()
            .size_full()
            .bg(color(SURFACE))
            .border_r_1()
            .border_color(color(BORDER))
            .child(
                row()
                    .flex_shrink_0()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .child(heading("点位序列").flex_shrink_0())
                    .child(muted(format!("{}", points.len())).flex_1())
                    .child(
                        self.button("add-point", "新增点位", Action::AddPoint, false, cx)
                            .secondary()
                            .small()
                            .h(px(28.)),
                    ),
            )
            .child(sequence)
            .child(
                row()
                    .flex_wrap()
                    .gap_1()
                    .flex_shrink_0()
                    .px_2()
                    .py_2()
                    .border_t_1()
                    .border_color(color(BORDER))
                    .child(
                        self.button(
                            "up",
                            "上移",
                            Action::Move(-1),
                            index.is_none_or(|i| i == 0),
                            cx,
                        )
                        .ghost()
                        .small()
                        .h(px(28.)),
                    )
                    .child(
                        self.button(
                            "down",
                            "下移",
                            Action::Move(1),
                            index.is_none_or(|i| i + 1 >= points.len()),
                            cx,
                        )
                        .ghost()
                        .small()
                        .h(px(28.)),
                    )
                    .child(
                        self.button("copy-point", "复制", Action::CopyPoint, index.is_none(), cx)
                            .accessibility_id("copy-point")
                            .ghost()
                            .small()
                            .h(px(28.)),
                    )
                    .child(
                        self.button(
                            "remove-point",
                            "删除",
                            Action::RemovePoint,
                            index.is_none(),
                            cx,
                        )
                        .ghost()
                        .small()
                        .h(px(28.))
                        .text_color(color(DANGER)),
                    ),
            )
    }

    fn operation_panel(
        &self,
        index: usize,
        operation: &OperationFields,
        cx: &Context<Self>,
    ) -> Div {
        let name = operation.name.read(cx).value().to_string();
        let title = if name.trim().is_empty() {
            op_name(operation.kind).to_owned()
        } else {
            name
        };
        let tooltip = format!(
            "动作 {}：{}（{}）",
            index + 1,
            title,
            op_name(operation.kind)
        );
        let mut panel = column()
            .relative()
            .gap_3()
            .py_3()
            .flex_shrink_0()
            .border_b_1()
            .border_color(color(BORDER))
            .child(
                row()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        row()
                            .flex_1()
                            .min_w(px(150.))
                            .gap_2()
                            .child(self.operation_drag_handle(index, operation, cx))
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_family("Bahnschrift")
                                    .text_color(color(EMPHASIS))
                                    .child(format!("{:02}", index + 1)),
                            )
                            .child(
                                column()
                                    .flex_1()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .id(format!("operation-summary-{index}"))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .tooltip(move |w, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(
                                                    tooltip.clone(),
                                                )
                                                .build(w, cx)
                                            })
                                            .child(title),
                                    )
                                    .child(muted(op_name(operation.kind))),
                            ),
                    )
                    .child(
                        row()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                div()
                                    .id(format!("operation-expansion-{index}"))
                                    .role(gpui_kit::Role::Group)
                                    .aria_label(format!("动作 {} 参数", index + 1))
                                    .aria_expanded(operation.expanded)
                                    .child(
                                        self.button(
                                            ("op-toggle", index),
                                            if operation.expanded {
                                                "收起"
                                            } else {
                                                "展开"
                                            },
                                            Action::ToggleOp(index),
                                            false,
                                            cx,
                                        )
                                        .accessibility_id(format!("operation-toggle-{index}"))
                                        .accessibility_label(format!(
                                            "{}动作 {}",
                                            if operation.expanded {
                                                "收起"
                                            } else {
                                                "展开"
                                            },
                                            index + 1
                                        ))
                                        .track_focus(&operation.toggle_focus)
                                        .ghost()
                                        .small(),
                                    ),
                            )
                            .child(self.operation_add_menu(
                                format!("operation-insert-{index}"),
                                "在下方添加 ▾",
                                Some(index + 1),
                                cx,
                            ))
                            .child(
                                self.button(
                                    ("op-delete", index),
                                    "删除",
                                    Action::RemoveOp(index),
                                    false,
                                    cx,
                                )
                                .ghost()
                                .small()
                                .text_color(color(DANGER))
                                .accessibility_label(format!("删除动作 {}", index + 1)),
                            ),
                    ),
            );
        if operation.expanded {
            panel = panel.child(
                column().gap_1().child(muted("动作名称（可选）")).child(
                    Input::new(&operation.name)
                        .accessibility_id(format!("operation-{index}-name"))
                        .aria_label(format!("动作 {} 名称", index + 1))
                        .disabled(self.controls_busy() || self.confirmation.is_some())
                        .w_full(),
                ),
            );
            let mut fields = row().items_start().flex_wrap().gap_3();
            for (key, input) in &operation.fields {
                let id = format!("operation-{index}-{key}");
                let label = format!("动作 {} {}", index + 1, parameter_name(key));
                let control = match input {
                    OperationInput::Text(input, _) => Input::new(input)
                        .accessibility_id(id)
                        .aria_label(label)
                        .disabled(self.controls_busy() || self.confirmation.is_some())
                        .w_full()
                        .into_any_element(),
                    OperationInput::Choice(state) => div()
                        .id(format!("operation-choice-{index}-{key}"))
                        .role(gpui_kit::Role::Group)
                        .accessibility_id(id.clone())
                        .w_full()
                        .child(
                            Select::new(state)
                                .id(id)
                                .accessibility_label(label)
                                .disabled(self.controls_busy() || self.confirmation.is_some())
                                .w_full(),
                        )
                        .into_any_element(),
                };
                fields = fields.child(
                    column()
                        .gap_1()
                        .flex_1()
                        .min_w(px(150.))
                        .max_w(px(340.))
                        .child(muted(parameter_name(key).to_owned()))
                        .child(control),
                );
            }
            panel = panel.child(fields);
            if operation.kind == OpKind::DragNative {
                panel = panel
                    .child(muted("速度为整数：0 瞬移，数值越大越慢；非毫秒时长。"))
                    .child(muted(
                        "预拖动默认 16 物理像素、速度 10、移动后等待 80 ms；距离为 0 恢复直接拖动。",
                    ))
                    .child(muted(
                        "预拖动沿起终点方向先移动一小段，全程按住左键；无法检测游戏实际拖动阈值。",
                    ))
                    .child(muted("急停会在本次原生移动返回后停止后续动作。"));
            }
        }
        if let Some(indicator) = self.operation_drop_indicator(index) {
            panel = panel.child(
                indicator
                    .top_0()
                    .id("operation-drop-indicator")
                    .accessibility_id("operation-drop-indicator")
                    .role(gpui_kit::Role::Status)
                    .aria_label(format!("插入到动作 {} 前", index + 1)),
            );
        }
        if index + 1 == self.operation_fields.len()
            && let Some(indicator) = self.operation_drop_indicator(index + 1)
        {
            panel = panel.child(
                indicator
                    .bottom_0()
                    .id("operation-drop-indicator")
                    .accessibility_id("operation-drop-indicator")
                    .role(gpui_kit::Role::Status)
                    .aria_label("插入到动作列表末尾"),
            );
        }
        panel
    }

    fn point_inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(index) = self.selected else {
            return column().size_full().bg(color(SURFACE)).child(empty(
                "选择一个点位",
                "在左侧新增或选择点位，在这里编辑名称、备注与动作。",
            ));
        };
        let point_name = self.text("point_name", cx);
        let title = if point_name.trim().is_empty() {
            format!("第 {} 点", index + 1)
        } else {
            point_name
        };
        let title_tooltip = title.clone();
        let any_expanded = self.operation_fields.iter().any(|op| op.expanded);
        let toolbar = column()
            .gap(px(3.))
            .px_4()
            .py_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(color(BORDER))
            .child(
                row()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        row()
                            .flex_1()
                            .min_w(px(110.))
                            .gap_2()
                            .child(
                                div()
                                    .id("point-context-name")
                                    .flex_1()
                                    .min_w_0()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .tooltip(move |w, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            title_tooltip.clone(),
                                        )
                                        .build(w, cx)
                                    })
                                    .child(title),
                            )
                            .child(
                                muted(format!("{} 个动作", self.operation_fields.len()))
                                    .flex_shrink_0(),
                            ),
                    )
                    .child(
                        div()
                            .id("point-info-expansion")
                            .role(gpui_kit::Role::Group)
                            .aria_label("点位基础信息")
                            .aria_expanded(self.point_info_open)
                            .child(
                                self.button(
                                    "point-info-toggle",
                                    "点位信息",
                                    Action::TogglePointInfo,
                                    false,
                                    cx,
                                )
                                .accessibility_id("point-info-toggle")
                                .track_focus(&self.point_info_focus)
                                .toggled(self.point_info_open)
                                .small()
                                .h(px(28.))
                                .ghost(),
                            ),
                    )
                    .child(
                        self.button(
                            "operation-toggle-all",
                            if any_expanded {
                                "全部收起"
                            } else {
                                "全部展开"
                            },
                            Action::ToggleAllOps,
                            self.operation_fields.is_empty(),
                            cx,
                        )
                        .accessibility_id("operation-toggle-all")
                        .track_focus(&self.all_ops_focus)
                        .small()
                        .h(px(28.))
                        .ghost(),
                    )
                    .child(self.operation_add_menu(
                        "operation-add-menu".to_owned(),
                        "添加动作 ▾",
                        None,
                        cx,
                    )),
            );
        let mut list = column()
            .id("operation-list")
            .accessibility_id("operation-list")
            .role(gpui_kit::Role::Group)
            .aria_label("动作编排列表")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.operation_scroll)
            .px_4()
            .pb_4()
            .on_drop(cx.listener(
                |view, payload: &super::interaction::OperationDragPayload, window, cx| {
                    view.finish_operation_drag(payload, window, cx);
                },
            ));
        if self.point_info_open {
            list = list.child(
                column()
                    .gap_3()
                    .py_3()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(color(BORDER))
                    .child(self.field("point_name", "点位名称"))
                    .child(self.field("point_note", "点位备注")),
            );
        }
        if self.operation_fields.is_empty() {
            list = list.child(empty(
                "暂无动作",
                "添加第一个动作开始编排；拖动手柄可调整顺序，时间单位为毫秒。",
            ));
        }
        for (i, op) in self.operation_fields.iter().enumerate() {
            list = list.child(
                column()
                    .id(format!("operation-panel-{i}"))
                    .accessibility_id(format!("operation-panel-{i}"))
                    .role(gpui_kit::Role::Group)
                    .aria_label(format!("动作 {}", i + 1))
                    .flex_shrink_0()
                    .child(self.operation_panel(i, op, cx)),
            );
        }
        column()
            .relative()
            .size_full()
            .min_h_0()
            .bg(color(SURFACE))
            .child(
                div()
                    .id("operation-toolbar")
                    .accessibility_id("operation-toolbar")
                    .role(gpui_kit::Role::Group)
                    .aria_label("动作编排工具栏")
                    .flex_shrink_0()
                    .child(toolbar),
            )
            .child(list)
            .children(self.operation_move_dialog(cx))
    }

    fn document_workspace(&self, cx: &Context<Self>) -> Div {
        let source_mode = self
            .document
            .as_ref()
            .is_some_and(|d| d.mode() == EditMode::Source);
        let body = if source_mode {
            div()
                .size_full()
                .p_4()
                .child(
                    Editor::new(&self.source)
                        .aria_label("路线 YAML 源码")
                        .disabled(self.controls_busy() || self.confirmation.is_some())
                        .size_full(),
                )
                .into_any_element()
        } else {
            h_resizable("point-panels")
                .child(
                    resizable_panel()
                        .size(px(250.))
                        .size_range(px(210.)..px(400.))
                        .child(self.point_sequence(cx)),
                )
                .child(resizable_panel().child(self.point_inspector(cx)))
                .into_any_element()
        };
        column()
            .size_full()
            .child(self.workspace_header(source_mode, cx))
            .child(div().flex_1().min_h_0().min_w_0().child(body))
    }

    fn welcome(&self, cx: &Context<Self>) -> Div {
        column().size_full().justify_center().px_5().bg(color(SURFACE))
            .child(column().max_w(px(520.)).gap_4().mx_auto()
                .child(gpui_kit::component::Icon::default().data(&format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><path d="M14 47V22a8 8 0 0 1 8-8h20a8 8 0 0 1 8 8v20" fill="none" stroke="#{EMPHASIS:06X}" stroke-width="3"/><circle cx="14" cy="47" r="6" fill="#{PRIMARY:06X}" stroke="#{EMPHASIS:06X}" stroke-width="2"/><circle cx="32" cy="14" r="5" fill="#{SELECTED:06X}" stroke="#{EMPHASIS:06X}" stroke-width="3"/><circle cx="50" cy="47" r="6" fill="#{SURFACE:06X}" stroke="#{EMPHASIS:06X}" stroke-width="3"/></svg>"##).into_bytes()).size(px(64.)))
                .child(div().text_size(px(28.)).font_weight(FontWeight::SEMIBOLD).child("把操作编排成路线"))
                .child(div().text_size(px(15.)).text_color(color(MUTED)).line_height(px(26.)).child("选择一条路线继续编辑，或从取点开始。点位决定操作位置，动作决定每一步做什么。"))
                .child(row().flex_wrap()
                    .child(self.button("welcome-new", "新建路线", Action::ToggleNew, false, cx).primary().border_1().border_color(color(EMPHASIS)))
                    .child(self.button("welcome-picker", "打开取点工具", Action::Picker, false, cx)))
                .child(muted("支持点击、按键、按住、滚轮、拖动和等待。")))
    }

    fn service_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let state = if self.daemon_error.is_some() {
            "状态未知"
        } else if self.daemon.busy {
            "执行中"
        } else if self.daemon.running {
            "已就绪"
        } else {
            "未启动"
        };
        let mut header = column()
            .id("service-header")
            .accessibility_id("service-header")
            .role(gpui_kit::Role::Group)
            .aria_label("执行服务与工具")
            .flex_shrink_0()
            .border_b_1()
            .border_color(color(BORDER))
            .bg(color(CANVAS))
            .child(
                row()
                    .flex_wrap()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .child(
                        self.button("library-toggle", "路线库", Action::ToggleLibrary, false, cx)
                            .ghost()
                            .small()
                            .h(px(28.)),
                    )
                    .child(div().flex_1())
                    .child(
                        row()
                            .gap_2()
                            .child(div().size(px(7.)).rounded_full().bg(color(
                                if self.daemon_error.is_some() {
                                    DANGER
                                } else if self.daemon.running {
                                    EMPHASIS
                                } else {
                                    MUTED
                                },
                            )))
                            .child(muted(format!("执行服务{state}"))),
                    )
                    .child(
                        self.button(
                            "service-toggle",
                            "服务控制",
                            Action::ToggleService,
                            false,
                            cx,
                        )
                        .ghost()
                        .small()
                        .h(px(28.))
                        .selected(self.service_open),
                    )
                    .child(
                        self.button("picker", "取点工具", Action::Picker, false, cx)
                            .small()
                            .h(px(28.)),
                    ),
            );
        if self.service_open {
            let details = if let Some(error) = &self.daemon_error {
                format!("状态未知：{error}")
            } else {
                format!(
                    "服务进程 PID {}，已注册 {} 个热键。",
                    self.daemon.pid, self.daemon.hotkeys
                )
            };
            header = header.child(
                column()
                    .gap_2()
                    .px_4()
                    .pb_3()
                    .child(
                        row()
                            .flex_wrap()
                            .child(
                                self.button(
                                    "start",
                                    "启动服务",
                                    Action::Start,
                                    self.daemon.running,
                                    cx,
                                )
                                .secondary()
                                .small(),
                            )
                            .child(
                                self.button(
                                    "stop",
                                    "停止服务",
                                    Action::Stop,
                                    !self.daemon.running,
                                    cx,
                                )
                                .small(),
                            )
                            .child(
                                self.button(
                                    "reload",
                                    "重载路线",
                                    Action::Reload,
                                    !self.daemon.running || self.daemon.busy,
                                    cx,
                                )
                                .ghost()
                                .small(),
                            )
                            .child(
                                self.button(
                                    "auto",
                                    format!(
                                        "随管理器启动：{}",
                                        if self
                                            .config
                                            .as_ref()
                                            .is_some_and(|c| c.value.auto_start_daemon)
                                        {
                                            "开"
                                        } else {
                                            "关"
                                        }
                                    ),
                                    Action::AutoStart,
                                    self.config.is_none() || self.daemon.busy,
                                    cx,
                                )
                                .ghost()
                                .small(),
                            ),
                    )
                    .child(muted(details))
                    .child(muted("所有路线通过执行服务回放；正在回放时可使用急停键。")),
            );
        }
        header
    }

    fn execution_progress(&self) -> String {
        self.document
            .as_ref()
            .and_then(|d| d.stem().ok())
            .and_then(|s| self.cursor.routes.get(&s))
            .map(|p| format!("单步 {}/{}，起始点 {}", p.last_done, p.total, p.start_at))
            .unwrap_or_else(|| "尚无单步进度".into())
    }

    fn execution_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let has_doc = self.document.is_some();
        let stop_key = self
            .config
            .as_ref()
            .map(|c| c.value.hotkeys.stop_all.as_str())
            .unwrap_or("F12");
        let disabled =
            !has_doc || !self.daemon.running || self.daemon.busy || self.daemon_error.is_some();
        let inputs_disabled = !has_doc
            || self.controls_busy()
            || self.confirmation.is_some()
            || self.capture.is_some();
        let binding = self
            .document
            .as_ref()
            .and_then(|doc| doc.stem().ok())
            .and_then(|stem| self.config.as_ref()?.value.binding_for(&stem));
        let hotkey_text = binding
            .map(|b| game_helper_core::config::hotkey_display(&b.hotkey))
            .unwrap_or_else(|| "未设置".into());
        row()
            .id("execution-bar")
            .accessibility_id("execution-bar")
            .role(gpui_kit::Role::Group)
            .aria_label("路线执行与热键绑定")
            .flex_shrink_0()
            .flex_wrap()
            .gap_2()
            .px_3()
            .py_1()
            .bg(color(SURFACE))
            .border_t_1()
            .border_color(color(BORDER))
            .child(
                row()
                    .flex_shrink_0()
                    .child(muted("起始点"))
                    .child(
                        div()
                            .id("start-field")
                            .accessibility_id("start-field")
                            .role(gpui_kit::Role::Group)
                            .aria_label("起始点输入区域")
                            .w(px(56.))
                            .child(Styled::h(
                                Input::new(&self.fields["start"])
                                    .accessibility_id("start")
                                    .aria_label("起始点")
                                    .small()
                                    .disabled(inputs_disabled),
                                px(32.),
                            )),
                    )
                    .child(
                        self.button(
                            "only",
                            "试跑选中点",
                            Action::RunPoint(self.selected.map(|i| i + 1).unwrap_or(0)),
                            disabled || self.selected.is_none(),
                            cx,
                        )
                        .small(),
                    ),
            )
            .child(
                row()
                    .flex_1()
                    .justify_end()
                    .min_w(px(290.))
                    .child(muted("路线热键"))
                    .child(
                        div()
                            .id("hotkey-current")
                            .accessibility_id("hotkey-current")
                            .role(gpui_kit::Role::Status)
                            .aria_label(format!("当前快捷键 {hotkey_text}"))
                            .max_w(px(120.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(12.))
                            .child(hotkey_text),
                    )
                    .child(
                        self.button(
                            "hotkey-set",
                            if binding.is_some() {
                                "修改快捷键"
                            } else {
                                "设置快捷键"
                            },
                            Action::SetHotkey,
                            !has_doc || self.daemon.busy || self.daemon_error.is_some(),
                            cx,
                        )
                        .accessibility_id("hotkey-set")
                        .track_focus(&self.hotkey_button_focus)
                        .small(),
                    )
                    .child(
                        self.button(
                            "hotkey-clear",
                            "清除",
                            Action::ClearHotkey,
                            binding.is_none() || self.daemon.busy || self.daemon_error.is_some(),
                            cx,
                        )
                        .accessibility_id("hotkey-clear")
                        .ghost()
                        .small(),
                    )
                    .child(
                        self.button(
                            "bind",
                            "保存起始点",
                            Action::Bind,
                            binding.is_none() || self.daemon.busy || self.daemon_error.is_some(),
                            cx,
                        )
                        .accessibility_id("bind")
                        .small(),
                    ),
            )
            .child(
                row()
                    .flex_shrink_0()
                    .pl_3()
                    .border_l_1()
                    .border_color(color(BORDER))
                    .child(muted("全局急停"))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(color(ERROR_BG))
                            .text_color(color(DANGER))
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(stop_key.to_owned()),
                    ),
            )
    }

    fn hotkey_dialog(&self, cx: &Context<Self>) -> impl IntoElement {
        let capture = self.capture.as_ref().expect("capture dialog");
        let current = capture
            .candidate
            .as_ref()
            .map(|key| game_helper_core::config::hotkey_display(key))
            .unwrap_or_else(|| {
                let mut modifiers = Vec::new();
                if capture.modifiers.control {
                    modifiers.push("Ctrl");
                }
                if capture.modifiers.alt {
                    modifiers.push("Alt");
                }
                if capture.modifiers.shift {
                    modifiers.push("Shift");
                }
                if capture.modifiers.platform {
                    modifiers.push("Win");
                }
                if modifiers.is_empty() {
                    "等待按键…".into()
                } else {
                    format!("{} + …", modifiers.join(" + "))
                }
            });
        let hint = if capture.saving {
            "正在保存快捷键…"
        } else if !capture.armed {
            "请先松开按键，再输入快捷键"
        } else if capture.candidate.is_some() {
            "已捕获，请松开按键"
        } else {
            "请按下一个按键，或 Ctrl / Alt / Shift / Win 与按键的组合"
        };
        div()
            .id("hotkey-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .bg(gpui_kit::rgba(SCRIM))
            .flex()
            .items_center()
            .justify_center()
            .p_5()
            .child(
                column()
                    .id("hotkey-dialog")
                    .accessibility_id("hotkey-dialog")
                    .role(gpui_kit::Role::Dialog)
                    .aria_label("设置快捷键，请按下快捷键")
                    .occlude()
                    .w_full()
                    .max_w(px(500.))
                    .gap_4()
                    .p_5()
                    .bg(color(SURFACE))
                    .border_1()
                    .border_color(color(BORDER))
                    .rounded(px(8.))
                    .child(heading("请按下快捷键"))
                    .child(
                        div()
                            .id("hotkey-capture-value")
                            .accessibility_id("hotkey-capture-value")
                            .role(gpui_kit::Role::Status)
                            .aria_label(current.clone())
                            .text_size(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .px_4()
                            .py_3()
                            .rounded(px(6.))
                            .bg(color(CANVAS))
                            .child(current),
                    )
                    .child(muted(hint))
                    .child(muted(
                        "捕获后自动保存；Esc 取消。录制期间路线热键临时停用，急停键仍有效。",
                    ))
                    .children(capture.error.as_ref().map(|error| {
                        div()
                            .id("hotkey-capture-error")
                            .role(gpui_kit::Role::Alert)
                            .aria_label(error.clone())
                            .text_color(color(DANGER))
                            .text_size(px(12.))
                            .child(error.clone())
                    }))
                    .child(
                        row().justify_end().child(
                            Button::new("hotkey-capture-cancel")
                                .accessibility_id("hotkey-capture-cancel")
                                .label("取消")
                                .ghost()
                                .track_focus(&self.capture_focus)
                                .disabled(capture.saving)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.cancel_hotkey_capture(window, cx)
                                })),
                        ),
                    ),
            )
    }

    fn confirmation_dialog(&self, cx: &Context<Self>) -> impl IntoElement {
        let confirm = self.confirmation.as_ref().expect("confirmation");
        let mut choices = row().flex_wrap().justify_end();
        for (i, (label, action)) in confirm.choices.iter().enumerate() {
            let action = action.clone();
            let mut button = Button::new(("confirm", i))
                .label(*label)
                .disabled(self.controls_busy())
                .on_click(cx.listener(move |v, _, w, cx| v.action(action.clone(), w, cx)));
            if *label == "取消" {
                button = button.ghost().track_focus(&self.confirmation_focus);
            } else if *label == "确认删除" {
                button = button.danger();
            } else if i == 0 {
                button = button.primary().border_1().border_color(color(EMPHASIS));
            }
            choices = choices.child(button);
        }
        div()
            .id("confirmation-backdrop")
            .occlude()
            .absolute()
            .inset_0()
            .bg(gpui_kit::rgba(SCRIM))
            .flex()
            .items_center()
            .justify_center()
            .p_5()
            .child(
                column()
                    .max_w(px(580.))
                    .w_full()
                    .gap_5()
                    .p_5()
                    .bg(color(SURFACE))
                    .border_1()
                    .border_color(color(BORDER))
                    .rounded(px(8.))
                    .id("confirmation-dialog")
                    .role(gpui_kit::Role::Dialog)
                    .aria_label("确认操作")
                    .occlude()
                    .child(heading("请确认此操作"))
                    .child(div().line_height(px(24.)).child(confirm.message.clone()))
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
            .unwrap_or(window.viewport_size().width >= px(1100.));
        let work = if self.document.is_some() {
            self.document_workspace(cx)
        } else {
            self.welcome(cx)
        };
        let body = if show_library {
            h_resizable("manager-panels")
                .child(
                    resizable_panel()
                        .size(px(246.))
                        .size_range(px(215.)..px(390.))
                        .child(self.route_library(cx)),
                )
                .child(resizable_panel().child(work))
                .into_any_element()
        } else {
            work.into_any_element()
        };
        let status: SharedString = if self.controls_busy() {
            "正在处理…".into()
        } else {
            self.status.clone()
        };
        let status_tooltip = status.clone();
        let progress = self.execution_progress();
        let progress_tooltip = progress.clone();
        let mut root = column()
            .relative()
            .size_full()
            .bg(color(CANVAS))
            .text_color(color(TEXT))
            .text_size(px(14.))
            .child(self.service_header(cx))
            .children(self.ui_persistence_warning(cx))
            .child(div().flex_1().min_h_0().min_w_0().child(body))
            .child(self.execution_bar(cx))
            .child(
                row()
                    .id("manager-status")
                    .role(gpui_kit::Role::Status)
                    .accessibility_id("manager-status")
                    .aria_label(format!("{status}。{progress}"))
                    .flex_shrink_0()
                    .px_3()
                    .py_1()
                    .line_height(px(20.))
                    .text_size(px(12.))
                    .bg(color(if self.status_is_error {
                        ERROR_BG
                    } else {
                        CANVAS
                    }))
                    .text_color(color(if self.status_is_error { DANGER } else { MUTED }))
                    .child(
                        div()
                            .id("manager-status-message")
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .tooltip(move |w, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(status_tooltip.clone())
                                    .build(w, cx)
                            })
                            .child(status),
                    )
                    .children((!self.status_is_error).then(|| {
                        div()
                            .id("execution-progress")
                            .accessibility_id("execution-progress")
                            .flex_shrink_0()
                            .max_w(px(280.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .tooltip(move |w, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(progress_tooltip.clone())
                                    .build(w, cx)
                            })
                            .child(progress)
                    })),
            );
        if self.opening_route {
            root = root.child(
                div()
                    .id("route-loading-shield")
                    .role(gpui_kit::Role::Status)
                    .accessibility_id("route-loading-shield")
                    .aria_label("正在切换路线")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        muted("正在切换路线…")
                            .px_4()
                            .py_2()
                            .bg(color(SURFACE))
                            .border_1()
                            .border_color(color(BORDER))
                            .rounded(px(6.)),
                    ),
            );
        }
        if self.capture.is_some() {
            root = root.child(gpui_kit::deferred(self.hotkey_dialog(cx)).with_priority(3));
        }
        if self.confirmation.is_some() {
            root = root.child(gpui_kit::deferred(self.confirmation_dialog(cx)).with_priority(2));
        }
        root.id("manager-root")
            .on_drag_move(cx.listener(
                |view,
                 event: &gpui_kit::DragMoveEvent<super::point_interaction::PointDragPayload>,
                 window,
                 cx| {
                    view.move_point_drag(event, window, cx);
                },
            ))
            .on_drag_move(cx.listener(
                |view,
                 event: &gpui_kit::DragMoveEvent<super::interaction::OperationDragPayload>,
                 window,
                 cx| {
                    view.move_operation_drag(event, window, cx);
                },
            ))
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.operation_drag_released(window, cx);
                }),
            )
            .on_mouse_up_out(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.operation_drag_released(window, cx);
                }),
            )
            .capture_key_down(
                cx.listener(|view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if view.capture.is_some() {
                        view.capture_key_down(event, window, cx);
                        return;
                    }
                    if event.keystroke.key == "escape" {
                        if view.drag.is_some() || view.point_drag.is_some() {
                            view.cancel_list_drag(cx);
                            cx.stop_active_drag(window);
                            cx.stop_propagation();
                        } else if view.move_operation.is_some() {
                            view.cancel_operation_move(window, cx);
                            cx.stop_propagation();
                        }
                    }
                }),
            )
            .on_key_down(cx.listener(|v, event: &gpui_kit::KeyDownEvent, w, cx| {
                if v.opening_route {
                    cx.stop_propagation();
                    return;
                }
                if v.confirmation.is_some() && event.keystroke.key == "escape" {
                    v.action(Action::Cancel, w, cx);
                    cx.stop_propagation();
                }
            }))
    }
}
