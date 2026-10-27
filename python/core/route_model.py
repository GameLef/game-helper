# -*- coding: utf-8 -*-
"""路线模型：YAML 读写 + schema 校验（错误定位到点位/操作）+ YAML↔JSON 转换。

数据契约见 docs/route-format.md（route/v1）。
"""
from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

from ruamel.yaml import YAML
from ruamel.yaml.error import YAMLError

FORMAT_VERSION = "route/v1"

OP_TYPES = ("click", "key", "press", "wheel", "drag", "wait")


class RouteValidationError(Exception):
    """校验失败；message 中带定位信息（points[i] / points[i].operations[j]）。"""


@dataclass
class Op:
    type: str
    # click: pos/button/clicks; key: key/up; press: key/ms; wheel: delta;
    # drag: from/to/duration; wait: ms; 通用: jitter
    params: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"type": self.type}
        d.update(self.params)
        return d


@dataclass
class Point:
    id: int
    pos: tuple[float, float]
    name: str = ""
    note: str = ""
    operations: list[Op] = field(default_factory=list)


@dataclass
class Route:
    name: str
    target: dict[str, str] = field(default_factory=lambda: {"process": "", "windowClass": "", "title": ""})
    points: list[Point] = field(default_factory=list)
    note: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "format": FORMAT_VERSION,
            "name": self.name,
            "note": self.note,
            "target": {
                "process": self.target.get("process", ""),
                "windowClass": self.target.get("windowClass", ""),
                "title": self.target.get("title", ""),
            },
            "points": [
                {
                    "id": p.id,
                    "name": p.name,
                    "note": p.note,
                    "pos": [round(p.pos[0], 6), round(p.pos[1], 6)],
                    "operations": [op.to_dict() for op in p.operations],
                }
                for p in self.points
            ],
        }


# ----------------------------------------------------------------------------
# 校验
# ----------------------------------------------------------------------------

def _where(idx: int, pt: dict) -> str:
    label = f"points[{idx}]"
    if isinstance(pt, dict):
        pid = pt.get("id", "?")
        pname = pt.get("name", "")
        tag = f"#{pid}" + (f"({pname})" if pname else "")
        label = f"{label} {tag}"
    return label


def _check_pair(value: Any, where: str) -> tuple[float, float]:
    if (
        not isinstance(value, (list, tuple))
        or len(value) != 2
        or not all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in value)
    ):
        raise RouteValidationError(f"{where}: 坐标必须是 [x, y] 两个数字")
    x, y = float(value[0]), float(value[1])
    if not (0.0 <= x <= 1.0 and 0.0 <= y <= 1.0):
        raise RouteValidationError(f"{where}: 坐标超出 0~1 范围: [{x}, {y}]")
    return (x, y)


def _require_number(d: dict, key: str, where: str, *, min_val: float | None = None, integer: bool = False) -> float:
    if key not in d:
        raise RouteValidationError(f"{where}: 缺少必填字段 {key}")
    v = d[key]
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise RouteValidationError(f"{where}: 字段 {key} 必须是数字")
    if integer and float(v) != int(v):
        raise RouteValidationError(f"{where}: 字段 {key} 必须是整数")
    if min_val is not None and v < min_val:
        raise RouteValidationError(f"{where}: 字段 {key} 不能小于 {min_val}（当前 {v}）")
    return float(v)


def _require_str(d: dict, key: str, where: str, *, allow_empty: bool = False) -> str:
    if key not in d:
        raise RouteValidationError(f"{where}: 缺少必填字段 {key}")
    v = d[key]
    if not isinstance(v, str):
        raise RouteValidationError(f"{where}: 字段 {key} 必须是字符串")
    if not allow_empty and not v.strip():
        raise RouteValidationError(f"{where}: 字段 {key} 不能为空")
    return v


def _validate_op(op_raw: Any, where: str) -> Op:
    if not isinstance(op_raw, dict):
        raise RouteValidationError(f"{where}: 操作必须是对象")
    t = op_raw.get("type")
    if t not in OP_TYPES:
        raise RouteValidationError(f"{where}: 未知操作类型 {t!r}（支持: {'/'.join(OP_TYPES)}）")
    params: dict[str, Any] = {}
    jitter = op_raw.get("jitter", 0)
    if isinstance(jitter, bool) or not isinstance(jitter, (int, float)) or jitter < 0:
        raise RouteValidationError(f"{where}: jitter 必须是 ≥0 的数字")
    if jitter:
        params["jitter"] = jitter

    if t == "click":
        params["pos"] = list(_check_pair(op_raw.get("pos"), f"{where}.pos"))
        btn = op_raw.get("button", "left")
        if btn not in ("left", "right", "middle"):
            raise RouteValidationError(f"{where}: button 只能是 left/right/middle")
        params["button"] = btn
        clicks = op_raw.get("clicks", 1)
        if isinstance(clicks, bool) or not isinstance(clicks, int) or clicks < 1:
            raise RouteValidationError(f"{where}: clicks 必须是 ≥1 的整数")
        params["clicks"] = clicks
    elif t in ("key", "press"):
        params["key"] = _require_str(op_raw, "key", where)
        if t == "key":
            up = op_raw.get("up", False)
            if not isinstance(up, bool):
                raise RouteValidationError(f"{where}: up 必须是布尔值")
            params["up"] = up
        else:
            params["ms"] = _require_number(op_raw, "ms", where, min_val=1, integer=True)
    elif t == "wheel":
        params["delta"] = _require_number(op_raw, "delta", where, integer=True)
        if params["delta"] == 0:
            raise RouteValidationError(f"{where}: delta 不能为 0")
    elif t == "drag":
        params["from"] = list(_check_pair(op_raw.get("from"), f"{where}.from"))
        params["to"] = list(_check_pair(op_raw.get("to"), f"{where}.to"))
        params["duration"] = _require_number(op_raw, "duration", where, min_val=1, integer=True) if "duration" in op_raw else 200
    elif t == "wait":
        params["ms"] = _require_number(op_raw, "ms", where, min_val=1, integer=True)
    return Op(type=t, params=params)


def validate_route_dict(data: Any) -> Route:
    """校验已解析的路线 dict，返回 Route 对象；失败抛 RouteValidationError。"""
    if not isinstance(data, dict):
        raise RouteValidationError("路线文件必须是 YAML 对象（字典）")
    fmt = data.get("format")
    if fmt != FORMAT_VERSION:
        raise RouteValidationError(f"format 必须是 {FORMAT_VERSION!r}（当前 {fmt!r}）")
    name = _require_str(data, "name", "路线")
    note = data.get("note", "") or ""
    target_raw = data.get("target")
    if not isinstance(target_raw, dict):
        raise RouteValidationError("target 必须是对象，且 process/windowClass/title 至少填一个")
    target = {
        "process": str(target_raw.get("process", "") or ""),
        "windowClass": str(target_raw.get("windowClass", "") or ""),
        "title": str(target_raw.get("title", "") or ""),
    }
    if not any(target.values()):
        raise RouteValidationError("target 中 process/windowClass/title 至少填一个")

    points_raw = data.get("points")
    if not isinstance(points_raw, list):
        raise RouteValidationError("points 必须是列表")

    points: list[Point] = []
    for i, pt in enumerate(points_raw):
        where = _where(i, pt if isinstance(pt, dict) else {})
        if not isinstance(pt, dict):
            raise RouteValidationError(f"{where}: 点位必须是对象")
        pid = pt.get("id")
        if isinstance(pid, bool) or not isinstance(pid, int) or pid < 1:
            raise RouteValidationError(f"{where}: id 必须是正整数")
        pos = _check_pair(pt.get("pos"), f"{where}.pos")
        ops_raw = pt.get("operations", []) or []
        if not isinstance(ops_raw, list):
            raise RouteValidationError(f"{where}: operations 必须是列表")
        ops = []
        for j, op_raw in enumerate(ops_raw):
            ops.append(_validate_op(op_raw, f"{where}.operations[{j}]"))
        points.append(
            Point(
                id=pid,
                pos=pos,
                name=str(pt.get("name", "") or ""),
                note=str(pt.get("note", "") or ""),
                operations=ops,
            )
        )
    return Route(name=name, target=target, points=points, note=note)


# ----------------------------------------------------------------------------
# 读写与转换
# ----------------------------------------------------------------------------

def _make_yaml() -> YAML:
    y = YAML()
    y.preserve_quotes = True
    return y


def load_route(path: str | Path) -> tuple[Route, Any]:
    """读 YAML → 校验。返回 (Route, 原始 ruamel 数据)。

    抛 RouteValidationError（业务错误，带定位）或 YAMLError/IOError。
    """
    p = Path(path)
    text = p.read_text(encoding="utf-8")
    return load_route_text(text)


def load_route_text(text: str) -> tuple[Route, Any]:
    y = _make_yaml()
    try:
        raw = y.load(text)
    except YAMLError as e:
        m = re.search(r"line (\d+)", str(e))
        loc = f"（第 {m.group(1)} 行附近）" if m else ""
        raise RouteValidationError(f"YAML 语法错误{loc}: {e}") from e
    if raw is None:
        raise RouteValidationError("路线文件为空")
    route = validate_route_dict(raw)
    return route, raw


def save_route_yaml(route: Route, path: str | Path) -> None:
    """整段重写 points（单一数据源原则）。"""
    p = Path(path)
    y = _make_yaml()
    y.default_flow_style = None  # 让嵌套列表用 flow（[x, y]），块结构用 block
    y.dump(route.to_dict(), p.open("w", encoding="utf-8"))


def route_to_json_text(route: Route) -> str:
    """转成给 AHK runner/daemon 的 JSON 文本（ensure_ascii=False 保持中文可读）。"""
    return json.dumps(route.to_dict(), ensure_ascii=False, separators=(",", ":"))


def save_route_cache(route: Route, cache_dir: str | Path, stem: str | None = None) -> Path:
    """写入 routes/_cache/<stem>.json；stem 缺省用路线文件名。"""
    cache = Path(cache_dir)
    cache.mkdir(parents=True, exist_ok=True)
    if stem is None:
        stem = _safe_stem(route.name) or "route"
    out = cache / f"{stem}.json"
    out.write_text(route_to_json_text(route), encoding="utf-8")
    return out


def _safe_stem(name: str) -> str:
    return re.sub(r'[\\/:*?"<>|\s]+', "_", name).strip("_")


def load_config(path: str | Path) -> dict:
    return json.loads(Path(path).read_text(encoding="utf-8"))


def save_config(cfg: dict, path: str | Path) -> None:
    Path(path).write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")
