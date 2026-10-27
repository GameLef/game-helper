# -*- coding: utf-8 -*-
"""game_helper v2 · core 包。

模块：
- route_model    路线 YAML 读写/校验/转换
- window_utils   窗口枚举、客户区、DPI aware
- launcher       AHK 探测、daemon 生命周期、_cache 转换、runner 直启
- hotkey_service GUI 自身快捷键
"""
from . import launcher, route_model, window_utils  # noqa: F401
