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
    operations: []       # 可选，该点位执行的操作序列，按顺序执行
```

点位不保存独立坐标；位置只由点击操作的 `pos` 或拖动操作的 `from` / `to` 指定。旧 YAML 中的点位层 `pos` 可兼容读取，但会被忽略；新建路线、结构化导出及执行缓存不再生成该字段。源码模式直接保存或导出时仍保留用户原文。取点工具将采集位置独立保存在 `state/picker.json`，复制 `[x, y]` 后可直接粘贴至管理器的点击或拖动坐标输入框；取点工具不生成路线或默认动作。

## 操作对象

所有操作共有的可选字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `name` | string | 动作显示名，可为空或重复；缺失时为空字符串，结构化序列化时省略空名称 |
| `jitter` | number | 随机抖动像素半径（执行时对换算后的物理坐标加 ±jitter 随机偏移），默认 0 |

`name` 与 `type` 同级，六类操作均支持。提供名称时必须为字符串，`null`、数值、布尔值、数组和对象均无效，错误定位到 `points[i].operations[j].name`。名称原样保留于 YAML 和 JSON 执行缓存，AHK 忽略该显示元数据；格式仍为 `route/v1`，旧路线无需迁移。

各类型专有字段：

### click 点击

```yaml
- { type: click, name: 打开仓库, pos: [0.95, 0.95], button: left, clicks: 1 }
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

## 校验规则

Rust `game-helper-core` 负责数据校验与规范执行缓存的生成，AHK executor 读取缓存执行操作。

YAML 是源文件，`routes/_cache/<文件 stem>.json` 是执行缓存。Rust 管理器显式保存或执行路线时会生成缓存。AHK 热键只读取缓存；首次使用拷贝进来的 YAML 路线时，应先在管理器保存，再绑定热键或重载 daemon。外部修改 YAML 后也需在管理器重新打开并保存，单独重载 AHK 不会转换 YAML。

1. `format` 必须为 `route/v1`
2. `name` 必须为非空字符串
3. `target` 三字段至少一个非空
4. `points` 必须为列表；每个点位 `id` 为正整数，不要求点位层 `pos`
5. 操作 `type` 必须为六种之一，且各自必填字段存在、类型正确；点击 `pos` 和拖动 `from` / `to` 必须为二元数组且数值在 0~1
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
