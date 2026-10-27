# -*- coding: utf-8 -*-
"""窗口工具：DPI aware、窗口枚举、客户区坐标、十字准星命中测试。"""
from __future__ import annotations

import ctypes
from dataclasses import dataclass
from typing import Optional

import win32api
import win32con
import win32gui
import win32process

try:  # Per-Monitor V2 DPI aware（失败则退回系统级）
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass


@dataclass
class WindowInfo:
    hwnd: int
    title: str
    cls: str
    process: str  # 进程名，如 notepad.exe

    @property
    def label(self) -> str:
        t = self.title[:40] or "(无标题)"
        return f"{t}  [{self.process}]  hwnd={self.hwnd}"


@dataclass
class ClientArea:
    left: int  # 客户区左上角屏幕坐标（物理像素）
    top: int
    width: int
    height: int


def list_windows(include_hidden: bool = False) -> list[WindowInfo]:
    """枚举可见顶层窗口（带标题的）。"""
    result: list[WindowInfo] = []

    def _cb(hwnd, _):
        if not include_hidden and not win32gui.IsWindowVisible(hwnd):
            return
        title = win32gui.GetWindowText(hwnd)
        if not title:
            return
        cls = win32gui.GetClassName(hwnd)
        result.append(WindowInfo(hwnd=hwnd, title=title, cls=cls, process=get_process_name(hwnd)))

    win32gui.EnumWindows(_cb, None)
    return result


def get_process_name(hwnd: int) -> str:
    try:
        _, pid = win32process.GetWindowThreadProcessId(hwnd)
        import psutil  # 可选依赖；未安装时优雅降级

        return psutil.Process(pid).name()
    except Exception:
        return ""


def get_client_area(hwnd: int) -> Optional[ClientArea]:
    """客户区屏幕坐标与尺寸（物理像素）。"""
    try:
        left, top, right, bottom = win32gui.GetClientRect(hwnd)
        l2, t2 = win32gui.ClientToScreen(hwnd, (left, top))
        return ClientArea(left=l2, top=t2, width=right - left, height=bottom - top)
    except Exception:
        return None


def window_from_point(x: int, y: int) -> Optional[WindowInfo]:
    """十字准星命中测试：屏幕坐标 → 该点所属的顶层窗口。"""
    try:
        hwnd = win32gui.WindowFromPoint((x, y))
        # 向上取顶层窗口（跳过分组窗口）
        root = win32gui.GetAncestor(hwnd, win32con.GA_ROOT)
        if root:
            hwnd = root
        return WindowInfo(
            hwnd=hwnd,
            title=win32gui.GetWindowText(hwnd),
            cls=win32gui.GetClassName(hwnd),
            process=get_process_name(hwnd),
        )
    except Exception:
        return None


def mouse_pos() -> tuple[int, int]:
    # pywin32 的 GetCursorPos 返回 (x, y) 元组（无 .x/.y 属性）
    x, y = win32api.GetCursorPos()
    return int(x), int(y)


def rel_and_norm(x: int, y: int, area: ClientArea) -> tuple[tuple[int, int], tuple[float, float]]:
    """屏幕坐标 → (相对客户区坐标, 归一化坐标)。"""
    rx, ry = x - area.left, y - area.top
    nw = area.width - 1 if area.width > 1 else 1
    nh = area.height - 1 if area.height > 1 else 1
    return (rx, ry), (rx / nw, ry / nh)


def norm_to_screen(nx: float, ny: float, area: ClientArea) -> tuple[int, int]:
    """归一化 → 屏幕物理像素（与 AHK executor EXE_ToScreen 同公式）。"""
    return (
        area.left + round(nx * (area.width - 1)),
        area.top + round(ny * (area.height - 1)),
    )
