//! Point-list dragging shares action-list feedback and cancellation semantics.
use super::interaction::{operation_destination, operation_handle_icon};
use super::*;
use crate::workspace_theme::*;
use gpui_kit::component::{Sizable, button::ButtonVariants};
use gpui_kit::{
    AnyWindowHandle, Div, DragMoveEvent, InteractiveElement, ParentElement, Pixels,
    Point as UiPoint, StatefulInteractiveElement, Styled, WeakEntity, div, px,
};

#[derive(Clone)]
pub(super) struct PointDragPayload {
    path: PathBuf,
    epoch: u64,
    from: usize,
    key: PointUiKey,
}

pub(super) struct PointDragState {
    payload: PointDragPayload,
    pub(super) gap: Option<usize>,
    position: UiPoint<Pixels>,
    pub(super) window: AnyWindowHandle,
}

struct PointDragPreview {
    manager: WeakEntity<Manager>,
    payload: PointDragPayload,
    title: String,
}
impl Render for PointDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.manager.upgrade().is_some_and(|manager| {
            manager.read(cx).point_drag.as_ref().is_some_and(|drag| {
                drag.payload.epoch == self.payload.epoch && drag.payload.from == self.payload.from
            })
        }) {
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
    fn point_interaction_available(&self) -> bool {
        !self.busy
            && !self.opening_route
            && !self.ui_closing
            && self.confirmation.is_none()
            && self
                .document
                .as_ref()
                .is_some_and(|doc| doc.mode() == EditMode::Form && doc.route().points.len() > 1)
    }

    fn point_drag_valid(&self, payload: &PointDragPayload) -> bool {
        self.point_interaction_available()
            && self.drag_epoch == payload.epoch
            && self.document.as_ref().is_some_and(|doc| {
                doc.path == payload.path
                    && PointUiKey::for_index(&doc.route().points, payload.from) == Some(payload.key)
            })
    }

    fn begin_point_drag(
        &mut self,
        payload: PointDragPayload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.point_drag_valid(&payload) {
            return;
        }
        self.drag_scroll.stop();
        self.drag = None;
        self.move_operation = None;
        if let Some(focus) = self.point_drag_focus.get(payload.from) {
            focus.focus(window, cx);
        }
        let position = window.mouse_position();
        self.point_drag = Some(PointDragState {
            payload,
            gap: self.point_gap_at(position),
            position,
            window: window.window_handle(),
        });
        cx.notify();
    }

    // Rows are direct scroll children, so offsets and row midpoints stay valid
    // after scrolling. Indicators are absolute and never alter these bounds.
    fn point_gap_at(&self, position: UiPoint<Pixels>) -> Option<usize> {
        if !self.point_scroll.bounds().contains(&position) {
            return None;
        }
        let count = self.document.as_ref()?.route().points.len();
        let offset = self.point_scroll.offset().y;
        for index in 0..count {
            let bounds = self.point_scroll.bounds_for_item(index)?;
            if position.y < bounds.top() + offset + bounds.size.height / 2. {
                return Some(index);
            }
        }
        Some(count)
    }

    pub(super) fn move_point_drag(
        &mut self,
        event: &DragMoveEvent<PointDragPayload>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(payload) = event.dragged_item().downcast_ref::<PointDragPayload>() else {
            return;
        };
        if event.event.pressed_button != Some(gpui_kit::MouseButton::Left)
            || !window.is_window_active()
            || !self.point_drag_valid(payload)
        {
            self.cancel_list_drag(cx);
            cx.stop_active_drag(window);
            return;
        }
        let position = event.event.position;
        let gap = self.point_gap_at(position);
        let Some(drag) = self.point_drag.as_mut() else {
            return;
        };
        drag.position = position;
        drag.gap = gap;
        let viewport = self.point_scroll.bounds();
        let delta = if position.x >= viewport.left() && position.x <= viewport.right() {
            gpui_kit::base::AutoScroll::compute_delta(position.y, viewport)
        } else {
            None
        };
        self.drag_scroll.set(delta, cx, |delta, view, cx| {
            let valid = view
                .point_drag
                .as_ref()
                .is_some_and(|drag| view.point_drag_valid(&drag.payload));
            if !cx.has_active_drag() || !valid {
                view.cancel_list_drag(cx);
                return;
            }
            let scroll = &view.point_scroll;
            let mut offset = scroll.offset();
            offset.y = (offset.y - delta).max(-scroll.max_offset().y).min(px(0.));
            scroll.set_offset(offset);
            if let Some(position) = view.point_drag.as_ref().map(|drag| drag.position) {
                let gap = view.point_gap_at(position);
                if let Some(drag) = view.point_drag.as_mut() {
                    drag.gap = gap;
                }
            }
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn finish_point_drag(
        &mut self,
        payload: &PointDragPayload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gap = self.point_gap_at(window.mouse_position());
        let valid = self.point_drag_valid(payload)
            && self.point_drag.as_ref().is_some_and(|drag| {
                drag.payload.epoch == payload.epoch && drag.payload.from == payload.from
            });
        self.drag_scroll.stop();
        self.point_drag = None;
        if valid && let Some(gap) = gap {
            let count = self.document.as_ref().unwrap().route().points.len();
            if let Some(to) = operation_destination(payload.from, gap, count)
                && payload.from != to
            {
                self.action(
                    Action::MovePointTo {
                        from: payload.from,
                        to,
                    },
                    window,
                    cx,
                );
            }
        }
        cx.notify();
    }

    pub(super) fn point_drag_handle(
        &self,
        index: usize,
        name: &str,
        cx: &Context<Self>,
    ) -> gpui_kit::AnyElement {
        if !self.point_interaction_available() {
            return Button::new(("point-drag-handle", index))
                .accessibility_id(format!("point-drag-{index}"))
                .accessibility_label(format!("拖动排序点位 {}", index + 1))
                .child(operation_handle_icon())
                .ghost()
                .small()
                .w(px(28.))
                .h(px(28.))
                .disabled(true)
                .into_any_element();
        }
        let doc = self.document.as_ref().unwrap();
        let payload = PointDragPayload {
            path: doc.path.clone(),
            epoch: self.drag_epoch,
            from: index,
            key: PointUiKey::for_index(&doc.route().points, index).unwrap(),
        };
        let manager = cx.weak_entity();
        let title = format!("{}  {}", index + 1, name);
        div()
            .id(("point-drag-handle", index))
            .accessibility_id(format!("point-drag-{index}"))
            .role(gpui_kit::Role::Button)
            .aria_label(format!("拖动排序点位 {}", index + 1))
            .aria_description(
                "拖动调整顺序；点击选中点位；Alt+↑/↓ 调整顺序；也可使用列表底部上移、下移按钮",
            )
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .hover(|style| style.bg(color(SELECTED)))
            .focus_visible(|style| style.border_1().border_color(color(EMPHASIS)))
            .w(px(28.))
            .h(px(28.))
            .flex_shrink_0()
            .track_focus(&self.point_drag_focus[index])
            .tab_stop(true)
            .child(operation_handle_icon())
            .on_click(cx.listener(move |view, _, window, cx| {
                view.action(Action::Select(index), window, cx);
                if view.selected == Some(index)
                    && let Some(focus) = view.point_drag_focus.get(index)
                {
                    focus.focus(window, cx);
                }
            }))
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    let modifiers = event.keystroke.modifiers;
                    if modifiers.alt
                        && !modifiers.control
                        && !modifiers.shift
                        && !modifiers.platform
                    {
                        let count = view
                            .document
                            .as_ref()
                            .map_or(0, |doc| doc.route().points.len());
                        let to = match event.keystroke.key.as_str() {
                            "up" => index.checked_sub(1),
                            "down" => index.checked_add(1).filter(|to| *to < count),
                            _ => return,
                        };
                        if view.point_interaction_available()
                            && let Some(to) = to
                        {
                            view.action(Action::MovePointTo { from: index, to }, window, cx);
                        }
                        cx.stop_propagation();
                    }
                }),
            )
            .on_drag(payload, move |payload, _, window, cx| {
                let _ = manager.update(cx, |view, cx| {
                    view.begin_point_drag(payload.clone(), window, cx)
                });
                cx.new(|_| PointDragPreview {
                    manager: manager.clone(),
                    payload: payload.clone(),
                    title: title.clone(),
                })
            })
            .into_any_element()
    }

    pub(super) fn point_drop_indicator(&self, gap: usize) -> Option<Div> {
        self.point_drag
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
}
