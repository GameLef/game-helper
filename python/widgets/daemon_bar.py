# -*- coding: utf-8 -*-
"""daemon 控制条：状态灯 + 心跳轮询 + 启动/停止/重载按钮 + 自启开关 + 单步游标显示。"""
from __future__ import annotations

import json

from PySide6.QtCore import QTimer, Signal
from PySide6.QtWidgets import QCheckBox, QHBoxLayout, QLabel, QPushButton, QWidget

from core import launcher

CURSOR_STATE_PATH = launcher.V2_DIR / "state" / "cursor.json"


class DaemonBar(QWidget):
    """嵌入 manager 顶栏。statusChanged 状态文本变化（含 busy 变化）。"""

    statusChanged = Signal(str)
    started = Signal()
    stopped = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        lay = QHBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)

        self.lbl_light = QLabel("⬤")
        self.lbl_light.setStyleSheet("color: #999; font-size: 16px;")
        self.lbl_state = QLabel("daemon: 检测中…")
        self.lbl_cursor = QLabel("—")
        self.lbl_cursor.setToolTip("路线热键为单步模式：每按一次执行一个点位；这里显示当前进度")
        self.btn_start = QPushButton("启动")
        self.btn_stop = QPushButton("停止")
        self.btn_reload = QPushButton("重载热键")
        self.chk_autostart = QCheckBox("manager 启动时自动拉起")
        self.btn_start.clicked.connect(self._start)
        self.btn_stop.clicked.connect(self._stop)
        self.btn_reload.clicked.connect(self._reload)

        lay.addWidget(self.lbl_light)
        lay.addWidget(self.lbl_state)
        lay.addWidget(self.btn_start)
        lay.addWidget(self.btn_stop)
        lay.addWidget(self.btn_reload)
        lay.addStretch(1)
        lay.addWidget(self.lbl_cursor)
        lay.addWidget(self.chk_autostart)

        # 心跳轮询
        self._timer = QTimer(self)
        self._timer.timeout.connect(self.refresh)
        self._timer.start(2000)
        self._was_running = False
        self._was_busy = False
        self.refresh()

        # 游标轮询（独立于心跳，daemon 写 state/cursor.json）
        self._cursor_timer = QTimer(self)
        self._cursor_timer.timeout.connect(self._refresh_cursor)
        self._cursor_timer.start(1000)
        self._refresh_cursor()

    # ------------------------------------------------------------------
    def refresh(self):
        try:
            s = launcher.daemon_status()
        except Exception as e:
            s = None
            self.lbl_state.setText(f"daemon: 查询失败({e.__class__.__name__})")
        if s is None:
            return
        if not s.running:
            self.lbl_light.setStyleSheet("color: #999; font-size: 16px;")
            self.lbl_state.setText("daemon: 已停止")
            self.lbl_cursor.setText("—")
            self.btn_start.setEnabled(True)
            self.btn_stop.setEnabled(False)
            self.btn_reload.setEnabled(False)
            if self._was_running:
                self._was_running = False
                self.stopped.emit()
                self.statusChanged.emit("daemon 已停止")
        else:
            busy = " · 执行中" if s.busy else ""
            self.lbl_light.setStyleSheet("color: #2ecc71; font-size: 16px;")
            self.lbl_state.setText(f"daemon: 运行中 pid={s.pid} 热键={s.hotkeys}{busy}")
            self.btn_start.setEnabled(False)
            self.btn_stop.setEnabled(True)
            self.btn_reload.setEnabled(True)
            if not self._was_running:
                self._was_running = True
                self.started.emit()
            if s.busy != self._was_busy:
                self._was_busy = s.busy
                self.statusChanged.emit("daemon 开始执行" if s.busy else "daemon 空闲")

    def _refresh_cursor(self):
        """读 state/cursor.json 显示各路线单步进度；失败/缺失按无数据展示，不报错。"""
        if not launcher.find_daemon_hwnd():
            self.lbl_cursor.setText("—")
            return
        try:
            with open(CURSOR_STATE_PATH, "r", encoding="utf-8") as f:
                data = json.load(f)
        except Exception:
            self.lbl_cursor.setText("…")
            return
        routes = data.get("routes") or {}
        if not routes:
            self.lbl_cursor.setText("—")
            return
        parts = []
        for name, info in routes.items():
            total = info.get("total", 0)
            done = info.get("lastDone", 0)
            parts.append(f"{name} ▸ {'未开始' if done <= 0 else f'{done}/{total}'}")
        self.lbl_cursor.setText("   ".join(parts))

    def maybe_autostart(self, enabled: bool):
        if enabled and not launcher.find_daemon_hwnd():
            self._start()

    # ------------------------------------------------------------------
    def _start(self):
        ok, msg = launcher.start_daemon()
        self.statusChanged.emit(f"daemon 启动: {msg}")
        self.refresh()

    def _stop(self):
        ok = launcher.stop_daemon()
        self.statusChanged.emit("daemon 停止" + ("" if ok else " 失败（超时）"))
        self.refresh()

    def _reload(self):
        ok = launcher.reload_daemon()
        self.statusChanged.emit("热键重载" + ("已发送" if ok else "失败：daemon 未运行"))
