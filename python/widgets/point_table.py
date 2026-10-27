# -*- coding: utf-8 -*-
"""点位表格化编辑视图：点位主表 + 操作子表 + 类型化参数编辑。

设计：
- PointsTableWidget   点位主表（id/名称/坐标/操作数），增删/上下移动
- OpsTableWidget      选中点位的操作子表，按操作类型动态渲染参数编辑器
- 双视图同步由 manager 负责；本组件只持有 Route 模型的可变引用并整段重写 points
"""
from __future__ import annotations

from typing import Optional

from PySide6.QtCore import Qt, Signal, QTimer
from PySide6.QtWidgets import (
    QAbstractItemView,
    QComboBox,
    QDoubleSpinBox,
    QFrame,
    QHBoxLayout,
    QHeaderView,
    QInputDialog,
    QLabel,
    QLineEdit,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QSplitter,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from core.route_model import OP_TYPES, Op, Point, Route, save_route_yaml  # noqa: F402

# 每种操作类型需要的参数定义: name -> (label, kind, default)
OP_PARAM_DEFS: dict[str, list[tuple[str, str, str]]] = {
    "click": [("pos", "坐标[x,y]", "pair", "0.5,0.5"), ("button", "鼠标键", "button", "left"), ("clicks", "连击次数", "int:1", "1"), ("jitter", "抖动px", "int:0", "0")],
    "key": [("key", "键名", "str", "m"), ("up", "只松开", "bool", "false"), ("jitter", "抖动px", "int:0", "0")],
    "press": [("key", "键名", "str", "w"), ("ms", "按住ms", "int:1", "500"), ("jitter", "抖动px", "int:0", "0")],
    "wheel": [("delta", "滚轮格数", "int:0", "-3"), ("jitter", "抖动px", "int:0", "0")],
    "drag": [("from", "起点[x,y]", "pair", "0.1,0.1"), ("to", "终点[x,y]", "pair", "0.2,0.2"), ("duration", "时长ms", "int:1", "200"), ("jitter", "抖动px", "int:0", "0")],
    "wait": [("ms", "等待ms", "int:1", "300")],
}


def _make_editor(kind: str, value, default: str):
    """按 kind 生成参数编辑器，返回 (widget, getter)。"""
    if kind == "button":
        w = QComboBox()
        w.addItems(["left", "right", "middle"])
        w.setCurrentText(str(value) if value is not None else default)
        return w, w.currentText
    if kind == "bool":
        w = QComboBox()
        w.addItems(["false", "true"])
        w.setCurrentText("true" if value else "false")
        return w, lambda: w.currentText() == "true"
    if kind == "pair":
        w = QLineEdit(str(default if value is None else ",".join(str(v) for v in value)))
        w.setPlaceholderText("0.5,0.5")
        return w, w.text
    if kind.startswith("int"):
        _, minv = kind.split(":")
        w = QSpinBox()
        w.setRange(int(minv), 999999)
        w.setValue(int(value) if value is not None else int(default))
        return w, lambda: int(w.value())
    if kind.startswith("float"):
        w = QDoubleSpinBox()
        w.setRange(0.0, 1.0)
        w.setDecimals(4)
        w.setValue(float(value) if value is not None else float(default))
        return w, lambda: float(w.value())
    # str
    w = QLineEdit(str(value) if value is not None else default)
    return w, w.text


def _parse_pair(text: str) -> tuple[float, float] | None:
    try:
        s = str(text).strip().strip("[]()（）").replace("，", ",").replace(" ", "")
        xs, ys = s.split(",")
        x, y = float(xs), float(ys)
        if 0.0 <= x <= 1.0 and 0.0 <= y <= 1.0:
            return (x, y)
    except Exception:
        pass
    return None


class RouteEditor(QWidget):
    """表格编辑视图：上半点位表、下半操作表。routeChanged 在模型被修改后发射。"""

    routeChanged = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.route: Optional[Route] = None
        self._loading = False

        lay = QVBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        splitter = QSplitter(Qt.Vertical)

        # ---------- 点位主表 ----------
        pts_widget = QWidget()
        pts_lay = QVBoxLayout(pts_widget)
        pts_lay.setContentsMargins(0, 0, 0, 0)
        pts_btns = QHBoxLayout()
        btn_add_pt = QPushButton("+ 点位")
        btn_add_pt.clicked.connect(self.add_point)
        btn_del_pt = QPushButton("删除点位")
        btn_del_pt.clicked.connect(self.delete_point)
        btn_up = QPushButton("上移")
        btn_up.clicked.connect(lambda: self.move_point(-1))
        btn_down = QPushButton("下移")
        btn_down.clicked.connect(lambda: self.move_point(1))
        pts_btns.addWidget(btn_add_pt)
        pts_btns.addWidget(btn_del_pt)
        pts_btns.addWidget(btn_up)
        pts_btns.addWidget(btn_down)
        pts_btns.addStretch(1)
        pts_lay.addLayout(pts_btns)

        self.tbl_points = QTableWidget(0, 4)
        self.tbl_points.setHorizontalHeaderLabels(["id", "名称", "坐标 [x, y]", "操作数"])
        self.tbl_points.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeToContents)
        self.tbl_points.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.tbl_points.currentCellChanged.connect(lambda row, _c, _pr, _pc: self._on_point_row_changed(row))
        self.tbl_points.itemChanged.connect(self._on_point_item_changed)
        pts_lay.addWidget(self.tbl_points)

        # ---------- 操作子表 ----------
        ops_widget = QWidget()
        ops_lay = QVBoxLayout(ops_widget)
        ops_lay.setContentsMargins(0, 0, 0, 0)
        ops_btns = QHBoxLayout()
        self.cmb_op_type = QComboBox()
        self.cmb_op_type.addItems(list(OP_TYPES))
        btn_add_op = QPushButton("+ 操作")
        btn_add_op.clicked.connect(self.add_op)
        btn_del_op = QPushButton("删除操作")
        btn_del_op.clicked.connect(self._delete_last_op)
        ops_btns.addWidget(QLabel("新操作类型:"))
        ops_btns.addWidget(self.cmb_op_type)
        ops_btns.addWidget(btn_add_op)
        ops_btns.addWidget(btn_del_op)
        ops_btns.addStretch(1)
        ops_lay.addLayout(ops_btns)

        self.ops_container = QWidget()
        self.ops_grid = QVBoxLayout(self.ops_container)
        self.ops_grid.setContentsMargins(0, 0, 0, 0)

        # 操作列表套滚动区：动作多时在固定高度内滚动，不再撑高窗口
        self.ops_scroll = QScrollArea()
        self.ops_scroll.setWidget(self.ops_container)
        self.ops_scroll.setWidgetResizable(True)
        self.ops_scroll.setFrameShape(QFrame.NoFrame)
        self.ops_scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        ops_lay.addWidget(self.ops_scroll, 1)

        splitter.addWidget(pts_widget)
        splitter.addWidget(ops_widget)
        splitter.setStretchFactor(0, 3)
        splitter.setStretchFactor(1, 2)
        lay.addWidget(splitter)

        self._editors: list[tuple[Op, str, object, object]] = []  # (op, param, widget, getter)

    # ------------------------------------------------------------------
    def load_route(self, route: Route):
        self._loading = True
        self.route = route
        self.tbl_points.setRowCount(0)
        for p in route.points:
            self._append_point_row(p)
        self._loading = False
        if route.points:
            self.tbl_points.selectRow(0)
        else:
            self._render_ops(None)

    def _append_point_row(self, p: Point):
        r = self.tbl_points.rowCount()
        self.tbl_points.insertRow(r)
        self.tbl_points.setItem(r, 0, QTableWidgetItem(str(p.id)))
        name_item = QTableWidgetItem(p.name)
        name_item.setToolTip("双击编辑名称")
        self.tbl_points.setItem(r, 1, name_item)
        self.tbl_points.setItem(r, 2, QTableWidgetItem(f"[{p.pos[0]:.4f}, {p.pos[1]:.4f}]"))
        self.tbl_points.setItem(r, 3, QTableWidgetItem(str(len(p.operations))))

    def _current_point(self) -> Optional[Point]:
        if not self.route:
            return None
        r = self.tbl_points.currentRow()
        if 0 <= r < len(self.route.points):
            return self.route.points[r]
        return None

    # ---------- 点位操作 ----------
    def add_point(self):
        if not self.route:
            return
        nid = max((p.id for p in self.route.points), default=0) + 1
        pos, ok = QInputDialog.getText(self, "新点位", "归一化坐标 x,y（0~1）:", text="0.5,0.5")
        if not ok:
            return
        pair = _parse_pair(pos)
        if not pair:
            QInputDialog.getText(self, "提示", "坐标格式错误，示例: 0.5,0.5")
            return
        p = Point(id=nid, pos=pair, name=f"点{nid}")
        self.route.points.append(p)
        self._append_point_row(p)
        self.tbl_points.selectRow(self.tbl_points.rowCount() - 1)
        self.routeChanged.emit()

    def delete_point(self):
        if not self.route:
            return
        r = self.tbl_points.currentRow()
        if r < 0 or r >= len(self.route.points):
            return
        del self.route.points[r]
        self.tbl_points.removeRow(r)
        self.routeChanged.emit()

    def move_point(self, delta: int):
        if not self.route:
            return
        r = self.tbl_points.currentRow()
        n = len(self.route.points)
        t = r + delta
        if r < 0 or t < 0 or t >= n:
            return
        pts = self.route.points
        pts[r], pts[t] = pts[t], pts[r]
        self._reload_rows()
        self.tbl_points.selectRow(t)
        self.routeChanged.emit()

    def _reload_rows(self):
        self._loading = True
        self.tbl_points.setRowCount(0)
        for p in self.route.points:
            self._append_point_row(p)
        self._loading = False

    def _on_point_item_changed(self, item: QTableWidgetItem):
        if self._loading or not self.route:
            return
        r, c = item.row(), item.column()
        if r >= len(self.route.points):
            return
        p = self.route.points[r]
        if c == 1:
            p.name = item.text()
            self.routeChanged.emit()
        # id/坐标/操作数列只读展示（id 与坐标编辑走模型，后续可扩展）

    def _on_point_row_changed(self, row: int):
        self._render_ops(self._current_point())

    # ---------- 操作编辑 ----------
    def _render_ops(self, point: Optional[Point]):
        # 清空旧编辑器
        while self.ops_grid.count():
            item = self.ops_grid.takeAt(0)
            w = item.widget()
            if w:
                w.deleteLater()
        self._editors = []
        if point is None:
            ph = QLabel("← 选择一个点位后在这里编辑操作序列")
            ph.setStyleSheet("color: gray;")
            self.ops_grid.addWidget(ph)
            return
        if not point.operations:
            ph = QLabel("该点位暂无操作，从上方选择类型添加")
            ph.setStyleSheet("color: gray;")
            self.ops_grid.addWidget(ph)
            return

        for idx, op in enumerate(point.operations):
            frame = QWidget()
            frame.setStyleSheet("QWidget { border: 1px solid #ddd; border-radius: 4px; }")
            v = QVBoxLayout(frame)
            v.setContentsMargins(6, 4, 6, 4)
            head = QHBoxLayout()
            head.addWidget(QLabel(f"<b>#{idx + 1} {op.type}</b>"))
            btn_up = QPushButton("↑")
            btn_up.setFixedWidth(28)
            btn_up.clicked.connect(lambda _, i=idx: self._move_op(point, i, -1))
            btn_down = QPushButton("↓")
            btn_down.setFixedWidth(28)
            btn_down.clicked.connect(lambda _, i=idx: self._move_op(point, i, 1))
            btn_del = QPushButton("×")
            btn_del.setFixedWidth(28)
            btn_del.clicked.connect(lambda _, i=idx: self._delete_op(point, i))
            head.addStretch(1)
            head.addWidget(btn_up)
            head.addWidget(btn_down)
            head.addWidget(btn_del)
            v.addLayout(head)

            grid = QHBoxLayout()
            for pname, label, kind, default in OP_PARAM_DEFS[op.type]:
                cell = QVBoxLayout()
                cell.addWidget(QLabel(label))
                w, getter = _make_editor(kind, op.params.get(pname), default)
                # 参数变更 → 写回模型
                def _apply(_=None, op=op, pname=pname, kind=kind, getter=getter):
                    self._apply_param(op, pname, kind, getter())
                if isinstance(w, QComboBox):
                    w.currentTextChanged.connect(_apply)
                elif isinstance(w, QLineEdit):
                    w.textChanged.connect(_apply)
                else:
                    w.valueChanged.connect(_apply)
                self._editors.append((op, pname, w, getter))
                cell.addWidget(w)
                grid.addLayout(cell)
            grid.addStretch(1)
            v.addLayout(grid)
            self.ops_grid.addWidget(frame)
        self.ops_grid.addStretch(1)

    def _apply_param(self, op: Op, pname: str, kind: str, value):
        if kind == "pair":
            pair = _parse_pair(str(value))
            if pair:
                op.params[pname] = list(pair)
        elif pname == "jitter":
            if int(value) > 0:
                op.params["jitter"] = int(value)
            else:
                op.params.pop("jitter", None)
        elif pname == "up":
            op.params["up"] = bool(value)
        elif pname in ("clicks",):
            op.params["clicks"] = int(value)
        elif pname in ("pos", "from", "to"):
            pass  # pair 已处理
        elif pname in ("duration", "ms", "delta"):
            op.params[pname] = int(value)
        else:  # key / button
            op.params[pname] = str(value)
        self.routeChanged.emit()

    def add_op(self):
        p = self._current_point()
        if not p:
            return
        t = self.cmb_op_type.currentText()
        op = Op(type=t, params={})
        # 填充默认值（与 OP_PARAM_DEFS 一致）
        for pname, label, kind, default in OP_PARAM_DEFS[t]:
            if kind == "pair":
                op.params[pname] = list(_parse_pair(default))
            elif kind == "int:0":
                op.params[pname] = int(default) if pname != "jitter" else 0
            elif kind == "int:1":
                op.params[pname] = int(default)
            elif kind == "bool":
                op.params["up"] = default == "true"
            elif kind == "button":
                op.params["button"] = default
            else:
                op.params[pname] = default
        if "jitter" in op.params and not op.params["jitter"]:
            op.params.pop("jitter")
        p.operations.append(op)
        self._update_op_count()
        self._render_ops(p)
        self.routeChanged.emit()
        # 新卡片渲染在列表末尾，滚动到底部让它可见
        def _scroll_bottom():
            sb = self.ops_scroll.verticalScrollBar()
            sb.setValue(sb.maximum())
        QTimer.singleShot(0, _scroll_bottom)

    def _delete_op(self, point: Point, idx: int):
        del point.operations[idx]
        self._update_op_count()
        self._render_ops(point)
        self.routeChanged.emit()

    def _delete_last_op(self):
        p = self._current_point()
        if p and p.operations:
            self._delete_op(p, len(p.operations) - 1)

    def _move_op(self, point: Point, idx: int, delta: int):
        t = idx + delta
        if t < 0 or t >= len(point.operations):
            return
        ops = point.operations
        ops[idx], ops[t] = ops[t], ops[idx]
        self._render_ops(point)
        self.routeChanged.emit()

    def _update_op_count(self):
        r = self.tbl_points.currentRow()
        if self.route and 0 <= r < len(self.route.points) and r < self.tbl_points.rowCount():
            p = self.route.points[r]
            self.tbl_points.setItem(r, 3, QTableWidgetItem(str(len(p.operations))))
