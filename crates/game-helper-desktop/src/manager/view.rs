//! Route workspace presentation. All document mutations stay in the parent actions.
use super::*;
use crate::workspace_theme::*;
use gpui_kit::base::{h_resizable, resizable_panel};
use gpui_kit::component::{
    Selectable, Sizable,
    button::{ButtonCustomVariant, ButtonVariants},
    input::{Editor, Input},
    select::Select,
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

    fn route_library(&self, cx: &Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let mut entries = column().gap_1();
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
            let binding = self
                .config
                .as_ref()
                .and_then(|c| c.value.binding_for(&entry.stem))
                .map(|b| b.hotkey.as_str())
                .unwrap_or("未绑定");
            let mut title = column()
                .gap_1()
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(entry.name.clone()),
                )
                .child(
                    list_detail(format!("{}.yaml", entry.stem), selected)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap(),
                )
                .child(
                    row()
                        .justify_between()
                        .child(list_detail(format!("{} 个点位", entry.points), selected))
                        .child(list_detail(binding.to_owned(), selected)),
                );
            if entry.error.is_some() {
                title = title.child(
                    div()
                        .text_size(px(12.))
                        .text_color(color(DANGER))
                        .child("格式有误，打开修复"),
                );
            }
            let variant = ButtonCustomVariant::new(cx)
                .color(color(SURFACE))
                .foreground(color(TEXT))
                .hover(color(CANVAS))
                .active(color(LIST_SELECTED));
            entries = entries.child(
                self.button(
                    ("route", i),
                    "",
                    Action::Open(entry.path.clone()),
                    false,
                    cx,
                )
                .accessibility_id(format!("route-{}", entry.stem))
                .accessibility_label(format!("{} {}", entry.stem, entry.name))
                .custom(variant)
                .selected(selected)
                .w_full()
                .h_auto()
                .p_3()
                .border_l_2()
                .border_color(color(if selected { EMPHASIS } else { SURFACE }))
                .child(title),
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
            .gap_3()
            .p_4()
            .child(
                row()
                    .justify_between()
                    .child(heading("路线库"))
                    .child(muted(format!("{} 条", self.routes.len()))),
            )
            .child(
                Input::new(&self.search)
                    .disabled(self.controls_busy() || self.confirmation.is_some())
                    .w_full(),
            )
            .child(
                row()
                    .flex_wrap()
                    .child(
                        self.button("new-toggle", "新建路线", Action::ToggleNew, false, cx)
                            .secondary()
                            .small(),
                    )
                    .child(
                        self.button("import", "导入", Action::Import, false, cx)
                            .ghost()
                            .small(),
                    )
                    .child(
                        self.button("refresh", "刷新", Action::Refresh, false, cx)
                            .ghost()
                            .small(),
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
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pb_3()
                    .child(entries),
            )
            .child(
                muted("路线仅在点击保存后写入文件")
                    .p_3()
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
                    .flex_wrap()
                    .child(
                        self.button("export", "导出", Action::Export, false, cx)
                            .small(),
                    )
                    .child(
                        self.button("reveal", "定位文件", Action::Reveal, false, cx)
                            .ghost()
                            .small(),
                    )
                    .child(
                        self.button(
                            "delete",
                            "删除路线",
                            Action::Delete(false),
                            self.daemon.busy,
                            cx,
                        )
                        .ghost()
                        .small()
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
        let mut sequence = column();
        for (i, point) in points.iter().enumerate() {
            let selected = self.selected == Some(i);
            let number = column()
                .w(px(32.))
                .flex_shrink_0()
                .items_center()
                .child(
                    div()
                        .size(px(30.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(color(if selected { EMPHASIS } else { CANVAS }))
                        .text_color(color(if selected { SURFACE } else { MUTED }))
                        .font_family("Bahnschrift")
                        .text_size(px(15.))
                        .child((i + 1).to_string()),
                )
                .child(
                    div()
                        .w(px(1.))
                        .h(px(26.))
                        .bg(color(if i + 1 < points.len() {
                            BORDER
                        } else if selected {
                            LIST_SELECTED
                        } else {
                            SURFACE
                        })),
                );
            let name = if selected {
                self.text("point_name", cx)
            } else {
                point.name.clone()
            };
            sequence = sequence.child(
                self.button(("point", i), "", Action::Select(i), false, cx)
                    .accessibility_id(format!("point-{i}"))
                    .accessibility_label(format!("第 {} 点 {}", i + 1, name))
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(color(SURFACE))
                            .foreground(color(TEXT))
                            .hover(color(CANVAS))
                            .active(color(LIST_SELECTED)),
                    )
                    .selected(selected)
                    .w_full()
                    .h_auto()
                    .border_l_2()
                    .border_color(color(if selected { EMPHASIS } else { SURFACE }))
                    .px_3()
                    .pt_3()
                    .pb_0()
                    .child(
                        row().w_full().items_start().gap_3().child(number).child(
                            column()
                                .flex_1()
                                .gap_1()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(name),
                                )
                                .child(list_detail(
                                    format!("{} 个动作", point.operations.len()),
                                    selected,
                                )),
                        ),
                    ),
            );
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
                column()
                    .gap_3()
                    .p_4()
                    .child(
                        row()
                            .justify_between()
                            .child(heading("点位序列"))
                            .child(muted(format!("{} 个", points.len()))),
                    )
                    .child(
                        self.button("add-point", "新增点位", Action::AddPoint, false, cx)
                            .secondary()
                            .small(),
                    ),
            )
            .child(
                div()
                    .id("points")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .child(sequence),
            )
            .child(
                row()
                    .flex_wrap()
                    .gap_1()
                    .p_3()
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
                        .small(),
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
                        .small(),
                    )
                    .child(
                        self.button(
                            "remove-point",
                            "删除点位",
                            Action::RemovePoint,
                            index.is_none(),
                            cx,
                        )
                        .ghost()
                        .small()
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
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(18.))
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
                            .child(
                                self.button(
                                    ("op-up", index),
                                    "上移",
                                    Action::MoveOp(index, -1),
                                    index == 0,
                                    cx,
                                )
                                .ghost()
                                .small()
                                .accessibility_label(format!("上移动作 {}", index + 1)),
                            )
                            .child(
                                self.button(
                                    ("op-down", index),
                                    "下移",
                                    Action::MoveOp(index, 1),
                                    index + 1 == self.operation_fields.len(),
                                    cx,
                                )
                                .ghost()
                                .small()
                                .accessibility_label(format!("下移动作 {}", index + 1)),
                            )
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
        let mut add = row().flex_wrap().gap_1();
        for (i, kind) in OpKind::ALL.into_iter().enumerate() {
            add = add.child(
                self.button(
                    ("op", i),
                    format!("+ {}", op_name(kind)),
                    Action::AddOp(kind),
                    false,
                    cx,
                )
                .accessibility_id(format!("operation-add-{}", kind.label()))
                .small()
                .h(px(28.))
                .ghost(),
            );
        }
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
                    ),
            )
            .child(add);
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
            .pb_4();
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
                "使用上方按钮添加动作；动作按列表顺序执行，时间单位为毫秒。",
            ));
        }
        for (i, op) in self.operation_fields.iter().enumerate() {
            list = list.child(self.operation_panel(i, op, cx));
        }
        column()
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
                    .child(muted(
                        "停止服务不会停止独立执行进程；正在回放时使用急停键。",
                    )),
            );
        }
        header
    }

    fn execution_progress(&self) -> String {
        if self.runner.is_some() {
            return "独立执行进程运行中".into();
        }
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
            !has_doc || self.daemon.busy || self.daemon_error.is_some() || self.runner.is_some();
        let inputs_disabled = !has_doc || self.controls_busy() || self.confirmation.is_some();
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
                            Action::RunPoint(self.selected.map(|i| i + 1).unwrap_or(0), false),
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
                            .id("hotkey-binding-field")
                            .accessibility_id("hotkey-binding-field")
                            .role(gpui_kit::Role::Group)
                            .aria_label("路线热键输入区域")
                            .w(px(124.))
                            .tooltip(|w, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "热键留空后保存绑定，可解除此路线的热键绑定",
                                )
                                .build(w, cx)
                            })
                            .child(Styled::h(
                                Input::new(&self.fields["hotkey"])
                                    .accessibility_id("hotkey")
                                    .aria_label("路线热键")
                                    .small()
                                    .disabled(inputs_disabled),
                                px(32.),
                            )),
                    )
                    .child(
                        self.button(
                            "bind",
                            "保存绑定",
                            Action::Bind,
                            !has_doc || self.daemon.busy,
                            cx,
                        )
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
        if self.confirmation.is_some() {
            root = root.child(gpui_kit::deferred(self.confirmation_dialog(cx)).with_priority(2));
        }
        root.id("manager-root").on_key_down(cx.listener(
            |v, event: &gpui_kit::KeyDownEvent, w, cx| {
                if v.opening_route {
                    cx.stop_propagation();
                    return;
                }
                if v.confirmation.is_some() && event.keystroke.key == "escape" {
                    v.action(Action::Cancel, w, cx);
                    cx.stop_propagation();
                }
            },
        ))
    }
}
