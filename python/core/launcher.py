# -*- coding: utf-8 -*-
"""daemon/runner 生命周期控制：探测 AHK、YAML→_cache 转换、启动/停止/心跳/重载/运行。

与 ahk/daemon.ahk 的消息协议：
  - FindWindow(None, "game_helper_daemon_v2")   ← daemon 主窗口改名后的标题
  - RegisterWindowMessage: GameHelperDaemon_Ping / _Reload / _Stop
  - WM_COPYDATA dwData=1: {"route": ..., "startAt": n, "only": n}
  - Ping 返回值: (热键数 << 1) | (执行中 ? 1 : 0)
"""
from __future__ import annotations

import ctypes
import json
import struct
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import win32con
import win32gui
import win32process

from .route_model import Route, RouteValidationError, load_route, route_to_json_text, save_route_cache

WM_COPYDATA = 0x004A
DAEMON_WIN_TITLE = "game_helper_daemon_v2"

_BASE = Path(__file__).resolve().parents[2]  # v2/
V2_DIR = _BASE
AHK_DIR = _BASE / "ahk"
ROUTES_DIR = _BASE / "routes"
CACHE_DIR = ROUTES_DIR / "_cache"
CONFIG_PATH = _BASE / "config.json"

_AHK_CANDIDATES = [
    Path(r"C:\Program Files\AutoHotkey\v2\AutoHotkey64.exe"),
    Path(r"C:\Program Files\AutoHotkey\v2\AutoHotkey.exe"),
]


def find_ahk() -> Optional[Path]:
    for p in _AHK_CANDIDATES:
        if p.exists():
            return p
    # 尝试注册表
    try:
        import winreg

        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\AutoHotkey") as k:
            install_dir, _ = winreg.QueryValueEx(k, "InstallDir")
            cand = Path(install_dir) / "v2" / "AutoHotkey64.exe"
            if cand.exists():
                return cand
    except Exception:
        pass
    return None


# ----------------------------------------------------------------------------
# daemon 控制（窗口消息）
# ----------------------------------------------------------------------------

def _reg_msg(name: str) -> int:
    return ctypes.windll.user32.RegisterWindowMessageW(name)


MSG_PING = _reg_msg("GameHelperDaemon_Ping")
MSG_RELOAD = _reg_msg("GameHelperDaemon_Reload")
MSG_STOP = _reg_msg("GameHelperDaemon_Stop")


def find_daemon_hwnd() -> Optional[int]:
    return win32gui.FindWindow(None, DAEMON_WIN_TITLE)


@dataclass
class DaemonStatus:
    running: bool
    pid: int = 0
    hotkeys: int = 0
    busy: bool = False


def daemon_status() -> DaemonStatus:
    hwnd = find_daemon_hwnd()
    if not hwnd:
        return DaemonStatus(running=False)
    ret = win32gui.SendMessage(hwnd, MSG_PING, 0, 0)
    hotkeys = (ret >> 1) & 0x7FFFFFFF
    busy = bool(ret & 1)
    _, pid = win32process.GetWindowThreadProcessId(hwnd)
    return DaemonStatus(running=True, pid=pid, hotkeys=hotkeys, busy=busy)


def start_daemon(ahk: Path | None = None) -> tuple[bool, str]:
    if find_daemon_hwnd():
        return True, "daemon 已在运行"
    ahk = ahk or find_ahk()
    if not ahk:
        return False, "未找到 AutoHotkey v2，请在 config.json 中配置 ahkPath"
    try:
        subprocess.Popen(
            [str(ahk), str(AHK_DIR / "daemon.ahk")],
            cwd=str(AHK_DIR),
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS,
        )
    except Exception as e:
        return False, f"启动失败: {e}"
    # 等窗口出现（最多 5 秒）
    import time

    for _ in range(50):
        time.sleep(0.1)
        if find_daemon_hwnd():
            return True, "daemon 已启动"
    return False, "daemon 启动超时（未见消息窗口）"


def stop_daemon() -> bool:
    hwnd = find_daemon_hwnd()
    if not hwnd:
        return True
    win32gui.SendMessage(hwnd, MSG_STOP, 0, 0)
    import time

    for _ in range(30):
        time.sleep(0.1)
        if not find_daemon_hwnd():
            return True
    return False


def reload_daemon() -> bool:
    hwnd = find_daemon_hwnd()
    if not hwnd:
        return False
    win32gui.SendMessage(hwnd, MSG_RELOAD, 0, 0)
    return True


def _copydata_run(hwnd: int, payload: dict) -> bool:
    text = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    buf = ctypes.create_string_buffer(text + b"\x00")

    class _CDS(ctypes.Structure):
        _fields_ = [("dwData", ctypes.c_void_p), ("cbData", ctypes.c_ulong), ("lpData", ctypes.c_void_p)]

    cds = _CDS()
    cds.dwData = 1
    cds.cbData = len(text) + 1
    cds.lpData = ctypes.addressof(buf)
    ret = ctypes.windll.user32.SendMessageW(hwnd, WM_COPYDATA, 0, ctypes.byref(cds))
    return bool(ret)


def run_route_via_daemon(route_stem: str, start_at: int = 1, only: int = 0) -> tuple[bool, str]:
    hwnd = find_daemon_hwnd()
    if not hwnd:
        return False, "daemon 未运行"
    ok = _copydata_run(hwnd, {"route": route_stem, "startAt": start_at, "only": only})
    return ok, "已发送" if ok else "daemon 拒绝了请求（路线未缓存？）"


# ----------------------------------------------------------------------------
# 缓存转换与 runner 直启
# ----------------------------------------------------------------------------

def refresh_route_cache(yaml_path: str | Path) -> tuple[bool, str]:
    """YAML → _cache/<stem>.json。成功返回 (True, stem)。"""
    p = Path(yaml_path)
    stem = p.stem
    try:
        route, _raw = load_route(p)
    except RouteValidationError as e:
        return False, str(e)
    except Exception as e:
        return False, f"读取失败: {e}"
    save_route_cache(route, CACHE_DIR, stem=stem)
    return True, stem


def run_runner(route_json: Path, start_at: int = 1, only: int = 0) -> subprocess.Popen:
    ahk = find_ahk()
    if not ahk:
        raise RuntimeError("未找到 AutoHotkey v2")
    args = [str(ahk), str(AHK_DIR / "runner.ahk"), str(route_json)]
    if start_at > 1:
        args += ["--start", str(start_at)]
    if only > 0:
        args += ["--only", str(only)]
    return subprocess.Popen(args, cwd=str(AHK_DIR))
