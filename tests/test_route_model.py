# -*- coding: utf-8 -*-
"""route_model 单元测试：合法/非法 schema、错误定位、YAML round-trip、YAML→JSON 一致性。"""
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from core.route_model import (  # noqa: E402
    FORMAT_VERSION,
    Route,
    RouteValidationError,
    load_route_text,
    route_to_json_text,
    save_route_yaml,
    validate_route_dict,
)

VALID = f"""
format: {FORMAT_VERSION}
name: 测试路线
target:
  process: notepad.exe
  windowClass: ""
  title: ""

points:
  - id: 1
    name: 第一个点
    pos: [0.5, 0.5]
    operations:
      - {{ type: click, pos: [0.9, 0.9], button: left, clicks: 2, jitter: 3 }}
      - {{ type: key, key: m }}
      - {{ type: press, key: w, ms: 800 }}
      - {{ type: wheel, delta: -5 }}
      - {{ type: drag, from: [0.1, 0.1], to: [0.2, 0.2], duration: 150 }}
      - {{ type: wait, ms: 100 }}
  - id: 2
    name: 第二个点
    pos: [0.1, 0.9]
"""


class TestValid:
    def test_load_valid(self):
        route, _raw = load_route_text(VALID)
        assert route.name == "测试路线"
        assert len(route.points) == 2
        assert route.points[0].pos == (0.5, 0.5)
        assert len(route.points[0].operations) == 6
        assert route.points[0].operations[0].type == "click"

    def test_yaml_to_json_roundtrip(self):
        import json

        route, _ = load_route_text(VALID)
        data = json.loads(route_to_json_text(route))
        assert data["format"] == FORMAT_VERSION
        assert data["points"][0]["operations"][0]["type"] == "click"
        assert data["points"][0]["operations"][0]["jitter"] == 3

    def test_save_and_reload(self, tmp_path):
        route, _ = load_route_text(VALID)
        p = tmp_path / "r.yaml"
        save_route_yaml(route, p)
        route2, _ = load_route_text(p.read_text(encoding="utf-8"))
        assert route2.name == route.name
        assert len(route2.points) == len(route.points)
        assert route2.points[0].operations[0].params["clicks"] == 2


class TestInvalid:
    def test_bad_format(self):
        with pytest.raises(RouteValidationError, match="format"):
            load_route_text("format: route/v999\nname: x\ntarget:\n  process: a.exe\npoints: []")

    def test_empty_target(self):
        text = "format: route/v1\nname: x\ntarget:\n  process: \"\"\n  windowClass: \"\"\n  title: \"\"\npoints: []"
        with pytest.raises(RouteValidationError, match="target"):
            load_route_text(text)

    def test_point_pos_out_of_range(self):
        text = VALID.replace("pos: [0.5, 0.5]", "pos: [1.5, 0.5]")
        with pytest.raises(RouteValidationError, match=r"points\[0\]"):
            load_route_text(text)

    def test_op_bad_type(self):
        text = VALID.replace("type: click", "type: explode")
        with pytest.raises(RouteValidationError, match="operations"):
            load_route_text(text)

    def test_op_missing_field(self):
        text = VALID.replace("ms: 800", "")
        with pytest.raises(RouteValidationError, match="ms"):
            load_route_text(text)

    def test_point_located_error_message(self):
        text = VALID.replace("pos: [0.1, 0.9]", "pos: [-0.1, 0.9]")
        with pytest.raises(RouteValidationError) as ei:
            load_route_text(text)
        assert "points[1]" in str(ei.value)

    def test_yaml_syntax_error_with_line(self):
        with pytest.raises(RouteValidationError, match="YAML 语法错误"):
            load_route_text("format: route/v1\nname: [unclosed\npoints: []")

    def test_id_not_int(self):
        text = VALID.replace("- id: 2", "- id: abc")
        with pytest.raises(RouteValidationError, match="id"):
            load_route_text(text)


class TestEdge:
    def test_empty_points_ok(self):
        text = "format: route/v1\nname: x\ntarget:\n  process: a.exe\npoints: []"
        route, _ = load_route_text(text)
        assert route.points == []

    def test_boundary_coords(self):
        text = VALID.replace("pos: [0.1, 0.9]", "pos: [0.0, 1.0]")
        route, _ = load_route_text(text)
        assert route.points[1].pos == (0.0, 1.0)

    def test_wait_ms_zero_rejected(self):
        text = VALID.replace("{ type: wait, ms: 100 }", "{ type: wait, ms: 0 }")
        with pytest.raises(RouteValidationError):
            load_route_text(text)

    def test_wheel_zero_rejected(self):
        text = VALID.replace("delta: -5", "delta: 0")
        with pytest.raises(RouteValidationError, match="delta"):
            load_route_text(text)
