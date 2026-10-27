# -*- coding: utf-8 -*-
"""GUI 自身快捷键服务（仅 manager/picker 内部使用；路线执行热键由 daemon 原生注册）。"""
from __future__ import annotations

from typing import Callable, Optional

import keyboard


class HotkeyService:
    """包装 keyboard 库：注册/注销 + 抑制按键传递。"""

    def __init__(self) -> None:
        self._hooks: dict[str, object] = {}

    def register(self, combo: str, callback: Callable[[], None], suppress: bool = True) -> bool:
        self.unregister(combo)
        try:
            self._hooks[combo] = keyboard.add_hotkey(combo, callback, suppress=suppress)
            return True
        except Exception:
            return False

    def unregister(self, combo: str) -> None:
        h = self._hooks.pop(combo, None)
        if h is not None:
            try:
                keyboard.remove_hotkey(h)
            except Exception:
                pass

    def unregister_all(self) -> None:
        for combo in list(self._hooks):
            self.unregister(combo)

    @staticmethod
    def normalize(combo: str) -> str:
        """keyboard 库的格式: ctrl+alt+p（统一小写、+ 连接）。"""
        parts = [p.strip().lower() for p in combo.replace("-", "+").split("+") if p.strip()]
        return "+".join(parts)


def hotkey_to_ahk(combo: str) -> str:
    """keyboard 格式 → AHK 热键格式：ctrl+alt+p → ^!p。"""
    mod_map = {"ctrl": "^", "alt": "!", "shift": "+", "win": "#"}
    parts = HotkeyService.normalize(combo).split("+")
    ahk = ""
    key = ""
    for p in parts:
        if p in mod_map:
            ahk += mod_map[p]
        else:
            key = p
    return ahk + key
