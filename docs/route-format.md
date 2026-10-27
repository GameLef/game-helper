# 路线文件格式规范 · route/v1

一个文件 = 一条路线。坐标一律 **0~1 归一化、相对目标窗口客户区**（非全屏、非屏幕）。

## 顶层字段

```yaml
format: route/v1        # 必填，固定值，用于版本识别
name: 示例路线           # 必填，路线显示名
note: 备注说明           # 可选
target:                 # 必填，目标窗口标识，三个子字段至少填一个
  process: notepad.exe    # 进程名（不含路径），优先匹配
  windowClass: ""         # 窗口类名，如 UnityWndClass
  title: ""               # 窗口标题（包含匹配）
points: []              # 必填，点位列表（可为空）
```

## 点位对象

```yaml
points:
  - id: 1                # 必填，正整数，建议连续；执行时按列表顺序而非 id 排序
    name: 第一个点        # 可选，显示名
    note: 备注           # 可选
    pos: [0.312, 0.486]  # 必填，[x, y] 归一化坐标，0~1 闭区间
    operations: []       # 可选，该点位执行的操作序列，按顺序执行
```

## 操作对象

所有操作共有的可选字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `jitter` | number | 随机抖动像素半径（执行时对换算后的物理坐标加 ±jitter 随机偏移），默认 0 |

各类型专有字段：

### click 点击

```yaml
- { type: click, pos: [0.95, 0.95], button: left, clicks: 1 }
```

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `pos` | 是 | - | 归一化坐标 [x, y] |
| `button` | 否 | left | left / right / middle |
| `clicks` | 否 | 1 | 连击次数 |

### key 按键

```yaml
- { type: key, key: m, up: false }
```

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `key` | 是 | - | AHK v2 键名（a、F1、Enter、Space 等，见 AHK KeyList） |
| `up` | 否 | false | true 表示只松开（配合 press 使用） |

### press 按住

```yaml
- { type: press, key: w, ms: 800 }
```

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `key` | 是 | - | AHK v2 键名 |
| `ms` | 是 | - | 按住时长（毫秒），≥1 |

### wheel 滚轮

```yaml
- { type: wheel, delta: -5 }
```

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `delta` | 是 | - | 正数向上滚，负数向下滚，绝对值为格数 |

### drag 拖拽

```yaml
- { type: drag, from: [0.1, 0.1], to: [0.2, 0.2], duration: 300 }
```

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `from` | 是 | - | 起点归一化坐标 |
| `to` | 是 | - | 终点归一化坐标 |
| `duration` | 否 | 200 | 拖拽时长（毫秒） |

### wait 等待

```yaml
- { type: wait, ms: 300 }
```

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `ms` | 是 | - | 等待毫秒数，≥1 |

## 校验规则（route_model.py 与 AHK executor 双端执行同一套规则）

1. `format` 必须为 `route/v1`
2. `name` 必须为非空字符串
3. `target` 三字段至少一个非空
4. `points` 必须为列表；每个点位 `id` 为正整数、`pos` 为二元数组且数值在 0~1
5. 操作 `type` 必须为六种之一，且各自必填字段存在、类型正确
6. `jitter` ≥ 0；`clicks` ≥ 1；`press.ms` / `wait.ms` ≥ 1
7. 错误信息需定位：路线级错误直接说明；点位级错误带 `points[i]`（第几个点位）与点位 id/name；操作级错误带 `points[i].operations[j]`

## 坐标换算公式

```
物理像素 x = round(norm_x × (clientW - 1))      # 相对客户区左上角
物理像素 y = round(norm_y × (clientH - 1))
屏幕坐标   = ClientToScreen(目标窗口, [物理像素 x, 物理像素 y])
```

取点时反向：

```
norm_x = (鼠标屏幕坐标 - 客户区左上角屏幕坐标).x / (clientW - 1)
```

> 注意：两端都必须声明 Per-Monitor DPI Aware，否则高 DPI 缩放下坐标会错位。
