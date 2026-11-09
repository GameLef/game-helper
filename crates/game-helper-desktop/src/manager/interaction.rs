//! Native action-list gestures and the explicit move-to-position alternative.
use super::*;
use crate::workspace_theme::*;
use gpui_kit::component::{FocusTrapElement, Sizable, button::ButtonVariants, input::Input};
use gpui_kit::{
    AnyWindowHandle, Div, DragMoveEvent, InteractiveElement, ParentElement, Pixels,
    Point as UiPoint, StatefulInteractiveElement, Styled, WeakEntity, div, px,
};

pub(super) fn operation_handle_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><g fill="currentColor"><circle cx="5" cy="3" r="1.2"/><circle cx="11" cy="3" r="1.2"/><circle cx="5" cy="8" r="1.2"/><circle cx="11" cy="8" r="1.2"/><circle cx="5" cy="13" r="1.2"/><circle cx="11" cy="13" r="1.2"/></g></svg>"#).size(px(16.))
}

/// Keep the dialog's keyboard scope local without adding long-lived Manager fields.
#[derive(IntoElement)]
struct OperationMoveFocus {
    child: gpui_kit::AnyElement,
}
impl gpui_kit::RenderOnce for OperationMoveFocus {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("operation-move-focus", cx, |_, cx| cx.focus_handle());
        let focus = state.read(cx).clone();
        div()
            .child(self.child)
            .focus_trap("operation-move-trap", &focus)
    }
}

/// Convert an insertion gap in the original list to a final index after removal.
pub(super) fn operation_destination(from: usize, gap: usize, count: usize) -> Option<usize> {
    if from >= count || gap > count {
        return None;
    }
    Some(gap - usize::from(gap > from))
}

#[derive(Clone)]
pub(super) struct OperationDragPayload {
    path: PathBuf,
    point: usize,
    epoch: u64,
    ui_id: u64,
}

pub(super) struct OperationDragState {
    payload: OperationDragPayload,
    pub(super) gap: Option<usize>,
    position: UiPoint<Pixels>,
    window: AnyWindowHandle,
}

struct OperationDragPreview {
    manager: WeakEntity<Manager>,
    payload: OperationDragPayload,
    title: String,
}
impl Render for OperationDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.manager.upgrade().is_some_and(|manager| {
            manager.read(cx).drag.as_ref().is_some_and(|drag| {
                drag.payload.epoch == self.payload.epoch && drag.payload.ui_id == self.payload.ui_id
            })
        });
        if !visible {
            return div();
        }
        div()
            .max_w(px(320.))
            .px_3()
            .py_2()
            .bg(color(SELECTED))
            .text_color(color(TEXT))
            .border_1()
            .border_color(color(EMPHASIS))
            .rounded(px(4.))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(self.title.clone())
    }
}

impl Manager {
    fn operation_interaction_available(&self) -> bool {
        !self.busy && !self.opening_route && !self.ui_closing && self.confirmation.is_none()
    }

    pub(super) fn operation_child_offset(&self) -> usize {
        usize::from(self.point_info_open)
    }

    fn operation_drag_valid(&self, payload: &OperationDragPayload) -> bool {
        self.operation_interaction_available()
            && self.drag_epoch == payload.epoch
            && self.selected == Some(payload.point)
            && self
                .document
                .as_ref()
                .is_some_and(|doc| doc.path == payload.path && doc.mode() == EditMode::Form)
            && self
                .operation_fields
                .iter()
                .any(|op| op.ui_id == payload.ui_id)
    }

    pub(super) fn cancel_list_drag(&mut self, cx: &mut Context<Self>) {
        self.drag_scroll.stop();
        self.move_operation = None;
        let windows = [
            self.drag.take().map(|drag| drag.window),
            self.point_drag.take().map(|drag| drag.window),
        ];
        for window in windows.into_iter().flatten() {
            // Structural changes can call this without a Window. Release GPUI's
            // active preview after this Manager borrow has left the stack.
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    cx.stop_active_drag(window);
                });
            });
        }
        cx.notify();
    }

    fn begin_operation_drag(
        &mut self,
        payload: OperationDragPayload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.operation_drag_valid(&payload) {
            return;
        }
        self.drag_scroll.stop();
        self.move_operation = None;
        self.point_drag = None;
        if let Some(op) = self
            .operation_fields
            .iter()
            .find(|op| op.ui_id == payload.ui_id)
        {
            op.drag_focus.focus(window, cx);
        }
        let position = window.mouse_position();
        self.drag = Some(OperationDragState {
            payload,
            gap: self.operation_gap_at(position),
            position,
            window: window.window_handle(),
        });
        cx.notify();
    }

    /// ScrollHandle child bounds are before its scroll offset. Keeping the
    /// indicator out of layout leaves these variable-height row bounds stable.
    fn operation_gap_at(&self, position: UiPoint<Pixels>) -> Option<usize> {
        if !self.operation_scroll.bounds().contains(&position) {
            return None;
        }
        let offset = self.operation_scroll.offset().y;
        for index in 0..self.operation_fields.len() {
            let bounds = self
                .operation_scroll
                .bounds_for_item(index + self.operation_child_offset())?;
            if position.y < bounds.top() + offset + bounds.size.height / 2. {
                return Some(index);
            }
        }
        Some(self.operation_fields.len())
    }

    pub(super) fn move_operation_drag(
        &mut self,
        event: &DragMoveEvent<OperationDragPayload>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(payload) = event.dragged_item().downcast_ref::<OperationDragPayload>() else {
            return;
        };
        if event.event.pressed_button != Some(gpui_kit::MouseButton::Left)
            || !window.is_window_active()
            || !self.operation_drag_valid(payload)
        {
            self.cancel_list_drag(cx);
            cx.stop_active_drag(window);
            return;
        }
        let position = event.event.position;
        let gap = self.operation_gap_at(position);
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        drag.position = position;
        drag.gap = gap;
        let viewport = self.operation_scroll.bounds();
        // Leaving horizontally cancels edge scrolling, but returning while
        // holding the mouse still permits dropping into this same list.
        let delta = if position.x >= viewport.left() && position.x <= viewport.right() {
            gpui_kit::base::AutoScroll::compute_delta(position.y, viewport)
        } else {
            None
        };
        self.drag_scroll.set(delta, cx, |delta, view, cx| {
            let valid = view
                .drag
                .as_ref()
                .is_some_and(|drag| view.operation_drag_valid(&drag.payload));
            if !cx.has_active_drag() || !valid {
                view.cancel_list_drag(cx);
                return;
            }
            let scroll = &view.operation_scroll;
            let mut offset = scroll.offset();
            offset.y = (offset.y - delta).max(-scroll.max_offset().y).min(px(0.));
            scroll.set_offset(offset);
            if let Some(position) = view.drag.as_ref().map(|drag| drag.position) {
                let gap = view.operation_gap_at(position);
                if let Some(drag) = view.drag.as_mut() {
                    drag.gap = gap;
                }
            }
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn finish_operation_drag(
        &mut self,
        payload: &OperationDragPayload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gap = self.operation_gap_at(window.mouse_position());
        let from = self
            .operation_fields
            .iter()
            .position(|op| op.ui_id == payload.ui_id);
        let valid = self.operation_drag_valid(payload)
            && self.drag.as_ref().is_some_and(|drag| {
                drag.payload.epoch == payload.epoch && drag.payload.ui_id == payload.ui_id
            });
        self.drag_scroll.stop();
        self.drag = None;
        if valid && let (Some(from), Some(gap)) = (from, gap) {
            let Some(to) = operation_destination(from, gap, self.operation_fields.len()) else {
                return;
            };
            if from != to {
                self.action(Action::MoveOpTo { from, to }, window, cx);
                if let Some(op) = self.operation_fields.get(to) {
                    op.drag_focus.focus(window, cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn operation_drag_released(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_none() && self.point_drag.is_none() {
            return;
        }
        self.drag_scroll.stop();
        // on_drop must run before outside-release cleanup, regardless of the
        // listener registration order of the parent and scrolling children.
        cx.defer_in(window, |view, window, cx| {
            if view.drag.is_some() || view.point_drag.is_some() {
                view.cancel_list_drag(cx);
                cx.stop_active_drag(window);
            }
        });
    }

    pub(super) fn operation_drag_handle(
        &self,
        index: usize,
        operation: &OperationFields,
        cx: &Context<Self>,
    ) -> gpui_kit::AnyElement {
        if !self.operation_interaction_available() {
            return Button::new(("operation-drag-handle", operation.ui_id))
                .accessibility_id(format!("operation-drag-{index}"))
                .accessibility_label(format!("拖动排序动作 {}", index + 1))
                .child(operation_handle_icon())
                .ghost()
                .small()
                .w(px(28.))
                .h(px(28.))
                .disabled(true)
                .into_any_element();
        }
        let payload = OperationDragPayload {
            path: self
                .document
                .as_ref()
                .map(|doc| doc.path.clone())
                .unwrap_or_default(),
            point: self.selected.unwrap_or(usize::MAX),
            epoch: self.drag_epoch,
            ui_id: operation.ui_id,
        };
        let manager = cx.weak_entity();
        let title = format!("{}  {}", index + 1, operation.name.read(cx).value());
        let handle = div()
            .id(("operation-drag-handle", operation.ui_id))
            .accessibility_id(format!("operation-drag-{index}"))
            .role(gpui_kit::Role::Button)
            .aria_label(format!("拖动排序动作 {}，点击移动到指定位置", index + 1))
            .aria_description("拖动调整顺序；点击指定位置；Alt+↑/↓ 调整顺序")
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .hover(|style| style.bg(color(SELECTED)))
            .focus_visible(|style| style.border_1().border_color(color(EMPHASIS)))
            .child(operation_handle_icon())
            .w(px(28.))
            .h(px(28.))
            .flex_shrink_0()
            .track_focus(&operation.drag_focus)
            .tab_stop(true)
            .on_click(cx.listener(move |view, _, window, cx| {
                view.open_operation_move(index, window, cx);
            }))
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    let modifiers = event.keystroke.modifiers;
                    if modifiers.alt
                        && !modifiers.control
                        && !modifiers.shift
                        && !modifiers.platform
                    {
                        let to = match event.keystroke.key.as_str() {
                            "up" => index.checked_sub(1),
                            "down" => index
                                .checked_add(1)
                                .filter(|to| *to < view.operation_fields.len()),
                            _ => return,
                        };
                        if view.operation_interaction_available()
                            && let Some(to) = to
                        {
                            view.action(Action::MoveOpTo { from: index, to }, window, cx);
                            if let Some(op) = view.operation_fields.get(to) {
                                op.drag_focus.focus(window, cx);
                            }
                        }
                        cx.stop_propagation();
                    }
                }),
            );
        handle
            .on_drag(payload, move |payload, _, window, cx| {
                let _ = manager.update(cx, |view, cx| {
                    view.begin_operation_drag(payload.clone(), window, cx);
                });
                cx.new(|_| OperationDragPreview {
                    manager: manager.clone(),
                    payload: payload.clone(),
                    title: title.clone(),
                })
            })
            .into_any_element()
    }

    pub(super) fn operation_drop_indicator(&self, gap: usize) -> Option<Div> {
        self.drag
            .as_ref()
            .filter(|drag| drag.gap == Some(gap))
            .map(|_| {
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .h(px(2.))
                    .bg(color(EMPHASIS))
            })
    }

    fn open_operation_move(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.operation_interaction_available() || index >= self.operation_fields.len() {
            return;
        }
        self.cancel_list_drag(cx);
        self.move_operation = Some(index);
        self.move_position.update(cx, |input, cx| {
            input.set_value((index + 1).to_string(), window, cx);
        });
        self.move_position
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    pub(super) fn cancel_operation_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.move_operation.take()
            && let Some(op) = self.operation_fields.get(index)
        {
            op.drag_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn confirm_operation_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.operation_interaction_available() {
            return;
        }
        let Some(from) = self.move_operation else {
            return;
        };
        let position = self.move_position.read(cx).value().trim().parse::<usize>();
        let count = self.operation_fields.len();
        let Ok(position) = position else {
            self.error(anyhow::anyhow!("请输入 1 到 {count} 之间的动作位置"));
            cx.notify();
            return;
        };
        if position == 0 || position > count || from >= count {
            self.error(anyhow::anyhow!("请输入 1 到 {count} 之间的动作位置"));
            cx.notify();
            return;
        }
        self.move_operation = None;
        if from != position - 1 {
            self.action(
                Action::MoveOpTo {
                    from,
                    to: position - 1,
                },
                window,
                cx,
            );
        }
        if let Some(op) = self.operation_fields.get(position - 1) {
            op.drag_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn operation_move_dialog(&self, cx: &Context<Self>) -> Option<Div> {
        let index = self.move_operation?;
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(gpui_kit::rgba(SCRIM))
                .occlude()
                .flex()
                .justify_center()
                .items_start()
                .pt_4()
                .child(OperationMoveFocus {
                    child: div()
                        .id("operation-move-dialog")
                        .accessibility_id("operation-move-dialog")
                        .role(gpui_kit::Role::Dialog)
                        .aria_label(format!("移动动作 {}", index + 1))
                        .w(px(300.))
                        .max_w_full()
                        .p_4()
                        .bg(color(SURFACE))
                        .border_1()
                        .border_color(color(CONTROL_BORDER))
                        .rounded(px(6.))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(format!("将动作 {} 移动到", index + 1))
                        .child(
                            Input::new(&self.move_position)
                                .accessibility_id("operation-move-position")
                                .aria_label(format!(
                                    "目标动作位置，1 到 {}",
                                    self.operation_fields.len()
                                ))
                                .disabled(!self.operation_interaction_available())
                                .w_full(),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("operation-move-cancel")
                                        .accessibility_id("operation-move-cancel")
                                        .label("取消")
                                        .small()
                                        .h(px(32.))
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.cancel_operation_move(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("operation-move-confirm")
                                        .accessibility_id("operation-move-confirm")
                                        .label("移动")
                                        .primary()
                                        .small()
                                        .h(px(32.))
                                        .disabled(!self.operation_interaction_available())
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.confirm_operation_move(window, cx);
                                        })),
                                ),
                        )
                        .on_key_down(cx.listener(
                            |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                                if event.keystroke.key == "enter" {
                                    view.confirm_operation_move(window, cx);
                                    cx.stop_propagation();
                                }
                            },
                        ))
                        .into_any_element(),
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::operation_destination;

    #[test]
    fn insertion_gaps_resolve_to_final_indices() {
        // Both gaps surrounding the source are no-ops; crossing its original
        // position must compensate for the removed item, including list ends.
        for (from, gap, count, expected) in [
            (0, 0, 4, Some(0)),
            (0, 1, 4, Some(0)),
            (0, 4, 4, Some(3)),
            (3, 0, 4, Some(0)),
            (3, 3, 4, Some(3)),
            (3, 4, 4, Some(3)),
            (1, 3, 4, Some(2)),
            (2, 1, 4, Some(1)),
            (0, 0, 0, None),
            (4, 0, 4, None),
            (1, 5, 4, None),
        ] {
            assert_eq!(
                operation_destination(from, gap, count),
                expected,
                "from={from}, gap={gap}, count={count}"
            );
        }
    }
}
