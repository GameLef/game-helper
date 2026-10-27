# game_helper v2

路线回放工具链：**Python 管、AHK 跑**。

```
YAML 路线（人写/人读）
   │ 保存/绑定时转换（非热路径）
   ▼
routes/_cache/*.json ──预载──▶ daemon.ahk（常驻，原生热键毫秒级响应）
   │                              ▲  窗口消息(Ping/Reload/Stop/CopyData-Run)
   │                              │
   └── runner.ahk（调试直启）    python 管理器 GUI（启停/重载/执行按钮）
```

## 功能

- **路线文件**：YAML，一个文件一条路线，点位为对象、内嵌操作序列，支持注释（格式见 [docs/route-format.md](docs/route-format.md)）
- **取点工具** `python/picker.py`：选目标窗口（下拉 + 十字准星）→ 实时鼠标相对客户区/归一化坐标 → `Ctrl+Alt+P` 采点 → 保存/导出 YAML
- **脚本管理器** `python/manager.py`：路线列表、表格/源码双视图编辑、导入导出、热键绑定、daemon 启停控制、执行/试跑按钮
- **执行核心**：AHK v2。daemon 常驻模式为主（热键→首动作 <50ms），runner CLI 为调试保底
- **急停**：全局 `F12`（config.json 可改）

## 安装

1. Python 3.12+，然后：

   ```
   pip install -r v2/requirements.txt
   ```

2. AutoHotkey v2（默认路径 `C:\Program Files\AutoHotkey\v2\AutoHotkey64.exe`；不同路径请改 `config.json` 的 `ahkPath`）

## 使用

```
# 启动管理器（推荐入口；可自动拉起 daemon）
python python/manager.py

# 只启动取点工具
python python/picker.py

# 手动启动/停止 daemon（也可在管理器顶栏操作）
python -c "import sys; sys.path.insert(0,'python'); from core import launcher; print(launcher.start_daemon())"
```

演示：打开记事本 → 管理器选中 `demo_notepad` → 「从头执行」，或按绑定热键 F9。

### 按热键执行（daemon 模式，单步）

1. 管理器左侧选中路线 → 底部「绑定热键…」输入如 `F9`（支持 `ctrl+alt+F8` 写法），并设起始点
2. daemon 自动重载；**热键为单步模式：每按一次执行一个点位，再按执行下一个，走完最后一个点后回到起始点循环**
3. 当前进度显示在管理器顶栏 daemon 状态区（如 `测试 ▸ 2/5`），触发时屏幕另有 `第 N/M 点` ToolTip 提示
4. 执行失败或急停时游标不前进，再按热键重试同一点
5. `F12` 全局急停；托盘图标可暂停/恢复热键
6. 需要一口气跑完整条路线时，用管理器的「从头执行 / 从此点开始」按钮（连续执行）

### 调试单条路线

```
"C:\Program Files\AutoHotkey\v2\AutoHotkey64.exe" ahk/runner.ahk routes/_cache/demo_notepad.json --start 2
```

## 目录

```
v2/
├── ahk/            daemon.ahk / runner.ahk / lib(JSON+executor)
├── python/
│   ├── core/       route_model / window_utils / launcher / hotkey_service
│   ├── widgets/    point_table（表格编辑）/ daemon_bar（daemon 控制条）
│   ├── picker.py   取点工具
│   └── manager.py  管理器
├── routes/         路线 YAML + _cache/（自动生成的 JSON）
├── logs/           执行日志（含热键→首动作延迟）
├── state/          cursor.json（daemon 单步游标，管理器进度显示用）
├── tests/          pytest 单元测试
├── docs/           路线格式规范
└── config.json     AHK 路径 / 热键绑定 / daemon 自启开关
```

## 注意事项

- **管理员权限**：若目标程序（如游戏）以管理员运行，daemon/runner 与管理器也需以管理员启动，否则 SendInput 无法注入
- **DPI**：两端均声明 Per-Monitor DPI Aware；取点与执行都基于窗口客户区归一化坐标，换分辨率无需重标
- **日志**：`logs/run_*.log` 记录每步动作与延迟，点空排查先看这里
- 本项目仅用于个人学习与内部交流；自动化操作游戏有违反用户协议与封号的风险，请自行评估

## 已知限制（本机验证结论）

- 本机 AHK v2 环境下 `Gui()` 初始化会静默阻塞（已用脚本主窗口改名方案替代），若换机器出现 daemon 无响应可先查此处
- 表格编辑视图保存时整段重写 points，YAML 点位级注释会丢失（文件级注释保留）
