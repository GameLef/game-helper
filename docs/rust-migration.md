# Rust 原生客户端迁移

Rust + GPUI Kit 0.9.0 原生 Windows 客户端。管理器与 `--picker` 使用同一 exe；使用项目内置 AHK v2 的 daemon 服务执行核心。客户端没有 Tauri、WebView 或网页前端。

## 当前交付范围

当前为**原生预览版**。路线与配置核心、Windows IPC、管理器和独立取点工具已接入；完整人工验收尚未收尾。按用户最新要求，Python 客户端、依赖与回退入口已删除；仓库 `start.bat` 统一启动 Rust，不提供 Python 回退。默认入口切换不代表下方待验收项已经通过。

| 模块 | 当前实现 |
|---|---|
| `game-helper-core` | route/v1、六类操作与默认值、带路径校验、YAML/JSON、BOM 游标、配置未知字段、脏文档、冲突检测、原文备份与原子保存、导入和删除协调 |
| `game-helper-windows` | 窗口元数据、客户区物理坐标、DPI、Ctrl+Alt+P 专用线程、分模式单实例、内置 AHK 优先定位、daemon 服务、超时 IPC |
| 管理器 | 路线列表及坏文件修复、新建草稿、属性、点位新增/删除/排序、六类操作逐项参数/排序/删除、YAML 高亮 Editor、导入覆盖/副本/取消、导出、保存保护、绑定、执行、daemon 状态及游标 |
| 取点工具 | 独立进程、窗口列表、延时准星选窗、实时坐标、全局热键与延时按钮采点、点名编辑/删除、纯坐标复制、按进程/窗口类/完整标题隔离自动保存与恢复、退出等待写入并释放热键 |

操作参数中的 `pos/from/to` 使用 `[x, y]`，`up` 使用 `true/false`；字符串参数直接输入，时间单位为 ms。坐标基于客户区物理像素及 `(width-1)/(height-1)`。

## 构建与启动

需要 Windows 10+、rustup、Visual Studio 2022 C++ 桌面工具与 Windows SDK。构建脚本使用 vswhere 加载 MSVC，并使用 VS 随附 CMake。

固定 Rust 1.99.1 MSVC 与 `gpui-kit = "=0.9.0"`。既有调查发现 GPUI 0.3.11 的标准库 API 无法在 1.90/1.93 构建，因此保留项目内工具链。

```bat
scripts\cargo.cmd check --workspace --locked
scripts\cargo.cmd test --workspace --locked
scripts\cargo.cmd clippy --workspace --all-targets --locked
scripts\cargo.cmd fmt --all -- --check
scripts\cargo.cmd build -p game-helper-desktop --release --locked

start.bat
start.bat --picker
```

也可直接运行：

```bat
target\release\game-helper.exe --app-root "E:\Project\github\game-helper"
target\release\game-helper.exe --picker --app-root "E:\Project\github\game-helper"
target\release\game-helper.exe --probe
```

`scripts/cargo.cmd` 把 Cargo、rustup、临时文件与编译产物分别放在 `.tools/cargo`、`.tools/rustup`、`.tools/tmp`、`target`。不切换系统默认 Rust。

根目录解析：显式 `--app-root` → exe 目录 → 开发目录向上查找 `ahk/daemon.ahk` 与 `config.json`。显式目录无效会显示错误，不退回其他数据目录、不创建生产数据。当前错误窗口要求修正参数后重新启动，尚未提供选目录恢复按钮。

## 保存、执行与进程约定

- 新建路线保留为内存草稿，填入至少一个目标条件后才能保存。显式保存、执行或开启路线 Switch 会生成、更新 `routes/_cache/<文件stem>.json` 执行缓存。
- 现有 YAML 路线首次使用热键前，可以在管理器保存生成缓存，或设置绑定后开启 Switch 校验磁盘路线并生成缓存；仅复制 YAML 和绑定不会生成缓存。外部修改 YAML 后也需重新打开并保存；重载 AHK 只重读配置与 JSON 缓存，不会转换 YAML。
- 源码保存保留已验证原文。结构化修改后保存会提示规范化与注释风险，并保留 `routes/.backups`。
- 磁盘变化会阻止静默覆盖；配置没有实际变化时不重写原文。绑定修改保留未知字段。
- `hotkeys.bindings[].enabled` 持久化快捷键启用状态；旧绑定缺失此字段按开启读取，新监听绑定默认保存为关闭，修改已有绑定保留原开关状态，已开启时应用同键互斥。允许多条路线保存同键；每次开启时一次配置保存关闭其他同键路线，最后开启者生效，关闭不恢复旧路线。外部重复开启项按数组最后一项胜出。
- 快捷键通过“设置快捷键”监听弹窗直接捕获，捕获后无需另点保存；取消保留原绑定，清除按钮解除绑定。“保存起始点”独立保存 `startAt`。路线列表开关只影响快捷键，服务就绪时按钮试跑不受关闭状态限制。
- 切换路线/关闭有保存、放弃、取消选择。错误源码切换失败时保留输入。
- 执行要求已保存且经校验的数据，`startAt/only` 使用 1-based 列表位置，与 `point.id` 无关。
- 所有回放通过 daemon 服务发送受理请求；服务未启动时试跑禁用，先在服务控制中启动。没有独立执行进程或离线回放入口。受理回执不代表回放完成，超时不自动重发。
- daemon busy 时拒绝重绑/重载。保存遇 busy 会提示稍后手动重载。
- 关闭管理器保留 daemon 服务；停止服务结束服务进程。服务忙碌时阻止重复提交。
- 急停键显示 `config.json` 的真实 `hotkeys.stopAll`，默认 F12。
- 取点窗口只有一条全局热键通道。采样失败不生成非法点位。点位按进程名、窗口类和完整标题保存于 `state/picker.json`，更换目标自动切换列表，无需清空；标题变化对应另一份列表。正常关闭等待最新修改写入，失败可重试、继续编辑或明确放弃。
- `.yaml/.yml` 同 stem 共用执行缓存，创建/删除检测到该冲突时拒绝继续。

## 本轮实际观察与诊断记录（2026-11-07～08）

以下是 Shell/Cargo 的原始诊断及 Windows UI Automation 的运行观察。专用 `run_verification(test)` 返回“本地 Vitest 未安装”，未产出 Rust 验证凭证；不将下表写成专用验证器已通过。

| 项目 | 实际结果与边界 |
|---|---|
| Rust 工作区测试 | 命令 exit 0；日志报告 core 51、Windows 9，合计 60 passed / 0 failed；另有 1 个 ignored 子进程服务入口，由父测试显式启动 |
| Debug 与 Release | Cargo 构建命令均观察到 exit 0；产物位于 `target/debug`、`target/release` |
| 类型、风格与格式 | `cargo check --workspace`、`clippy --workspace --all-targets`、`fmt --all -- --check` 最终命令均 exit 0，Clippy 保留文档/风格 warning |
| 管理器原生窗口 | 观察到中文表单↔YAML 往返、坏 YAML 切换保留、显式源码保存/缓存、脏关闭拦截与放弃关闭 |
| 全局取点 | 专用 WinForms 无害窗口中，准星选窗、失焦 Ctrl+Alt+P 恰好生成一个点、点名保存、YAML/缓存、释放热键与正常关闭均已观察 |
| IPC 契约 | 独立 Win32 子进程覆盖 Ping/Reload/Stop/COPYDATA、busy 拒绝、挂死超时和锁；尚未完成真实 AHK 六类操作闭环 |
| 视觉与系统输入 | UI Automation 写入中文不覆盖系统 IME 组合输入；截图文件已生成，但本轮图片读取超时，没有宣称视觉验收完成 |

原始本地日志：`.tools/workspace-tests.log`、`desktop-build.log`、`release-build.log`、`workspace-check.log`、`clippy.log`、`format-check.log`。`.tools/` 为忽略目录。

`GPUI Kit test-support` 与既有锁中的 `taffy 0.15.1` 发生解析冲突（可选 GPUI Fast 要求 `=0.15.0`）。当前移除该开发特性，保留生产 GPUI 依赖；未为测试引入其他 UI 框架。Clippy 尚有文档及风格 warning，不宣称零警告。

Windows UI Automation 脚本（先在无测试进程运行时执行 `scripts/prepare-smoke.ps1` 创建/重置隔离数据；脚本含中文部分使用 UTF-8 BOM，兼容 Windows PowerShell 5.1）：

- `scripts/smoke-manager.ps1 -TargetProcessId <pid>`：仅用于 `.tools/acceptance sandbox` 管理器，检查并修改专用 Smoke 路线。
- `scripts/smoke-manager-launch.ps1`：在已有隔离副本中启动 Debug 管理器后调用上述脚本。
- `scripts/smoke-picker.ps1`：创建专用无害目标窗口，启动隔离取点进程，模拟一次采点组合键。
- `scripts/inspect-native.ps1 -TargetProcessId <pid>`：输出 UIA 树和项目内截图。

脚本只作为运行观察工具；不可用生产路线替换其隔离目录。UIA 对滚动区域外按钮的 Invoke 可能不生效，取点观察采用可见的准星路径；列表鼠标选择仍需人工补验。系统负载较高时需要等待 Editor 的可访问性值就绪，脚本已加入有界重试。

## 便携预览包

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\bundle.ps1
```

输出到项目 `dist/game-helper-native-preview-<时间>` 及同名 zip，包含 exe、AHK 脚本、普通 x64 AutoHotkey v2.0.31 解释器、完整许可证、对应源码归档、来源及版本校验信息、空路线目录、默认开启 daemon 自启的配置、迁移说明及 SHA-256 清单。默认不复制用户路线或绑定；确需复制时显式传 `-IncludeUserData`；保留用户自启设置，移除构建机专属 `ahkPath`。不打包构建缓存、日志、游标、备份或 Python 环境。

客户端运行无需 Python/Rust/Tauri，也无需安装 AutoHotkey。服务解释器定位顺序为应用目录 `ahk/runtime/AutoHotkey64.exe` → 配置 `ahkPath` → 系统安装；内置解释器存在时优先使用，旧本机绝对路径不会阻止便携运行。打包前校验解释器、许可、源码与元数据的完整性及哈希，运行时无需下载。必须分发完整便携目录，`cargo build --release` 本身不会把资源嵌入管理器 exe。PE 依赖检查还发现 `VCRUNTIME140.dll`，目标电脑需具备 Microsoft Visual C++ x64 运行库；当前包未随附该 DLL，也尚未在干净 Windows 上验收。当前为预览包；仓库默认启动器已按用户要求切换为 Rust，Python 回退入口已删除，完整验收仍按下方待办继续。

便携启动已在项目内的中文/空格路径副本观察：管理器和 picker 均能从不同 cwd 定位 exe 同目录数据并正常关闭；SHA-256 清单核对一致。

## 仍待完成的验收

- 真实 AHK daemon 服务与专用窗口的 click/key/press/wheel/drag/wait、连续/从 N/仅 N、单步成功/失败/回绕、reload 复位、急停闭环。
- 中文系统 IME、文件对话框完整选择/取消、导入重名与删除部分失败的原生窗口流程。
- 100%/150%/200% DPI、多显示器负坐标、窗口移动缩放；现有自动化仅覆盖当前桌面环境。
- 取点窗口列表的滚动后鼠标选择、准星和热键冲突恢复，以及重复长按与多次切换的人工验收。
- 默认入口已切换为 Rust，Python 端已删除；继续完成上述功能矩阵与干净 Windows 验收，当前预览交付不等于计划全部验收完成。

## 管理器界面重设计

Rust 管理器已采用路线库、点位序列和动作编辑分栏的浅色工作台，新增内存搜索、属性/文件/服务控制折叠区、固定执行栏和可键盘取消的确认层。原有文档与执行契约保持不变；通过 `start.bat` 统一启动 Rust 客户端。

本轮 Debug/Release 构建、工作区 60 项测试、Clippy 和格式检查通过；隔离 UI 验收观察到中文双视图往返、保存/缓存、脏关闭保护、搜索、900×640 布局、空状态、120 个长名称点位及 Escape 取消。实际 DPI 为 96；截图已捕获，当前模型无法读取图像，未宣称逐像素目视验收。详细选择、脚本和实际截图见[界面设计记录](manager-design.md)。

## 删除 Python 与默认入口切换验证（2026-11-08）

本次已删除 Python 端，`start.bat` 统一启动 Rust，无 Python 回退。`cargo fmt --all -- --check`、工作区测试、工作区 `cargo check` 与 Release 构建均 exit 0；测试为 60 passed / 0 failed，另有 1 个 ignored 私有子进程服务入口。构建保留原有 18 个缺少文档警告。本轮未运行 GUI 验收，上述真实 AHK、IME、多 DPI 等待办继续保留；上述历史窗口观察不作为本次 GUI 复验结果。

## 取点工具自动保存与坐标复制

已移除取点工具的路线保存/导出及覆盖确认，点位独立保存至 `state/picker.json`，按进程名、窗口类与完整标题隔离。坐标区域和复制按钮均输出六位小数的 `[x, y]`，可粘贴至管理器动作坐标框。正常关闭等待自动保存，失败时保留内存并提供重试或明确放弃。

本次工作区 75 项测试通过，另有 1 个私有子进程入口保持 ignored；Debug/Release 构建、Clippy 和格式检查通过，保留既有警告。Release 的三个隔离脚本 `smoke-picker.ps1`、`smoke-picker-layout.ps1`、`smoke-picker-coordinates.ps1` 均通过，覆盖标题隔离、重启恢复、快速编辑关闭、读写失败、剪贴板临时占用恢复、96 DPI 布局以及真实 Ctrl+V 粘贴保存并重启管理器恢复。详细行为与截图见[取点工具说明](picker-design.md)。这些检查没有请求 AHK 游戏回放。

## 官方资料

- [GPUI Kit 入门](https://gpui-kit.com/docs/getting-started)
- [Windows 安装要求](https://gpui-kit.com/docs/installation)
- [GPUI Kit 0.9.0](https://crates.io/crates/gpui-kit/0.9.0)
- [源码编辑器](https://gpui-kit.com/component/editor)
- [测试支持](https://gpui-kit.com/docs/test)


## 快捷键与内置服务回归

本轮新增或调整的脚本均使用隔离副本；不要并行运行会争用前台窗口的 GUI 检查。

```powershell
pwsh -NoProfile -File scripts\smoke-manager-hotkeys.ps1 -Executable target\release\game-helper.exe
pwsh -NoProfile -File scripts\smoke-native-drag.ps1
pwsh -NoProfile -File scripts\smoke-portable.ps1 -BundleDir "dist\game-helper-native-preview-<时间>"
```

- `smoke-manager-hotkeys.ps1`：自建 `autoStartDaemon=false` 的路线与配置，验证监听捕获/取消/清除、旧绑定兼容、同键互斥、开关不选择路线、单独保存起始点及重启恢复。录制用按键消息仅发给脚本创建的管理器窗口，不发送全局输入；离线断言要求生产服务当前未运行，检测到服务时停止检查，不操作该服务。
- `smoke-native-drag.ps1`：默认使用项目内置解释器；可传 `-AutoHotkey <绝对路径>` 指定测试解释器。全部路线回放通过唯一标题的隔离服务 IPC 执行，保留执行器的发送模式恢复、急停与异常清理 harness。仅操作脚本创建的空白 WinForms 目标窗口。
- `smoke-portable.ps1`：校验发布清单，在中文/空格路径副本和不同 cwd 打开管理器及取点工具；关闭副本自启、清空副本绑定并写入失效旧 `ahkPath`，直接启动包内解释器下的唯一标题服务，核对实际进程路径与 Ping/Stop。此步骤无需网络或系统安装的解释器；Rust 解释器选择顺序由路径解析测试另行覆盖。脚本不宣称已在禁网或卸载 AHK 的干净 Windows 上验收。

本轮 Release 的快捷键 UI 回归已 exit 0，证据见[快捷键回归摘要](../.tools/manager-hotkeys-5e8fa5bef82646c4987b8d9bf57a4d67/summary.json)：监听 F10 捕获、取消、清除、旧绑定兼容、同键开关互斥、独立保存起始点及重启恢复均通过，`playbackRequested=false`。内置 AHK 2.0.31 的原生拖动服务回归也已 exit 0，覆盖 9 个服务回放用例以及发送模式恢复、四阶段急停、异常清理 harness，证据见[服务回放摘要](../.tools/native-drag-2a439400392a45538ad86046fe79b5fb/summary.json)，生产服务窗口保持不变。真实服务捕获租约集成检查已通过（5.37 秒）。最终工作区测试为 **126 passed / 0 failed / 2 ignored**；其中捕获租约集成检查已单独执行并通过，另一个私有 helper 由父测试调用。最终 Release 构建、格式检查及 Clippy 均通过，Clippy 保留既有警告。便携检查也已 exit 0，证据见 `.tools/便携 测试-46283832149e4cda9a0f2231b41e9e17/portable-summary.json`：中文及空格路径、不同工作目录下的管理器和取点工具均正常启动，包内 AHK 进程路径正确，独立服务 Ping/Stop 成功，未请求路线回放；历史测试数量和旧截图不作为本轮已通过的证明。
