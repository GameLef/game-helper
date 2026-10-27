# -*- coding: utf-8 -*-
"""脚本管理器：路线列表 + 双视图编辑（表格/源码）+ 导入导出 + 热键绑定 + daemon 控制 + 执行。

布局：
  顶栏：DaemonBar + 打开取点工具按钮
  左：路线列表（名称/点数/目标/热键）
  右：QTabWidget「表格编辑 | YAML 源码」+ 底部操作按钮（执行/从N点/仅试跑/终止/保存/导入/导出/绑定热键）
"""
from __future__ import annotations

import shutil
import sys
from pathlib import Path

from PySide6.QtCore import QTimer, Qt
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QApplication,
    QComboBox,
    QFileDialog,
    QFrame,
    QHBoxLayout,
    QHeaderView,
    QInputDialog,
    QLabel,
    QLineEdit,
    QMainWindow,
    QMenu,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QSplitter,
    QStatusBar,
    QTabWidget,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from core import launcher  # noqa: E402
from core.route_model import (  # noqa: E402
    Route,
    RouteValidationError,
    load_route,
    load_route_text,
    save_route_yaml,
    validate_route_dict,
)
from ruamel.yaml import YAML  # noqa: E402
from widgets.daemon_bar import DaemonBar  # noqa: E402
from widgets.point_table import RouteEditor  # noqa: E402

ROUTES_DIR = Path(__file__).resolve().parents[1] / "routes"
CONFIG_PATH = Path(__file__).resolve().parents[1] / "config.json"


class ManagerWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("game_helper v2 · 脚本管理器")
        self.resize(1180, 720)

        self._route: Route | None = None          # 当前编辑中的模型
        self._route_path: Path | None = None      # 当前路线文件
        self._yaml = YAML()
        self._yaml.preserve_quotes = True

        central = QWidget()
        self.setCentralWidget(central)
        root = QVBoxLayout(central)

        # ---- 顶栏 ----
        top = QHBoxLayout()
        self.daemon_bar = DaemonBar()
        self.daemon_bar.statusChanged.connect(self.statusBar().showMessage)
        btn_picker = QPushButton("打开取点工具")
        btn_picker.clicked.connect(self._open_picker)
        top.addWidget(self.daemon_bar, 1)
        top.addWidget(btn_picker)
        root.addLayout(top)

        # ---- 中部：左列表 + 右编辑 ----
        splitter = QSplitter(Qt.Horizontal)

        # 左：路线列表
        left = QWidget()
        left_lay = QVBoxLayout(left)
        left_lay.setContentsMargins(0, 0, 0, 0)
        left_btns = QHBoxLayout()
        btn_refresh = QPushButton("刷新")
        btn_refresh.clicked.connect(self.refresh_list)
        btn_new = QPushButton("新建")
        btn_new.clicked.connect(self.new_route)
        btn_del = QPushButton("删除")
        btn_del.clicked.connect(self.delete_route)
        left_btns.addWidget(btn_refresh)
        left_btns.addWidget(btn_new)
        left_btns.addWidget(btn_del)
        left_lay.addLayout(left_btns)
        self.tbl_routes = QTableWidget(0, 4)
        self.tbl_routes.setHorizontalHeaderLabels(["文件", "名称", "点数", "热键"])
        self.tbl_routes.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeToContents)
        self.tbl_routes.setSelectionBehavior(QTableWidget.SelectRows)
        self.tbl_routes.setEditTriggers(QTableWidget.NoEditTriggers)
        self.tbl_routes.currentCellChanged.connect(self._on_route_selected)
        self.tbl_routes.setContextMenuPolicy(Qt.CustomContextMenu)
        self.tbl_routes.customContextMenuRequested.connect(self._route_context_menu)
        left_lay.addWidget(self.tbl_routes)
        splitter.addWidget(left)

        # 右：编辑区（Tab：表格 / 源码）
        right = QWidget()
        right_lay = QVBoxLayout(right)
        right_lay.setContentsMargins(0, 0, 0, 0)

        self.tabs = QTabWidget()
        self.editor = RouteEditor()
        self.editor.routeChanged.connect(self._on_model_dirty)
        self.txt_yaml = QPlainTextEdit()
        self.txt_yaml.setFont(QApplication.font())
        self.tabs.addTab(self.editor, "表格编辑")
        self.tabs.addTab(self.txt_yaml, "YAML 源码")
        self.tabs.currentChanged.connect(self._on_tab_changed)
        right_lay.addWidget(self.tabs, 1)

        # 底部操作条（放入可横向滚动容器：窗口变窄时按钮仍可滚到）
        ops_host = QWidget()
        ops = QHBoxLayout(ops_host)
        ops.setContentsMargins(0, 0, 0, 0)
        btn_run = QPushButton("▶ 从头执行")
        btn_run.clicked.connect(lambda: self._run(1, 0))
        self.btn_run_from = QPushButton("▶ 从第 N 点执行")
        self.btn_run_from.clicked.connect(self._run_from_dialog)
        self.btn_only = QPushButton("▶ 仅试跑第 N 点")
        self.btn_only.clicked.connect(self._only_dialog)
        btn_stop = QPushButton("■ 终止")
        btn_stop.clicked.connect(self._stop_run)
        btn_save = QPushButton("保存")
        btn_save.clicked.connect(self.save_current)
        btn_import = QPushButton("导入…")
        btn_import.clicked.connect(self.import_route)
        btn_export = QPushButton("导出…")
        btn_export.clicked.connect(self.export_route)
        self.btn_hotkey = QPushButton("绑定热键…")
        self.btn_hotkey.clicked.connect(self.bind_hotkey)
        for b in (btn_run, self.btn_run_from, self.btn_only, btn_stop):
            ops.addWidget(b)
        ops.addStretch(1)
        for b in (btn_save, btn_import, btn_export, self.btn_hotkey):
            ops.addWidget(b)
        ops_scroll = QScrollArea()
        ops_scroll.setWidget(ops_host)
        ops_scroll.setWidgetResizable(True)
        ops_scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAsNeeded)
        ops_scroll.setVerticalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        ops_scroll.setFrameShape(QFrame.NoFrame)
        ops_scroll.setFixedHeight(ops_host.sizeHint().height() + 6)
        right_lay.addWidget(ops_scroll)

        splitter.addWidget(right)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 3)
        root.addWidget(splitter, 1)

        self._load_config()
        self.refresh_list()
        QTimer.singleShot(100, lambda: self.daemon_bar.maybe_autostart(self._cfg.get("autoStartDaemon", True)))

    # ============================================================
    # 配置
    # ============================================================
    def _load_config(self):
        import json

        try:
            self._cfg = json.loads(CONFIG_PATH.read_text(encoding="utf-8"))
        except Exception:
            self._cfg = {}
        self.daemon_bar.chk_autostart.setChecked(self._cfg.get("autoStartDaemon", True))

    def _save_config(self):
        import json

        self._cfg["autoStartDaemon"] = self.daemon_bar.chk_autostart.isChecked()
        CONFIG_PATH.write_text(json.dumps(self._cfg, ensure_ascii=False, indent=2), encoding="utf-8")

    # ============================================================
    # 路线列表
    # ============================================================
    def refresh_list(self):
        self.tbl_routes.setRowCount(0)
        ROUTES_DIR.mkdir(exist_ok=True)
        hotkeys = self._hotkey_map()
        for f in sorted(ROUTES_DIR.glob("*.y*ml")):
            try:
                route, _ = load_route(f)
            except RouteValidationError as e:
                # 坏文件也进列表（灰显），不再静默消失
                r = self.tbl_routes.rowCount()
                self.tbl_routes.insertRow(r)
                item = QTableWidgetItem(f.stem)
                item.setForeground(QColor("#b00"))
                item.setToolTip(str(e))
                self.tbl_routes.setItem(r, 0, item)
                self.tbl_routes.setItem(r, 1, QTableWidgetItem("(无效路线)"))
                self.tbl_routes.setItem(r, 2, QTableWidgetItem("-"))
                self.tbl_routes.setItem(r, 3, QTableWidgetItem(""))
                continue
            except Exception:
                continue  # IO/解析层异常不炸列表
            r = self.tbl_routes.rowCount()
            self.tbl_routes.insertRow(r)
            self.tbl_routes.setItem(r, 0, QTableWidgetItem(f.stem))
            self.tbl_routes.setItem(r, 1, QTableWidgetItem(route.name))
            self.tbl_routes.setItem(r, 2, QTableWidgetItem(str(len(route.points))))
            hk = ", ".join(hotkeys.get(f.stem, []))
            self.tbl_routes.setItem(r, 3, QTableWidgetItem(hk))

    def _hotkey_map(self) -> dict[str, list[str]]:
        m: dict[str, list[str]] = {}
        for b in self._cfg.get("hotkeys", {}).get("bindings", []):
            m.setdefault(b.get("route", ""), []).append(b.get("hotkey", ""))
        return m

    def _current_stem(self) -> str | None:
        r = self.tbl_routes.currentRow()
        if r < 0:
            return None
        return self.tbl_routes.item(r, 0).text()

    def _on_route_selected(self, row, _col, _pr, _pc):
        if row < 0:
            return
        stem = self.tbl_routes.item(row, 0).text()
        path = ROUTES_DIR / f"{stem}.yaml"
        if not path.exists():
            path = ROUTES_DIR / f"{stem}.yml"
        self._open_route_file(path)

    def _open_route_file(self, path: Path):
        # 已是当前编辑中的路线（如新建后自动选中/草稿态），不重新加载
        if self._route is not None and self._route_path == path:
            return
        try:
            route, raw = load_route(path)
        except RouteValidationError as e:
            QMessageBox.warning(self, "路线无效", str(e))
            return
        self._route = route
        self._route_path = path
        self.editor.load_route(route)
        self._sync_yaml_text(raw)
        self.statusBar().showMessage(f"已加载: {path.name}")

    def _route_context_menu(self, pos):
        r = self.tbl_routes.rowAt(pos.y())
        if r < 0:
            return
        self.tbl_routes.selectRow(r)
        menu = QMenu(self)
        menu.addAction("从头执行", lambda: self._run(1, 0))
        menu.addAction("从第 N 点执行…", self._run_from_dialog)
        menu.addAction("仅试跑第 N 点…", self._only_dialog)
        menu.addSeparator()
        menu.addAction("在资源管理器中显示", lambda: launcher.V2_DIR and __import__("os").startfile(self._route_path.parent))
        menu.exec(self.tbl_routes.viewport().mapToGlobal(pos))

    def new_route(self):
        name, ok = QInputDialog.getText(self, "新建路线", "路线名称:")
        if not ok or not name.strip():
            return
        stem = name.strip()
        path = ROUTES_DIR / f"{stem}.yaml"
        if path.exists():
            QMessageBox.information(self, "新建路线", f"{path.name} 已存在，请在左侧选择它。")
            return
        # 草稿态：target 留空在编辑器里填（空 target 不符合 route/v1，不能落盘）
        route = Route(name=stem, target={"process": "", "windowClass": "", "title": ""})
        save_route_yaml(route, path)
        self._route = route
        self._route_path = path
        self.editor.load_route(route)
        self._sync_yaml_text(route.to_dict())
        self.refresh_list()
        # 选中新建的
        for r in range(self.tbl_routes.rowCount()):
            if self.tbl_routes.item(r, 0).text() == stem:
                self.tbl_routes.selectRow(r)
                break
        self.statusBar().showMessage(f"已新建 {path.name}（记得填写目标窗口后保存）")

    def delete_route(self):
        stem = self._current_stem()
        if not stem:
            return
        if QMessageBox.question(self, "删除路线", f"确定删除 {stem}.yaml？") != QMessageBox.Yes:
            return
        (ROUTES_DIR / f"{stem}.yaml").unlink(missing_ok=True)
        (ROUTES_DIR / f"{stem}.yml").unlink(missing_ok=True)
        # 同步清理该路线的热键绑定：daemon 重载/重启时按 config.json 注册热键，
        # 残留 binding 会让已删路线的热键继续生效，与新路线的快捷键冲突
        binds = self._cfg.get("hotkeys", {}).get("bindings", [])
        kept = [b for b in binds if b.get("route") != stem]
        removed = len(kept) != len(binds)
        if removed:
            self._cfg.setdefault("hotkeys", {})["bindings"] = kept
            self._save_config()
        # 清掉 daemon 用的 _cache JSON，避免孤儿缓存被预载
        (launcher.CACHE_DIR / f"{stem}.json").unlink(missing_ok=True)
        # 编辑器脱离已删文件，防止后续“保存”把 YAML 复活
        self._route = None
        self._route_path = None
        if removed and launcher.reload_daemon():
            self.statusBar().showMessage(f"已删除 {stem}（热键已注销，daemon 已重载）")
        else:
            self.statusBar().showMessage(f"已删除 {stem}（热键绑定与缓存已同步清理）")
        self.refresh_list()

    # ============================================================
    # 双视图同步
    # ============================================================
    def _sync_yaml_text(self, raw):
        import io

        buf = io.StringIO()
        y = YAML()
        y.preserve_quotes = True
        y.default_flow_style = None
        y.dump(raw, buf)
        self.txt_yaml.setPlainText(buf.getvalue())

    def _on_tab_changed(self, idx: int):
        if idx == 1 and self._route is not None:
            # 表格 → 源码：模型 dump
            import io

            buf = io.StringIO()
            y = YAML()
            y.default_flow_style = None
            y.dump(self._route.to_dict(), buf)
            self.txt_yaml.setPlainText(buf.getvalue())
        elif idx == 0 and self._route is not None:
            # 源码 → 表格：解析校验
            try:
                route, raw = load_route_text(self.txt_yaml.toPlainText())
            except RouteValidationError as e:
                QMessageBox.warning(self, "YAML 无效，无法切换", str(e))
                self.tabs.setCurrentIndex(1)
                return
            self._route = route
            self.editor.load_route(route)

    def _on_model_dirty(self):
        self.statusBar().showMessage("模型已修改（记得保存）", 3000)

    # ============================================================
    # 保存 / 导入 / 导出
    # ============================================================
    def save_current(self):
        if self._route is None or self._route_path is None:
            QMessageBox.information(self, "提示", "先从左侧选择一条路线")
            return
        # 若当前在源码视图，先解析回模型
        if self.tabs.currentIndex() == 1:
            try:
                route, _raw = load_route_text(self.txt_yaml.toPlainText())
            except RouteValidationError as e:
                QMessageBox.warning(self, "YAML 无效", str(e))
                return
            self._route = route
        save_route_yaml(self._route, self._route_path)
        # 刷新缓存 + 通知 daemon
        ok, msg = launcher.refresh_route_cache(self._route_path)
        if ok and self._cfg.get("autoReloadOnSave", True):
            launcher.reload_daemon()
        self.refresh_list()
        self.statusBar().showMessage(f"已保存并刷新缓存: {self._route_path.name}")

    def import_route(self):
        path, _ = QFileDialog.getOpenFileName(self, "导入路线", "", "YAML (*.yaml *.yml)")
        if not path:
            return
        src = Path(path)
        try:
            route, _ = load_route(src)
        except RouteValidationError as e:
            QMessageBox.warning(self, "导入失败", str(e))
            return
        dst = ROUTES_DIR / src.name
        shutil.copy(src, dst)
        self.refresh_list()
        self.statusBar().showMessage(f"已导入: {dst.name}")

    def export_route(self):
        if self._route is None:
            QMessageBox.information(self, "提示", "先选择一条路线")
            return
        path, _ = QFileDialog.getSaveFileName(self, "导出路线", f"{self._route.name}.yaml", "YAML (*.yaml *.yml)")
        if not path:
            return
        save_route_yaml(self._route, path)
        self.statusBar().showMessage(f"已导出: {path}")

    # ============================================================
    # 热键绑定
    # ============================================================
    def bind_hotkey(self):
        stem = self._current_stem()
        if not stem:
            QMessageBox.information(self, "提示", "先选择一条路线")
            return
        route = self._route
        total = len(route.points) if route else 0
        dlg = QInputDialog(self)
        dlg.setInputMode(QInputDialog.TextInput)
        label = (
            f"为 {stem} 绑定热键（如 F8 或 ctrl+alt+F8），留空清除全部绑定\n"
            f"路线共 {total} 点\n"
            "热键为单步模式：每按一次执行一个点位，走完回到起始点循环\n"
            "进度显示在顶部 daemon 状态栏"
        )
        combo_text, ok = QInputDialog.getText(self, "绑定热键", label)
        if not ok:
            return
        from core.hotkey_service import HotkeyService, hotkey_to_ahk

        combo = HotkeyService.normalize(combo_text) if combo_text.strip() else ""
        start_at, ok2 = QInputDialog.getInt(
            self, "起始点", f"单步从第几个点位开始（1~{max(total,1)}，每轮循环回到此点）:", 1, 1, max(total, 1)
        )
        if not ok2:
            return
        # 移除该路线旧绑定
        binds = [b for b in self._cfg.get("hotkeys", {}).get("bindings", []) if b.get("route") != stem]
        if combo:
            binds.append({"hotkey": hotkey_to_ahk(combo), "action": "run_route", "route": stem, "startAt": start_at})
        self._cfg.setdefault("hotkeys", {})["bindings"] = binds
        self._save_config()
        # daemon 重载；未运行则拉起，否则新绑定不会生效（热键由 daemon 注册）
        if launcher.reload_daemon():
            self.statusBar().showMessage(f"已绑定 {combo or '(清除)'} → {stem}，daemon 已重载")
        elif combo:
            started, msg = launcher.start_daemon()
            self.statusBar().showMessage(
                f"已绑定 {combo} → {stem}，daemon {'已启动（热键已生效）' if started else '启动失败: ' + msg}"
            )
        else:
            self.statusBar().showMessage("已清除绑定（daemon 未运行，下次启动后生效）")
        self.refresh_list()

    # ============================================================
    # 执行控制
    # ============================================================
    def _ensure_cache(self) -> str | None:
        """确保当前路线缓存最新，返回 stem。"""
        if self._route_path is None:
            return None
        ok, stem_or_err = launcher.refresh_route_cache(self._route_path)
        if not ok:
            QMessageBox.warning(self, "缓存转换失败", stem_or_err)
            return None
        return stem_or_err

    def _run(self, start_at: int, only: int):
        stem = self._ensure_cache()
        if not stem:
            return
        if launcher.find_daemon_hwnd():
            ok, msg = launcher.run_route_via_daemon(stem, start_at, only)
            self.statusBar().showMessage(f"daemon 执行: {msg}")
            if not ok:
                QMessageBox.warning(self, "执行失败", msg)
        else:
            resp = QMessageBox.question(
                self, "daemon 未运行", "daemon 未运行。\n是否直接以 runner.ahk 单次执行？",
            )
            if resp == QMessageBox.Yes:
                cache = launcher.CACHE_DIR / f"{stem}.json"
                launcher.run_runner(cache, start_at, only)
                self.statusBar().showMessage("runner 直启已发出")

    def _run_from_dialog(self):
        if not self._route:
            return
        n, ok = QInputDialog.getInt(self, "从第 N 点执行", "N:", 1, 1, max(len(self._route.points), 1))
        if ok:
            self._run(n, 0)

    def _only_dialog(self):
        if not self._route:
            return
        n, ok = QInputDialog.getInt(self, "仅试跑第 N 点", "N:", 1, 1, max(len(self._route.points), 1))
        if ok:
            self._run(1, n)

    def _stop_run(self):
        """终止：优先 daemon 路线内急停；daemon 不在则提示用 F12。"""
        s = launcher.daemon_status()
        if s.running:
            launcher.reload_daemon()  # reload 不会打断，仅刷新；真正的急停是 F12
            self.statusBar().showMessage("请按 F12 急停（daemon 正在执行时全局生效）")
        else:
            self.statusBar().showMessage("daemon 未运行；若 runner 在跑请按 F12")

    # ============================================================
    def _open_picker(self):
        import subprocess

        subprocess.Popen([sys.executable, str(Path(__file__).parent / "picker.py")])

    def closeEvent(self, e):
        self._save_config()
        super().closeEvent(e)


def main():
    app = QApplication(sys.argv)
    win = ManagerWindow()
    win.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
