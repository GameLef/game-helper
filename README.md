# game-helper

面向 Windows 的原生路线管理器与独立取点工具。使用 **Rust + GPUI Kit** 编辑路线，通过 **AutoHotkey v2** 执行鼠标、键盘操作，适合需要按点位组织和回放操作的场景。

当前为原生预览版本，仅维护 Rust 桌面客户端。界面采用原生组件，无需 WebView；仓库内附带 x64 AutoHotkey 运行时。

## 功能概览

| 功能 | 说明 |
| --- | --- |
| 路线管理 | 新建、搜索、导入、导出、删除路线，支持表单与 YAML 源码两种编辑方式。 |
| 点位与动作编辑 | 点位可复制、排序；动作支持命名、插入、拖动排序及展开/收起，展开状态独立保存。 |
| 七类动作 | 点击、单次按键/松开、按住、滚轮、拖动、等待、原生拖动。 |
| 独立取点 | 搜索窗口或延时选窗，查看屏幕、客户区、归一化坐标，通过 `Ctrl+Alt+P` 或延时按钮采点。 |
| 单步回放 | 每按一次路线快捷键执行一个点位，也可在管理器中试跑选中点、保存单步起始位置。 |
| 文件保护 | 提示未保存修改，保存时检测磁盘冲突，对被改写的原文件留存备份。 |
| 执行服务 | 在管理器中启动、停止、重载 AHK 服务，查看执行状态，使用全局急停键中断回放。 |

## 环境要求

- **运行平台**：Windows 10 及以上，x64。
- **源码构建**：Rust 工具链、Visual Studio 2022 的“使用 C++ 的桌面开发”工作负载及 Windows SDK，使用 `x86_64-pc-windows-msvc` 工具链。
- **版本约定**：[rust-toolchain.toml](rust-toolchain.toml) 固定 Rust **1.99.1**，[Cargo.toml](Cargo.toml) 固定 GPUI Kit **0.9.0**，依赖版本由 [Cargo.lock](Cargo.lock) 锁定。
- **运行依赖**：如系统缺少 `VCRUNTIME140.dll`，需安装 Microsoft Visual C++ x64 运行库。运行已编译的客户端无需安装 Rust 或系统版 AutoHotkey。

## 从源码构建与启动

在已安装 Git、Rustup 和上述 C++ 构建工具的 Windows 终端中执行：

```bat
git clone https://github.com/GameLef/game-helper.git
cd game-helper
copy config.example.json config.json
cargo build -p game-helper-desktop --release --locked
```

首次运行 Cargo 时，Rustup 会按工具链配置准备指定版本；首次构建还需要下载依赖。构建产物为 `target/release/game-helper.exe`。

```bat
rem 打开路线管理器
start.bat

rem 打开独立取点工具
start.bat --picker
```

[start.bat](start.bat) 优先启动 release 产物，其次使用 `target/debug/game-helper.exe`，并将仓库根目录传给客户端。它只负责启动，不会自动编译。若未找到 exe，请执行上面的标准 `cargo build` 命令；脚本中现有的 `scripts\cargo.cmd` 提示指向未纳入仓库的本地工具。

开发时也可以直接运行：

```bat
cargo run -p game-helper-desktop --locked -- --app-root .
cargo run -p game-helper-desktop --locked -- --app-root . --picker
```

客户端还提供 `--probe` 原生能力验证窗口，用于检查输入框、YAML 编辑器、文件对话框和全局采点热键。

### 应用目录与携带运行

管理器和取点工具要求应用根目录同时包含本地 `config.json` 与 [ahk/daemon.ahk](ahk/daemon.ahk)。首次使用时从 [config.example.json](config.example.json) 复制生成本地配置。可用 `--app-root` 显式指定；未指定时，程序从 exe 所在目录开始向上查找。

若要自行整理可携带的运行目录，可采用以下结构：

```text
game-helper/
├─ game-helper.exe           从编译产物复制
├─ config.json               按本机需求设置
└─ ahk/                     保留仓库中的整个目录
   ├─ daemon.ahk
   ├─ lib/
   └─ runtime/
```

在该目录中运行 `game-helper.exe` 或 `game-helper.exe --picker`。应用目录需要可写，以便保存路线、配置、状态和日志。需要迁移个人数据时，再复制自己使用的路线及状态目录。

## 首次使用

远程仓库不包含个人路线。首次打开路线库为空属于正常情况；保存路线、取点或启动服务时，程序会按需创建对应数据目录。

1. **准备配置**：首次克隆后执行 `copy config.example.json config.json`，再按需修改本地配置。模板中的 `hotkeys.bindings` 为空列表，不包含个人路线或快捷键。已有本地配置时请保留自己的设置，无需覆盖。
2. **获取坐标**：打开取点工具，搜索目标窗口或使用延时准星选窗。启用全局采点热键后，将光标移入目标客户区并按 `Ctrl+Alt+P`，也可直接使用延时采点按钮。点击坐标或复制按钮即可复制归一化坐标；采集记录按进程名、窗口类和完整标题分别自动保存，可调整顺序。
3. **编排路线**：在管理器中新建路线，填写目标窗口匹配条件，添加点位和动作。将坐标粘贴到点击位置或拖动起止位置，保存路线。
4. **试跑单点**：在服务控制中启动执行服务，再点击“试跑选中点”。路线存在未保存修改时，需要先保存。配置模板关闭了服务自启，也可在管理器中开启随启动运行。
5. **设置快捷键**：保存路线后点击“设置快捷键”，在弹窗中按下按键或组合键。新绑定默认关闭，需要打开路线列表右侧的开关才会生效；“保存起始点”单独设置单步循环的起点。
6. **单步执行与急停**：每按一次路线快捷键执行一个点位，成功后推进到下一点；最后一点成功后，下次从设定起点继续。执行失败或急停时游标不前进。默认全局急停键为 `F12`。

同一快捷键可保存给多条路线，开启其中一条时，其他使用同键的路线会自动关闭。关闭路线快捷键不影响服务就绪时的按钮试跑。急停会中断当前回放并保留服务；管理器中的“停止服务”会结束服务进程。**关闭管理器后，已启动的执行服务仍会继续驻留。**

## 路线格式

路线使用 `route/v1` YAML 格式，保存在本地 `routes/` 下，支持 `.yaml` 和 `.yml`。下面的示例匹配记事本进程，在客户区中心点击后等待 200 毫秒；使用前请将目标和动作改为自己的需求。

```yaml
format: route/v1
name: 示例路线
note: 在目标客户区中心点击并等待
target:
  process: notepad.exe
  windowClass: ""
  title: ""
points:
  - id: 1
    name: 第一个点位
    note: ""
    operations:
      - type: click
        name: 点击中心
        pos: [0.5, 0.5]
        button: left
        clicks: 1
      - type: wait
        name: 等待界面响应
        ms: 200
```

`target` 中的 `process`、`windowClass`、`title` 至少填写一项。执行器优先按进程名查找，否则使用窗口类，最后使用标题子串，选择首个可见匹配窗口；多个字段不会组合筛选。点位 `id` 必须是正整数。位置使用客户区归一化坐标 `[x, y]`，范围为 `0～1`，`[0, 0]` 为左上角，`[1, 1]` 为右下角，换算使用客户区的 `width - 1`、`height - 1`。

坐标存放在具体动作中。旧路线点位层的 `pos` 字段兼容读取但会被忽略，结构化保存和执行缓存不会保留该字段。

| 动作类型 | 含义 | 主要参数 |
| --- | --- | --- |
| `click` | 在指定位置点击 | `pos`；`button` 为 `left/right/middle`，默认 `left`；`clicks` 为正整数，默认 `1`。 |
| `key` | 发送一次按键或仅松开 | `key` 为 AHK 键名；`up: false` 发送一次完整按键，`up: true` 仅发送松开，默认 `false`。持续按住请使用 `press`。 |
| `press` | 按住按键一段时间后松开 | `key`、正整数 `ms`。 |
| `wheel` | 滚动鼠标滚轮 | 非零整数 `delta`，正值向上、负值向下。 |
| `drag` | 按指定时长拖动 | `from`、`to`；正整数 `duration`，默认 `200` 毫秒。 |
| `wait` | 等待 | 正整数 `ms`，单位为毫秒。 |
| `drag_native` | 使用 AHK 原生鼠标拖动 | `from`、`to`；`speed` 为 `0～100` 的整数，默认 `2`；可设置 `pre_drag_distance`、`pre_drag_speed`、`pre_drag_delay`。 |

所有动作均可填写可选名称 `name`。完整字段校验及默认值见 [路线模型](crates/game-helper-core/src/route.rs) 和 [校验实现](crates/game-helper-core/src/route/validate.rs)。

源码模式保存会保留通过校验的原文；表单编辑后保存会规范化 YAML，可能改变注释和排版，管理器会提示并备份原文件。

## 配置说明

本地 `config.json` 已加入忽略规则，管理器保存设置时会更新它，不会随仓库分发。仓库提供 [config.example.json](config.example.json) 作为初始化模板，其内容如下；请将它复制为本地配置后按需调整：

```json
{
  "routesDir": "routes",
  "hotkeys": {
    "stopAll": "F12",
    "bindings": []
  },
  "autoStartDaemon": false,
  "autoReloadOnSave": true
}
```

| 字段 | 说明 |
| --- | --- |
| `routesDir` | 当前执行端仅支持 `"routes"`，不可改为其他目录。 |
| `hotkeys.stopAll` | 全局急停键，当前为 `F12`。 |
| `hotkeys.bindings` | 路线热键列表；每项包含 `hotkey`、`action: "run_route"`、路线文件名去掉扩展名后的 `route`、从 `1` 开始的点位序号 `startAt` 和 `enabled`。建议通过管理器设置。 |
| `autoStartDaemon` | 是否随管理器启动执行服务；模板值为 `false`。 |
| `autoReloadOnSave` | 保存路线后是否自动重载服务；模板值为 `true`。 |
| `ahkPath` | 可选的外部 AutoHotkey v2 解释器绝对路径，仅在内置运行时缺失时使用。 |

服务优先使用 [ahk/runtime/AutoHotkey64.exe](ahk/runtime/AutoHotkey64.exe)，其次使用非空的 `ahkPath`，最后探测系统安装位置。仅在内置运行时缺失时校验 `ahkPath`；非空路径无效则报错，不再继续系统探测。不依赖 `PATH` 搜索解释器。

仓库自带 AutoHotkey **2.0.31 x64**，版本、来源及 SHA256 记录见 [metadata.json](ahk/runtime/metadata.json)。分发内置运行时时，请一并保留 [完整上游许可证](ahk/runtime/license.txt) 和 [对应源码归档](ahk/runtime/source.zip)。

## 仓库结构

下面是 Git 跟踪、克隆后可获得的主要内容：

```text
game-helper/
├─ .gitignore
├─ Cargo.toml                        Rust workspace 与共享依赖
├─ Cargo.lock                        锁定依赖版本
├─ rust-toolchain.toml                工具链版本与组件
├─ README.md
├─ start.bat                         桌面客户端启动入口
├─ config.example.json               本地配置初始化模板
├─ ahk/
│  ├─ daemon.ahk                     热键、单步游标与 IPC 服务
│  ├─ lib/
│  │  ├─ JSON.ahk                    JSON 支持
│  │  └─ executor.ahk                动作执行器
│  └─ runtime/                       解释器、许可证、元数据与源码归档
└─ crates/
   ├─ game-helper-core/              路线、配置、校验、存储与编辑状态
   │  ├─ src/
   │  ├─ tests/
   │  └─ examples/route_json.rs       路线校验与 JSON 输出工具
   ├─ game-helper-windows/           DPI、窗口、全局热键、进程与 IPC
   │  ├─ src/
   │  └─ tests/
   └─ game-helper-desktop/           GPUI Kit 管理器、取点工具与验证窗口
      └─ src/
```

## 执行机制与检查

```text
Rust 管理器 ──校验、保存──▶ routes/*.yaml
     │
     └──生成执行缓存──▶ routes/_cache/*.json ──预载──▶ AHK daemon
     │                                                   ▲
     └──────────── Win32 IPC：状态 / 重载 / 停止 / 执行 ────┘
```

路线保存、试跑及启用路线快捷键时会生成或更新执行缓存。AHK 服务读取缓存完成回放，Rust 客户端负责编辑、校验与服务控制。取点工具的采集记录独立保存，复制坐标后再粘贴到路线动作中。

可从仓库根目录执行以下检查：

```bat
rem 核心层契约测试，不需要打开桌面界面
cargo test -p game-helper-core --locked

rem 工作区测试，需在 Windows 环境运行
cargo test --workspace --locked

rem Rust 格式检查
cargo fmt --all -- --check

rem 校验自己的路线并将执行用 JSON 输出到终端，不写入缓存
cargo run -p game-helper-core --example route_json --locked -- routes/example.yaml
```

Windows 测试涉及真实 Win32 API，部分用例会启动内置 AHK 进程；影响前台窗口或输入的部分用例默认忽略。自动测试通过仍需结合实际目标程序验证回放效果。

## 常见问题

- **找不到客户端 exe**：先执行源码构建命令，再运行 [start.bat](start.bat)。远程仓库不包含 `target/` 下的编译产物。
- **未能定位应用目录**：用 `--app-root` 指向同时包含配置文件与 AHK 服务脚本的目录；携带运行时请保留整个 `ahk/` 目录。
- **首次没有路线，快捷键也无效**：个人路线已被忽略，需要自行新建或导入；检查配置中是否仍有未随仓库分发的路线绑定，确认执行服务已启动且路线快捷键已开启。
- **“试跑选中点”不可用**：确认路线已保存、点位有效，执行服务已启动且空闲，服务状态读取正常。
- **无法采点或目标失效**：还原最小化的目标窗口，将光标移入目标客户区；窗口关闭或重建后重新选择。
- **输入无法送达目标程序**：检查目标窗口匹配条件与权限。目标以管理员身份运行时，管理器和 AHK 服务也需要相应权限。执行细节可查看本地 `logs/run_*.log`。
- **保存提示磁盘冲突**：文件在打开后被其他程序修改，请重新加载后再编辑保存。

当前版本在真实目标程序中的动作兼容性、系统输入法以及多 DPI、多显示器场景仍需持续验证；归一化坐标也不能保证界面布局改变后仍命中同一控件。
