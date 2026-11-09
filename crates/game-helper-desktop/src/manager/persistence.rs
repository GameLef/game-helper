//! Editor-only operation identities and disclosure persistence. Never writes route data.
use super::*;

impl Manager {
    pub(super) fn prepare_ui_baseline(&mut self) {
        self.ui_baseline.clear();
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let Ok(route_key) = normalize_route_key(&self.paths, &doc.path) else {
            return;
        };
        let points = doc.route().points.clone();
        for (index, point) in points.iter().enumerate() {
            let key = PointUiKey::for_index(&points, index).unwrap();
            match self
                .ui_state
                .reconcile_point(&route_key, key, &point.operations)
                .and_then(|operations| {
                    cache_reconciled_point(
                        &mut self.ui_state,
                        &mut self.ui_persisted_state,
                        &route_key,
                        key,
                        operations,
                    )
                }) {
                Ok(()) => {}
                Err(error) => {
                    self.ui_error = Some(format!("无法恢复展开状态：{error:#}"));
                    return;
                }
            }
        }
        self.ui_baseline = points
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                let key = PointUiKey::for_index(&points, index)?;
                self.ui_state.point(&route_key, key).cloned()
            })
            .collect();
    }

    pub(super) fn restore_discarded_ui(&mut self) {
        let Some(doc) = &self.document else { return };
        let Ok(route_key) = normalize_route_key(&self.paths, &doc.path) else {
            return;
        };
        self.ui_state.remove_route(&route_key);
        for point in self.ui_baseline.clone() {
            if let Err(error) = self
                .ui_state
                .put_point(&route_key, point.key, point.operations)
            {
                self.ui_error = Some(format!("无法恢复展开状态：{error:#}"));
            }
        }
        self.ui_changed();
    }

    pub(super) fn current_ui_key(&self) -> Result<Option<(String, PointUiKey)>> {
        let Some(doc) = self.document.as_ref() else {
            return Ok(None);
        };
        let Some(index) = self.selected else {
            return Ok(None);
        };
        let Some(key) = PointUiKey::for_index(&doc.route().points, index) else {
            return Ok(None);
        };
        Ok(Some((normalize_route_key(&self.paths, &doc.path)?, key)))
    }

    pub(super) fn ui_changed(&mut self) {
        self.ui_revision = self.ui_revision.wrapping_add(1);
        self.ui_due = Some(Instant::now() + Duration::from_millis(300));
    }

    /// Uses committed operation descriptors, so invalid text never blocks disclosure.
    pub(super) fn remember_operation_state(&mut self) -> Result<()> {
        let Some(doc) = self.document.as_ref() else {
            return Ok(());
        };
        if doc.mode() != EditMode::Form {
            return Ok(());
        }
        let Some((route, key)) = self.current_ui_key()? else {
            return Ok(());
        };
        let Some(point) = self.selected.and_then(|i| doc.route().points.get(i)) else {
            return Ok(());
        };
        if point.operations.len() != self.operation_fields.len() {
            return Ok(());
        }
        let states = point
            .operations
            .iter()
            .zip(&self.operation_fields)
            .map(|(descriptor, fields)| OperationUiState {
                id: fields.ui_id,
                expanded: fields.expanded,
                descriptor: descriptor.clone(),
            })
            .collect::<Vec<_>>();
        // Keep the saved descriptors for discard, while remembering disclosure
        // choices made to identities that still belong to the saved route.
        for baseline in &mut self.ui_baseline {
            for saved in &mut baseline.operations {
                if let Some(current) = states.iter().find(|current| current.id == saved.id) {
                    saved.expanded = current.expanded;
                }
            }
        }
        let previous = self.ui_state.point(&route, key).cloned();
        self.ui_state.put_point(&route, key, states)?;
        if previous.as_ref() != self.ui_state.point(&route, key) {
            self.ui_changed();
        }
        Ok(())
    }

    pub(super) fn restore_operation_states(&mut self) -> Vec<OperationUiState> {
        let restored = (|| -> Result<Vec<OperationUiState>> {
            let Some((route, key)) = self.current_ui_key()? else {
                return Ok(Vec::new());
            };
            let operations = self
                .document
                .as_ref()
                .and_then(|doc| self.selected.and_then(|i| doc.route().points.get(i)))
                .map(|point| point.operations.clone())
                .unwrap_or_default();
            let states = self.ui_state.reconcile_point(&route, key, &operations)?;
            // Reconciliation on first view is memory-only; simply viewing a route does not write.
            cache_reconciled_point(
                &mut self.ui_state,
                &mut self.ui_persisted_state,
                &route,
                key,
                states.clone(),
            )?;
            Ok(states)
        })();
        match restored {
            Ok(states) => states,
            Err(error) => {
                self.ui_error = Some(format!("无法恢复展开状态：{error:#}"));
                Vec::new()
            }
        }
    }

    pub(super) fn schedule_ui_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui_writing
            || self.ui_timer.is_some()
            || self.ui_due.is_none()
            || self.ui_error.is_some()
        {
            return;
        }
        let delay = self
            .ui_due
            .unwrap()
            .saturating_duration_since(Instant::now());
        self.ui_timer = Some(cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor().timer(delay).await;
            let _ = weak.update_in(cx, |view, window, cx| {
                view.ui_timer = None;
                if view.ui_due.is_some_and(|due| due <= Instant::now()) {
                    view.persist_ui_state(window, cx);
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn persist_ui_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui_writing || self.ui_revision == self.ui_saved_revision {
            return;
        }
        let Some(snapshot) = self.ui_snapshot.clone() else {
            return;
        };
        let mut file = ManagerUiFile {
            state: self.ui_state.clone(),
            snapshot,
        };
        let paths = self.paths.clone();
        let revision = self.ui_revision;
        self.ui_writing = true;
        self.ui_due = None;
        self.ui_job = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = cx
                .background_spawn(
                    async move { file.save(&paths).map(|_| (file.snapshot, file.state)) },
                )
                .await;
            let _ = weak.update_in(cx, |view, window, cx| {
                view.ui_writing = false;
                match result {
                    Ok((snapshot, persisted)) => {
                        view.ui_snapshot = Some(snapshot);
                        view.ui_persisted_state = persisted;
                        view.ui_saved_revision = revision;
                        view.ui_error = None;
                        if view.ui_saved_revision != view.ui_revision {
                            view.ui_due = Some(Instant::now());
                        } else if view.ui_closing {
                            window.remove_window();
                        }
                    }
                    Err(error) => {
                        view.ui_due = None;
                        view.ui_error = Some(format!(
                            "展开状态未能保存：{error:#}；本次状态仍在内存中，可重试"
                        ));
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn retry_ui_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui_writing {
            return;
        }
        let paths = self.paths.clone();
        self.ui_writing = true;
        self.ui_job = Some(cx.spawn_in(window, async move |weak, cx| {
            let loaded = cx
                .background_spawn(async move { ManagerUiFile::open(&paths) })
                .await;
            let _ = weak.update_in(cx, |view, window, cx| {
                view.ui_writing = false;
                match loaded {
                    Ok(file) => {
                        let remote = file.state;
                        let merged = merge_ui_changes(
                            &view.ui_persisted_state,
                            &view.ui_state,
                            remote.clone(),
                        );
                        let (mut merged, mut identities) = match merged {
                            Ok(merged) => merged,
                            Err(error) => {
                                view.ui_error = Some(format!("展开状态重试失败：{error:#}"));
                                cx.notify();
                                return;
                            }
                        };
                        let baseline = match remap_ui_baseline(
                            &mut merged,
                            &view.ui_baseline,
                            &mut identities,
                        ) {
                            Ok(baseline) => baseline,
                            Err(error) => {
                                view.ui_error = Some(format!("展开状态重试失败：{error:#}"));
                                cx.notify();
                                return;
                            }
                        };
                        view.ui_state = merged;
                        view.ui_baseline = baseline;
                        view.ui_snapshot = Some(file.snapshot);
                        view.ui_persisted_state = remote;
                        view.ui_error = None;
                        // Match identities, not positions: remote state may have a different order.
                        for fields in &mut view.operation_fields {
                            if let Some(id) = identities.get(&fields.ui_id) {
                                fields.ui_id = *id;
                                if let Some(state) = view
                                    .ui_state
                                    .routes
                                    .values()
                                    .flatten()
                                    .flat_map(|point| &point.operations)
                                    .find(|state| state.id == *id)
                                {
                                    fields.expanded = state.expanded;
                                }
                            }
                        }
                        view.ui_changed();
                        view.persist_ui_state(window, cx);
                    }
                    Err(error) => {
                        view.ui_error = Some(format!("展开状态重试失败：{error:#}；原文件已保留"))
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn close_with_ui_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_list_drag(cx);
        if self.ui_discard_pending {
            self.restore_discarded_ui();
            self.ui_discard_pending = false;
        } else if let Err(error) = self.remember_operation_state() {
            self.ui_error = Some(format!("展开状态未能保存：{error:#}"));
        }
        if self.ui_revision == self.ui_saved_revision {
            window.remove_window();
        } else {
            self.ui_closing = true;
            self.ui_due = Some(Instant::now());
            self.persist_ui_state(window, cx);
        }
    }

    pub(super) fn clear_route_ui(&mut self, path: &std::path::Path) {
        if let Ok(key) = normalize_route_key(&self.paths, path) {
            self.ui_state.remove_route(&key);
            self.ui_changed();
        }
    }

    pub(super) fn edit_sequence(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let action = match action {
            Action::Move(delta) => {
                let Some(from) = self.selected else {
                    return Ok(());
                };
                let Some(to) = from.checked_add_signed(delta as isize) else {
                    return Ok(());
                };
                Action::MovePointTo { from, to }
            }
            action => action,
        };
        if let Action::MovePointTo { from, to } = action {
            let count = self
                .document
                .as_ref()
                .map_or(0, |doc| doc.route().points.len());
            if from >= count || to >= count || from == to {
                return Ok(());
            }
        }
        self.flush(cx)?;
        self.cancel_list_drag(cx);
        self.drag_epoch = self.drag_epoch.wrapping_add(1);
        self.move_operation = None;
        let mut route = self
            .document
            .as_ref()
            .context("请选择路线")?
            .route()
            .clone();
        let route_key = normalize_route_key(&self.paths, &self.document.as_ref().unwrap().path)?;
        let mut staged_ui = self.ui_state.clone();
        let mut point_states = route
            .points
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let key = PointUiKey::for_index(&route.points, index).unwrap();
                staged_ui
                    .point(&route_key, key)
                    .map(|point| point.operations.clone())
            })
            .collect::<Vec<_>>();
        let mut states = self
            .current_ui_key()?
            .and_then(|(r, key)| {
                self.ui_state
                    .point(&r, key)
                    .map(|point| point.operations.clone())
            })
            .unwrap_or_default();
        let mut focuses = self
            .operation_fields
            .iter()
            .map(|op| (op.toggle_focus.clone(), op.drag_focus.clone()))
            .collect::<Vec<_>>();
        let mut point_focuses = self.point_drag_focus.clone();
        let selected = self.selected;
        let mut new_selected = selected;
        let mut remap_points = false;
        let mut reveal = None;
        let mut adding = false;
        let mut changing_point = false;
        let mut changed = false;
        match action {
            Action::AddPoint => {
                let id = next_point_id(&route.points)?;
                route.points.push(Point::new(id));
                new_selected = Some(route.points.len() - 1);
                point_states.push(Some(Vec::new()));
                point_focuses.push(cx.focus_handle());
                remap_points = true;
                changing_point = true;
                changed = true;
            }
            Action::CopyPoint => {
                let index = selected.context("请选择要复制的点位")?;
                let mut point = route
                    .points
                    .get(index)
                    .context("选中的点位已不存在")?
                    .clone();
                point.id = next_point_id(&route.points)?;
                point.name = copy_point_name(&point.name, &route.points);
                route.points.insert(index + 1, point);
                new_selected = Some(index + 1);
                let copied = staged_ui.copy_operations(&states)?;
                point_states.insert(index + 1, Some(copied));
                point_focuses.insert(index + 1, cx.focus_handle());
                remap_points = true;
                changing_point = true;
                changed = true;
            }
            Action::RemovePoint => {
                if let Some(index) = selected.filter(|i| *i < route.points.len()) {
                    route.points.remove(index);
                    new_selected =
                        (!route.points.is_empty()).then(|| index.min(route.points.len() - 1));
                    point_states.remove(index);
                    point_focuses.remove(index);
                    remap_points = true;
                    changing_point = true;
                    changed = true;
                }
            }
            Action::MovePointTo { from, to } => {
                move_point_slot(
                    &mut route.points,
                    &mut point_states,
                    &mut new_selected,
                    from,
                    to,
                );
                let focus = point_focuses.remove(from);
                point_focuses.insert(to, focus);
                remap_points = true;
                changed = true;
            }
            Action::AddOp(kind, at) => {
                let point = selected
                    .and_then(|index| route.points.get_mut(index))
                    .context("请选择点位")?;
                let index = at.unwrap_or(point.operations.len());
                ensure!(
                    index <= point.operations.len(),
                    "插入位置已失效，请重新选择"
                );
                let operation = Operation::new(kind, point.action_position());
                let state = staged_ui.fresh_operation(&operation)?;
                point.operations.insert(index, operation);
                states.insert(index, state);
                focuses.insert(index, (cx.focus_handle(), cx.focus_handle()));
                reveal = Some(index);
                adding = true;
                changed = true;
            }
            Action::RemoveOp(index) => {
                if let Some(point) = selected.and_then(|index| route.points.get_mut(index)) {
                    if index < point.operations.len() {
                        point.operations.remove(index);
                        states.remove(index);
                        focuses.remove(index);
                        reveal = (!point.operations.is_empty())
                            .then(|| index.min(point.operations.len() - 1));
                        changed = true;
                    }
                }
            }
            Action::MoveOpTo { from, to } => {
                if let Some(point) = selected.and_then(|index| route.points.get_mut(index)) {
                    if from < point.operations.len() && to < point.operations.len() && from != to {
                        let operation = point.operations.remove(from);
                        point.operations.insert(to, operation);
                        let state = states.remove(from);
                        states.insert(to, state);
                        let focus = focuses.remove(from);
                        focuses.insert(to, focus);
                        reveal = Some(to);
                        changed = true;
                    }
                }
            }
            _ => unreachable!(),
        }
        if !changed {
            return Ok(());
        }
        // Stage all key remapping before editing the document. Duplicate point IDs
        // change occurrence keys on move/delete, but their operation identities stay.
        if remap_points {
            remap_point_states(&mut staged_ui, &route_key, &route.points, point_states)?;
        } else if let Some(index) = new_selected {
            let key = PointUiKey::for_index(&route.points, index).context("选中的点位已不存在")?;
            staged_ui.put_point(&route_key, key, states)?;
        }
        self.document
            .as_mut()
            .unwrap()
            .edit(|current| *current = route)?;
        self.selected = new_selected;
        self.point_drag_focus = point_focuses;
        self.ui_state = staged_ui;
        if changing_point {
            self.reset_point_view();
        }
        self.ui_changed();
        // A point move keeps the same selected point and its live input entities,
        // disclosure state, keyboard focus, metadata visibility and scroll offset.
        if let Action::MovePointTo { to, .. } = action {
            self.point_scroll.scroll_to_item(to);
            if let Some(focus) = self.point_drag_focus.get(to) {
                focus.focus(window, cx);
            }
            self.status = "编排已更新，保存后生效".into();
            return Ok(());
        }
        self.sync_point(window, cx);
        if !changing_point {
            for (op, (toggle, drag)) in self.operation_fields.iter_mut().zip(focuses) {
                op.toggle_focus = toggle;
                op.drag_focus = drag;
            }
        }
        if let Some(index) = reveal {
            self.operation_scroll
                .scroll_to_top_of_item(index + self.operation_child_offset());
            let operation = &self.operation_fields[index];
            if adding {
                operation.name.read(cx).focus_handle(cx).focus(window, cx);
            } else {
                operation.drag_focus.focus(window, cx);
            }
        } else if changing_point || self.operation_fields.is_empty() {
            self.point_info_focus.focus(window, cx);
        }
        self.status = if matches!(action, Action::CopyPoint) {
            "点位已复制，可修改副本动作后保存"
        } else {
            "编排已更新，保存后生效"
        }
        .into();
        Ok(())
    }
}

/// Move a point and its optional cached action state as one slot. Selection is
/// positional, so every crossed index must follow its original point.
fn move_point_slot<T>(
    points: &mut Vec<Point>,
    slots: &mut Vec<T>,
    selected: &mut Option<usize>,
    from: usize,
    to: usize,
) {
    let point = points.remove(from);
    points.insert(to, point);
    let slot = slots.remove(from);
    slots.insert(to, slot);
    *selected = selected.map(|index| {
        if index == from {
            to
        } else if from < index && index <= to {
            index - 1
        } else if to <= index && index < from {
            index + 1
        } else {
            index
        }
    });
}

fn cache_reconciled_point(
    local: &mut ManagerUiState,
    logical_base: &mut ManagerUiState,
    route: &str,
    key: PointUiKey,
    operations: Vec<OperationUiState>,
) -> Result<()> {
    // Read-only cache population is part of the logical merge baseline, not a
    // local disclosure change. A successful disk write resets this baseline.
    if local.point(route, key) == logical_base.point(route, key) {
        logical_base.put_point(route, key, operations.clone())?;
    }
    local.put_point(route, key, operations)
}

fn remap_point_states(
    state: &mut ManagerUiState,
    route: &str,
    points: &[Point],
    slots: Vec<Option<Vec<OperationUiState>>>,
) -> Result<()> {
    ensure!(points.len() == slots.len(), "点位界面状态数量不一致");
    state.remove_route(route);
    for (index, operations) in slots.into_iter().enumerate() {
        if let Some(operations) = operations {
            let key = PointUiKey::for_index(points, index).unwrap();
            state.put_point(route, key, operations)?;
        }
    }
    Ok(())
}

fn remap_ui_baseline(
    state: &mut ManagerUiState,
    baseline: &[PointUiState],
    identities: &mut BTreeMap<u64, u64>,
) -> Result<Vec<PointUiState>> {
    let mut allocator = state.clone();
    let mut remapped = baseline.to_vec();
    let mut mapping = identities.clone();
    for point in &mut remapped {
        for saved in &mut point.operations {
            let old_id = saved.id;
            saved.id = if let Some(id) = mapping.get(&old_id) {
                *id
            } else {
                // Removed working rows still exist in the saved route. Reserve
                // fresh IDs for discard so a remote writer cannot own their IDs.
                let id = allocator.fresh_operation(&saved.descriptor)?.id;
                mapping.insert(old_id, id);
                id
            };
            if let Some(current) = state
                .routes
                .values()
                .flatten()
                .flat_map(|point| &point.operations)
                .find(|current| current.id == saved.id)
            {
                saved.expanded = current.expanded;
            }
            // Keep the saved descriptor, even if the working action was edited.
        }
    }
    state.next_id = allocator.next_id;
    *identities = mapping;
    Ok(remapped)
}

fn merge_ui_changes(
    base: &ManagerUiState,
    local: &ManagerUiState,
    mut remote: ManagerUiState,
) -> Result<(ManagerUiState, BTreeMap<u64, u64>)> {
    let mut identities = BTreeMap::new();
    let routes = base
        .routes
        .keys()
        .chain(local.routes.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    for route in routes {
        let previous = base.routes.get(&route);
        let current = local.routes.get(&route);
        if previous == current {
            continue;
        }
        let Some(current) = current else {
            remote.remove_route(&route);
            continue;
        };
        let keys = previous
            .into_iter()
            .flatten()
            .map(|point| point.key)
            .chain(current.iter().map(|point| point.key))
            .collect::<std::collections::BTreeSet<_>>();
        for key in keys {
            let before = base.point(&route, key);
            let after = local.point(&route, key);
            if before == after {
                continue;
            }
            if let Some(after) = after {
                let operations = remote.copy_operations(&after.operations)?;
                identities.extend(
                    after
                        .operations
                        .iter()
                        .zip(&operations)
                        .map(|(old, new)| (old.id, new.id)),
                );
                remote.put_point(&route, key, operations)?;
            } else {
                remote.remove_point(&route, &key);
            }
        }
    }
    // Unchanged local records may have been changed by the other writer. Match
    // their live identities within the same point, consuming duplicate content.
    for (route, points) in &local.routes {
        for point in points {
            let Some(merged) = remote.point(route, point.key) else {
                continue;
            };
            let mut consumed = std::collections::BTreeSet::new();
            for operation in &point.operations {
                if identities.contains_key(&operation.id) {
                    continue;
                }
                let matching = merged
                    .operations
                    .iter()
                    .find(|candidate| {
                        !consumed.contains(&candidate.id)
                            && candidate.id == operation.id
                            && candidate.descriptor == operation.descriptor
                    })
                    .or_else(|| {
                        merged.operations.iter().find(|candidate| {
                            !consumed.contains(&candidate.id)
                                && candidate.descriptor == operation.descriptor
                        })
                    });
                if let Some(matching) = matching {
                    consumed.insert(matching.id);
                    identities.insert(operation.id, matching.id);
                }
            }
        }
    }
    Ok((remote, identities))
}

fn next_point_id(points: &[Point]) -> Result<u64> {
    points
        .iter()
        .map(|point| point.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .context("点位 ID 已达到上限，无法新增点位")
}

fn copy_point_name(name: &str, points: &[Point]) -> String {
    let base = if name.trim().is_empty() {
        "点位"
    } else {
        name
    };
    let mut ordinal = 1usize;
    loop {
        let candidate = if ordinal == 1 {
            format!("{base}（副本）")
        } else {
            format!("{base}（副本 {ordinal}）")
        };
        if !points.iter().any(|point| point.name == candidate) {
            return candidate;
        }
        ordinal += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(name: &str) -> Operation {
        let mut operation = Operation::new(OpKind::Wait, [0.5, 0.5]);
        operation.name = name.into();
        operation
    }

    fn add_point(state: &mut ManagerUiState, route: &str, key: PointUiKey, name: &str) {
        let row = state.fresh_operation(&operation(name)).unwrap();
        state.put_point(route, key, vec![row]).unwrap();
    }

    #[test]
    fn merge_keeps_remote_unchanged_points_and_applies_local_deletions() {
        let key = PointUiKey::new(1, 0);
        let deleted_key = PointUiKey::new(2, 0);
        let mut base = ManagerUiState::default();
        add_point(&mut base, "routes/a.yaml", key, "local");
        add_point(&mut base, "routes/a.yaml", deleted_key, "delete point");
        add_point(&mut base, "routes/b.yaml", key, "remote");
        add_point(&mut base, "routes/c.yaml", key, "delete route");
        let mut local = base.clone();
        let mut rows = local
            .point("routes/a.yaml", key)
            .unwrap()
            .operations
            .clone();
        rows[0].expanded = false;
        local.put_point("routes/a.yaml", key, rows).unwrap();
        local.remove_point("routes/a.yaml", &deleted_key);
        local.remove_route("routes/c.yaml");
        let mut remote = base.clone();
        let mut rows = remote
            .point("routes/b.yaml", key)
            .unwrap()
            .operations
            .clone();
        rows[0].expanded = false;
        remote.put_point("routes/b.yaml", key, rows).unwrap();
        let expected_remote = remote.point("routes/b.yaml", key).unwrap().clone();
        let (merged, identities) = merge_ui_changes(&base, &local, remote).unwrap();
        assert_eq!(merged.point("routes/b.yaml", key), Some(&expected_remote));
        assert!(!merged.point("routes/a.yaml", key).unwrap().operations[0].expanded);
        assert!(merged.point("routes/a.yaml", deleted_key).is_none());
        assert!(!merged.routes.contains_key("routes/c.yaml"));
        let old = local.point("routes/a.yaml", key).unwrap().operations[0].id;
        assert_eq!(
            identities[&old],
            merged.point("routes/a.yaml", key).unwrap().operations[0].id
        );
        merged.validate().unwrap();
    }

    #[test]
    fn retry_remaps_baseline_and_reserves_removed_rows_without_id_collisions() {
        let key = PointUiKey::new(1, 0);
        let mut local = ManagerUiState::default();
        let rows = local
            .reconcile_point(
                "routes/a.yaml",
                key,
                &[operation("saved name"), operation("removed working row")],
            )
            .unwrap();
        local.put_point("routes/a.yaml", key, rows).unwrap();
        let baseline = local.routes["routes/a.yaml"].clone();
        let mut edited = baseline[0].operations.clone();
        edited[0].descriptor.name = "unsaved name".into();
        edited[0].expanded = false;
        edited.remove(1);
        local.put_point("routes/a.yaml", key, edited).unwrap();
        let mut remote = ManagerUiState::default();
        // Another writer started from the same missing file and owns IDs 1/2.
        add_point(&mut remote, "routes/b.yaml", key, "remote one");
        add_point(&mut remote, "routes/c.yaml", key, "remote two");
        let (mut merged, mut identities) =
            merge_ui_changes(&ManagerUiState::default(), &local, remote).unwrap();
        let restored = remap_ui_baseline(&mut merged, &baseline, &mut identities).unwrap();
        assert_eq!(restored[0].operations[0].descriptor.name, "saved name");
        assert!(!restored[0].operations[0].expanded);
        assert_eq!(
            restored[0].operations[1].descriptor.name,
            "removed working row"
        );
        assert_eq!(
            restored[0].operations[0].id,
            identities[&baseline[0].operations[0].id]
        );
        assert_eq!(
            restored[0].operations[1].id,
            identities[&baseline[0].operations[1].id]
        );
        assert!(restored[0].operations.iter().all(|row| row.id > 2));
        assert_ne!(restored[0].operations[0].id, restored[0].operations[1].id);
        merged.remove_route("routes/a.yaml");
        merged
            .put_point("routes/a.yaml", key, restored[0].operations.clone())
            .unwrap();
        merged.validate().unwrap();
        let fresh = merged.fresh_operation(&operation("later")).unwrap();
        assert!(fresh.id > restored[0].operations[1].id);
    }

    #[test]
    fn readonly_cache_is_a_logical_baseline_and_does_not_override_remote_choice() {
        let key = PointUiKey::new(1, 0);
        let mut local = ManagerUiState::default();
        let mut base = ManagerUiState::default();
        let visited = local
            .reconcile_point("routes/b.yaml", key, &[operation("visited")])
            .unwrap();
        cache_reconciled_point(&mut local, &mut base, "routes/b.yaml", key, visited).unwrap();
        assert_eq!(
            local.point("routes/b.yaml", key),
            base.point("routes/b.yaml", key)
        );
        add_point(&mut local, "routes/a.yaml", key, "actually changed");
        let mut remote = ManagerUiState::default();
        add_point(&mut remote, "routes/b.yaml", key, "visited");
        let mut rows = remote
            .point("routes/b.yaml", key)
            .unwrap()
            .operations
            .clone();
        rows[0].expanded = false;
        remote.put_point("routes/b.yaml", key, rows).unwrap();
        let expected = remote.point("routes/b.yaml", key).unwrap().clone();
        let (merged, _) = merge_ui_changes(&base, &local, remote).unwrap();
        assert_eq!(merged.point("routes/b.yaml", key), Some(&expected));
    }

    #[test]
    fn readonly_reconciliation_keeps_actual_local_changes_out_of_baseline() {
        let key = PointUiKey::new(1, 0);
        let mut local = ManagerUiState::default();
        add_point(&mut local, "routes/a.yaml", key, "action");
        let mut base = local.clone();
        let mut rows = local
            .point("routes/a.yaml", key)
            .unwrap()
            .operations
            .clone();
        rows[0].expanded = false;
        local.put_point("routes/a.yaml", key, rows.clone()).unwrap();
        cache_reconciled_point(&mut local, &mut base, "routes/a.yaml", key, rows).unwrap();
        assert!(base.point("routes/a.yaml", key).unwrap().operations[0].expanded);
        assert!(!local.point("routes/a.yaml", key).unwrap().operations[0].expanded);
    }

    #[test]
    fn point_moves_preserve_selection_contents_and_crossed_order() {
        // Duplicate IDs cannot identify the selected point: use distinct content
        // to assert that every possible selection follows its original slot.
        let originals = (0..5)
            .map(|index| {
                let mut point = Point::new(if index % 2 == 0 { 8 } else { 3 });
                point.name = format!("point {index}");
                point.operations = vec![operation(&format!("action {index}"))];
                point
            })
            .collect::<Vec<_>>();
        for from in 0..5 {
            for to in 0..5 {
                for selection in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
                    let mut points = originals.clone();
                    let mut slots = (0..5).collect::<Vec<_>>();
                    let mut selected = selection;
                    move_point_slot(&mut points, &mut slots, &mut selected, from, to);
                    assert_eq!(slots[to], from);
                    let mut crossed = slots.clone();
                    crossed.remove(to);
                    assert_eq!(crossed, (0..5).filter(|i| *i != from).collect::<Vec<_>>());
                    for (point, original) in points.iter().zip(slots) {
                        assert_eq!(point, &originals[original]);
                    }
                    assert_eq!(
                        selected.map(|i| &points[i]),
                        selection.map(|i| &originals[i])
                    );
                }
            }
        }
    }

    #[test]
    fn duplicate_point_remapping_preserves_rows_and_removes_stale_occurrences() {
        let route = "routes/a.yaml";
        let mut points = vec![Point::new(8), Point::new(3), Point::new(8)];
        let mut state = ManagerUiState::default();
        for index in 0..points.len() {
            let key = PointUiKey::for_index(&points, index).unwrap();
            add_point(&mut state, route, key, &format!("point {index}"));
        }
        let mut slots = (0..points.len())
            .map(|index| {
                state
                    .point(route, PointUiKey::for_index(&points, index).unwrap())
                    .map(|point| point.operations.clone())
            })
            .collect::<Vec<_>>();
        let first_id = slots[0].as_ref().unwrap()[0].id;
        let last_id = slots[2].as_ref().unwrap()[0].id;
        slots[2].as_mut().unwrap()[0].expanded = false;
        let mut selected = Some(0);
        move_point_slot(&mut points, &mut slots, &mut selected, 2, 0);
        assert_eq!(selected, Some(1));
        remap_point_states(&mut state, route, &points, slots.clone()).unwrap();
        assert!(
            !state
                .point(route, PointUiKey::new(8, 0))
                .unwrap()
                .operations[0]
                .expanded
        );
        assert_eq!(
            state
                .point(route, PointUiKey::new(8, 0))
                .unwrap()
                .operations[0]
                .id,
            last_id
        );
        assert_eq!(
            state
                .point(route, PointUiKey::new(8, 1))
                .unwrap()
                .operations[0]
                .id,
            first_id
        );
        points.remove(0);
        slots.remove(0);
        remap_point_states(&mut state, route, &points, slots).unwrap();
        assert_eq!(
            state
                .point(route, PointUiKey::new(8, 0))
                .unwrap()
                .operations[0]
                .id,
            first_id
        );
        assert!(state.point(route, PointUiKey::new(8, 1)).is_none());
        state.validate().unwrap();
    }

    #[test]
    pub(super) fn copy_names_avoid_collisions_and_ids_check_overflow() {
        let mut points = vec![Point::new(1), Point::new(2), Point::new(3)];
        points[0].name = "采集".into();
        points[1].name = "采集（副本）".into();
        points[2].name = "采集（副本 2）".into();
        assert_eq!(copy_point_name("采集", &points), "采集（副本 3）");
        assert_eq!(copy_point_name("", &points), "点位（副本）");
        assert_eq!(next_point_id(&points).unwrap(), 4);
        points[1].id = u64::MAX;
        assert!(next_point_id(&points).is_err());
    }
}
