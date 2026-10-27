# -*- coding: utf-8 -*-
"""取点工具：选目标窗口 → 实时相对坐标 → 快捷键采点 → 导出路线 YAML。

界面布局：
  上：目标窗口选择（下拉刷新 + 十字准星瞄准按钮）
  中：实时坐标区（物理/相对/归一化/客户区尺寸）+ 已采点位表
  下：保存/导出按钮 + 状态栏
"""
from __future__ import annotations

import ctypes
import sys
from ctypes import wintypes
from pathlib import Path

from PySide6.QtCore import Qt, QThread, QTimer, Signal
from PySide6.QtGui import QCursor, QKeySequence, QShortcut
from PySide6.QtWidgets import (
    QApplication,
    QComboBox,
    QFileDialog,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QMainWindow,
    QMessageBox,
    QPushButton,
    QStatusBar,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from core import window_utils as wu  # noqa: E402
from core.hotkey_service import HotkeyService  # noqa: E402
from core.route_model import Point, Route, save_route_yaml  # noqa: E402

ROUTES_DIR = Path(__file__).resolve().parents[1] / "routes"

MOD_ALT = 0x0001
MOD_CONTROL = 0x0002
WM_HOTKEY = 0x0312
CAPTURE_HOTKEY_ID = 0x5C4A  # 进程内唯一即可
_VK_P = 0x50


class _RegisterHotKeyThread(QThread):
    """在独立线程注册系统热键并泵消息（已实测：本机可靠收到 WM_HOTKEY）。

    triggered 经 queued connection 自动回到 GUI 线程。
    """

    triggered = Signal()
    registered = Signal(bool)

    def __init__(self, hotkey_id: int, mods: int, vk: int, parent=None):
        super().__init__(parent)
        self._id = hotkey_id
        self._mods = mods
        self._vk = vk
        self._stop = False

    def run(self):
        u32 = ctypes.windll.user32
        ok = bool(u32.RegisterHotKey(None, self._id, self._mods, self._vk))
        self.registered.emit(ok)
        if not ok:
            return
        msg = wintypes.MSG()
        while not self._stop:
            # PM_REMOVE=1；PeekMessage 非阻塞，10ms 轮询（实测可稳定收到注入与真实按键）
            if u32.PeekMessageW(ctypes.byref(msg), None, 0, 0, 1):
                if msg.message == WM_HOTKEY and msg.wParam == self._id:
                    self.triggered.emit()
                continue  # 立刻再取，清空队列
            self.msleep(10)
        u32.UnregisterHotKey(None, self._id)

    def shutdown(self):
        self._stop = True
        self.wait(2000)


class CrosshairButton(QPushButton):
    """Spy++ 风格：按住后鼠标变十字，松开时瞄准光标下的窗口。"""

    picked = Signal(object)  # WindowInfo

    def __init__(self, parent=None):
        super().__init__("⊙ 瞄准窗口（按住拖到目标上松开）", parent)
        self.setCheckable(True)

    def mouseReleaseEvent(self, e):
        if self.isChecked():
            pos = QCursor.pos()
            info = wu.window_from_point(pos.x(), pos.y())
            if info:
                self.picked.emit(info)
            self.setChecked(False)
        super().mouseReleaseEvent(e)


class PickerWindow(QMainWindow):
    #: keyboard 回调线程 → GUI 线程的跨线程投递（queued connection）
    _captureRequested = Signal()

    def __init__(self):
        super().__init__()
        self.setWindowTitle("game_helper v2 · 取点工具")
        self.resize(720, 520)
        self._points: list[Point] = []
        self._target: wu.WindowInfo | None = None
        self._hotkeys = HotkeyService()
        self._next_id = 1

        central = QWidget()
        self.setCentralWidget(central)
        lay = QVBoxLayout(central)

        # --- 目标窗口选择 ---
        top = QHBoxLayout()
        self.cmb_windows = QComboBox()
        self.cmb_windows.setMinimumWidth(380)
        btn_refresh = QPushButton("刷新")
        btn_refresh.clicked.connect(self.refresh_windows)
        self.btn_aim = CrosshairButton()
        self.btn_aim.picked.connect(self.on_aim_picked)
        top.addWidget(QLabel("目标窗口:"))
        top.addWidget(self.cmb_windows, 1)
        top.addWidget(btn_refresh)
        top.addWidget(self.btn_aim)
        lay.addLayout(top)

        # --- 实时坐标 ---
        coords = QHBoxLayout()
        self.lbl_mouse = QLabel("屏幕: -")
        self.lbl_rel = QLabel("相对: -")
        self.lbl_norm = QLabel("归一化: -")
        self.lbl_client = QLabel("客户区: -")
        for l in (self.lbl_mouse, self.lbl_rel, self.lbl_norm, self.lbl_client):
            l.setTextInteractionFlags(Qt.TextSelectableByMouse)
            coords.addWidget(l)
        lay.addLayout(coords)

        # --- 已采点位 ---
        self.tbl = QTableWidget(0, 5)
        self.tbl.setHorizontalHeaderLabels(["id", "名称", "归一化 [x, y]", "操作数", "操作"])
        self.tbl.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeToContents)
        self.tbl.setEditTriggers(QTableWidget.DoubleClicked | QTableWidget.SelectedClicked)
        lay.addWidget(self.tbl, 1)

        # --- 按钮 ---
        btns = QHBoxLayout()
        self.btn_capture = QPushButton("捕获当前点 (Ctrl+Alt+P)")
        self.btn_capture.clicked.connect(self.capture_point)
        btn_del = QPushButton("删除选中")
        btn_del.clicked.connect(self.delete_selected)
        btn_save = QPushButton("保存路线到 routes/")
        btn_save.clicked.connect(self.save_route)
        btn_export = QPushButton("导出到...")
        btn_export.clicked.connect(self.export_route)
        btns.addWidget(self.btn_capture)
        btns.addWidget(btn_del)
        btns.addStretch(1)
        btns.addWidget(btn_save)
        btns.addWidget(btn_export)
        lay.addLayout(btns)

        self.statusBar().showMessage("就绪 · 先选择目标窗口")

        # 定时器：实时坐标
        self._timer = QTimer(self)
        self._timer.timeout.connect(self.update_coords)
        self._timer.start(50)

        # 局部快捷键（GUI 内）
        QShortcut(QKeySequence("Ctrl+Alt+P"), self, activated=self.capture_point)
        # 全局热键（即使窗口不在前台也能采点）
        # 信号：跨线程/系统回调 → GUI 线程（queued connection）
        self._captureRequested.connect(self.capture_point)
        # 主路径：Windows 原生 RegisterHotKey（独立线程注册+泵消息，实测可靠；
        # keyboard 库的 LL 钩子在部分环境下收不到事件，仅作退路）
        self._hk_thread = _RegisterHotKeyThread(CAPTURE_HOTKEY_ID, MOD_CONTROL | MOD_ALT, _VK_P, self)
        self._hk_thread.triggered.connect(self._captureRequested.emit)
        self._hk_thread.registered.connect(self._on_native_hotkey_result)
        self._hk_thread.start()

        self.refresh_windows()

    # ---------------- 窗口选择 ----------------
    def refresh_windows(self):
        self.cmb_windows.clear()
        self._windows = wu.list_windows()
        for w in self._windows:
            self.cmb_windows.addItem(w.label, w.hwnd)
        self.statusBar().showMessage(f"已列出 {len(self._windows)} 个可见窗口")

    def on_aim_picked(self, info: wu.WindowInfo):
        # 下拉里选中瞄准到的窗口
        idx = self.cmb_windows.findData(info.hwnd)
        if idx >= 0:
            self.cmb_windows.setCurrentIndex(idx)
        else:
            self.cmb_windows.addItem(info.label, info.hwnd)
            self.cmb_windows.setCurrentIndex(self.cmb_windows.count() - 1)
        self._select_target(info)

    def _current_window(self) -> wu.WindowInfo | None:
        idx = self.cmb_windows.currentIndex()
        if idx < 0:
            return None
        for w in getattr(self, "_windows", []):
            if w.hwnd == self.cmb_windows.itemData(idx):
                return w
        return None

    def _select_target(self, info: wu.WindowInfo):
        self._target = info
        area = wu.get_client_area(info.hwnd)
        if area:
            self.lbl_client.setText(f"客户区: {area.width}x{area.height} @({area.left},{area.top})")
        self.statusBar().showMessage(f"目标: {info.label}")

    # ---------------- 坐标轮询 ----------------
    def update_coords(self):
        w = self._current_window()
        if w and w.hwnd != (self._target.hwnd if self._target else None):
            self._select_target(w)
        if not self._target:
            return
        area = wu.get_client_area(self._target.hwnd)
        if not area:
            return
        x, y = wu.mouse_pos()
        (rx, ry), (nx, ny) = wu.rel_and_norm(x, y, area)
        self.lbl_mouse.setText(f"屏幕: ({x}, {y})")
        self.lbl_rel.setText(f"相对: ({rx}, {ry})")
        self.lbl_norm.setText(f"归一化: ({nx:.4f}, {ny:.4f})")

    # ---------------- 采点 ----------------
    def capture_point(self):
        if not self._target:
            # 全局热键下不能弹模态框（会抢焦点），用状态栏提示
            self.statusBar().showMessage("先选择目标窗口，再按 Ctrl+Alt+P 捕获")
            self.activateWindow()
            return
        area = wu.get_client_area(self._target.hwnd)
        if not area:
            return
        x, y = wu.mouse_pos()
        (rx, ry), (nx, ny) = wu.rel_and_norm(x, y, area)
        pt = Point(id=self._next_id, pos=(nx, ny), name=f"点{self._next_id}")
        self._next_id += 1
        self._points.append(pt)
        self._add_row(pt)
        self.statusBar().showMessage(f"已捕获 #{pt.id}  norm=({nx:.4f}, {ny:.4f})")

    def _global_capture(self):
        # keyboard 库回调线程 → 通过信号转回 GUI 线程（queued connection）
        self._captureRequested.emit()

    def _on_native_hotkey_result(self, ok: bool):
        if ok:
            self.statusBar().showMessage("就绪 · 全局热键 Ctrl+Alt+P 已注册（先选择目标窗口）")
        else:
            # 退路：keyboard 库 LL 钩子（回调在其监听线程，仍经信号转回 GUI 线程）
            kb_ok = self._hotkeys.register("ctrl+alt+p", self._global_capture)
            self.statusBar().showMessage(
                "就绪 · 全局热键被占用，退回 keyboard 钩子" if kb_ok else "就绪 · 全局热键注册失败（GUI 内快捷键仍可用）"
            )

    def _add_row(self, pt: Point):
        r = self.tbl.rowCount()
        self.tbl.insertRow(r)
        self.tbl.setItem(r, 0, QTableWidgetItem(str(pt.id)))
        self.tbl.setItem(r, 1, QTableWidgetItem(pt.name))
        self.tbl.setItem(r, 2, QTableWidgetItem(f"[{pt.pos[0]:.4f}, {pt.pos[1]:.4f}]"))
        self.tbl.setItem(r, 3, QTableWidgetItem(str(len(pt.operations))))
        btn_copy = QPushButton("复制")
        btn_copy.setToolTip("复制为 x,y 格式，可直接粘贴到脚本管理器的坐标输入框")
        btn_copy.clicked.connect(lambda _, p=pt: self._copy_point(p))
        self.tbl.setCellWidget(r, 4, btn_copy)

    def _copy_point(self, pt: Point):
        """复制归一化坐标为 'x,y'（脚本管理器新增点位/点击操作坐标框可直接粘贴）。"""
        text = f"{pt.pos[0]:.4f},{pt.pos[1]:.4f}"
        QApplication.clipboard().setText(text)
        self.statusBar().showMessage(f"已复制 #{pt.id}: {text}（可粘贴到脚本管理器坐标框）")

    def delete_selected(self):
        rows = sorted({i.row() for i in self.tbl.selectedIndexes()}, reverse=True)
        for r in rows:
            pid = int(self.tbl.item(r, 0).text())
            self._points = [p for p in self._points if p.id != pid]
            self.tbl.removeRow(r)

    # ---------------- 保存/导出 ----------------
    def _build_route(self) -> Route | None:
        if not self._points:
            QMessageBox.information(self, "提示", "还没有采集任何点位")
            return None
        if not self._target:
            return None
        # 名称单元格可能被用户编辑过 → 同步回模型
        for r in range(self.tbl.rowCount()):
            pid = int(self.tbl.item(r, 0).text())
            name = self.tbl.item(r, 1).text()
            for p in self._points:
                if p.id == pid:
                    p.name = name
        return Route(
            name=f"未命名路线_{self._target.process or 'target'}",
            target={"process": self._target.process, "windowClass": self._target.cls, "title": ""},
            points=list(self._points),
        )

    def save_route(self):
        route = self._build_route()
        if not route:
            return
        ROUTES_DIR.mkdir(exist_ok=True)
        path, _ = QFileDialog.getSaveFileName(
            self, "保存路线", str(ROUTES_DIR / f"{route.name}.yaml"), "YAML (*.yaml *.yml)"
        )
        if not path:
            return
        try:
            save_route_yaml(route, path)
        except Exception as e:
            QMessageBox.critical(self, "保存失败", str(e))
            return
        self.statusBar().showMessage(f"已保存: {path}")

    def export_route(self):
        route = self._build_route()
        if not route:
            return
        path, _ = QFileDialog.getSaveFileName(self, "导出路线", f"{route.name}.yaml", "YAML (*.yaml *.yml)")
        if not path:
            return
        try:
            save_route_yaml(route, path)
        except Exception as e:
            QMessageBox.critical(self, "导出失败", str(e))
            return
        self.statusBar().showMessage(f"已导出: {path}")

    def closeEvent(self, e):
        self._hotkeys.unregister_all()
        hk = getattr(self, "_hk_thread", None)
        if hk is not None:
            hk.shutdown()
        super().closeEvent(e)


def main():
    app = QApplication(sys.argv)
    win = PickerWindow()
    win.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
