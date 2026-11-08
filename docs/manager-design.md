# 路线管理器界面重设计

Rust + GPUI Kit 0.9.0 原生管理器的浅色路线编排工作台。通过 `start.bat` 统一启动 Rust 客户端；Python 客户端与回退入口已删除。现有 AHK v2 daemon/runner 继续作为执行后端。

## 界面结构

- **路线库**：名称和文件名搜索，两行路线摘要、点数与绑定热键，明确的当前选择。新建区按需展开；新建仍是内存草稿，显式保存后才写入文件。坏文件可打开修复。
- **路线工作区**：原生窗口标题为“路线管理”，顶部不再显示品牌或“路线工作台”标题，仅保留路线库、服务状态与工具控件。工作区提供路线标题与保存状态、保存按钮、点位编辑 / YAML 切换。路线属性与文件操作分别展开，导出、定位文件和删除保持原有动作。
- **点位序列**：序号表示真实执行顺序，连接线展示点位关系；列表和动作编辑区独立滚动。首尾排序按钮禁用，零点位与零动作有引导。列表仅显示点位名称、序号及动作数量；编辑区的点位名称、备注默认收起，点击工具栏“点位信息”才显示在滚动内容首部，展开后也可滚出视野，不占固定区域。新生成 YAML / 缓存不保存点位层 `pos`，旧字段兼容读取并忽略，点击与拖动动作自身的坐标仍可编辑。
- **动作编辑**：固定工具栏只保留当前点位名、动作数量、点位信息、全部展开/收起以及七类添加入口。动作列表占据剩余高度独立滚动，新增动作追加末尾并定位到名称输入。每条动作包含可选名称、执行序号、类型、单项展开/收起和排序/删除控件；收起时继续显示名称和类型，输入实体保留，折叠不触发校验或脏状态。排序、增删与保存保留其余动作的展开状态，切换点位或 YAML 重建表单时恢复默认展开。参数标注单位与输入格式，点击的鼠标按键使用“左键 / 右键 / 中键”下拉框，按下 / 松开的按键动作使用“按下 / 松开”下拉框。名称与参数均提供包括动作序号的中文可访问名称和稳定 ID。
- **固定执行栏**：起始点、选中点试跑、热键绑定、单步进度及真实急停键；已移除管理器的“执行路线”按钮和整条路线连续执行入口。切换路线时起始点读取该路线绑定值，无绑定时显示 1。执行服务和独立单点试跑进程分别提示状态；受理请求不表示试跑完成。
- **确认区域**：保存 / 放弃 / 取消等原有分支使用覆盖式确认区域；焦点进入取消按钮，Escape 可取消。确认层在编辑器与调整面板之上处理点击。

布局设计基准为 1280×900；小于 1100px 时路线库默认折叠，可通过顶栏重新展开。用户手动选择的展开状态在本次会话中保留。最小客户区为 900×640，原生标题栏与边框另计；本机紧凑截图为 916×679。

## 紧凑动作编排布局

点位基础信息默认不渲染表单；点击固定工具栏的“点位信息”才在滚动区首部展开，名称和备注可以随内容滚出视野。工具栏只显示点位摘要、动作数量、信息开关、全部展开/收起以及七类添加按钮。动作参数收起时仍保留序号、名称和类型。

本次在 96 DPI 下通过 UI Automation 测量工具栏和列表边界：1280×900 时列表占右侧点位编辑区高度的 86.3%，900×640 时占 75.9%；紧凑窗口手动展开路线库后仍占 65.6%。比例为列表高度除以“工具栏＋列表”高度，不是整个客户区的比例。首次打开、重新打开以及调整窗口后的首动作应保持在视窗内。

实际原生窗口截图：[默认布局](assets/native-manager-actions.png)、[紧凑布局](assets/native-manager-actions-compact.png)、[紧凑窗口展开路线库](assets/native-manager-actions-library-open.png)。这些截图对应已验证的布局；交互回归结果另列。当前模型不支持图像输入，视觉验证以控件边界及真实交互为依据，未声称完成截图逐像素审查。

[动作交互回归脚本](../scripts/smoke-manager-actions.ps1) 使用唯一隔离目录、36 个混合动作及关闭自动执行服务的配置，覆盖固定工具栏、信息折叠、名称保存、动作排序、YAML 往返、重开恢复和最后一个动作删除后的键盘焦点；`-CaptureScreenshots` 可输出当前原生窗口截图：

```powershell
pwsh -NoProfile -File scripts\smoke-manager-actions.ps1 -Executable target\release\game-helper.exe -CaptureScreenshots
```

## 原生拖动动作

动作工具栏新增“原生拖动”（`drag_native`）。起点、终点沿用归一化坐标输入，默认从当前点位已有空间动作取坐标，无空间动作时使用窗口中心。参数“移动速度（0～100）”默认 2，0 为瞬移，数值越大越慢；表单提示其不是毫秒时长，并说明急停会在单次原生移动返回后停止后续动作。

回放临时使用 AHK Event 模式，按住左键后通过一次 `MouseMove` 移到终点，结束或异常时松开左键、恢复发送模式。原有“拖动”（`drag`）继续使用拖动时长和分步线性插值。两种动作都支持名称、排序、折叠、YAML 编辑与缓存保存，详见[路线格式](route-format.md#drag_native-原生拖动)。

两组隔离检查分别验证编辑器保存/重开和真实 AHK 回放。运行时会操作各自创建的测试窗口，应顺序执行：

```powershell
pwsh -NoProfile -File scripts\smoke-manager-actions.ps1 -Executable target\release\game-helper.exe
pwsh -NoProfile -File scripts\smoke-native-drag.ps1
```

AHK 检查使用独立空白目标窗口、临时路线和独立命名的 daemon 窗口，验证默认速度、瞬移与慢速、相同起终点、发送模式恢复和急停后跳过下一动作。它不读取用户路线；证据保存在输出所示的 `.tools/native-drag-*` 目录。

本次工作区 85 项测试通过，另有 1 个由父测试调用的私有子进程入口保持 ignored；编译检查、格式检查、Clippy 与 Release 构建通过，保留既有警告。AHK 2.0.30 的隔离回放通过：runner 的默认速度、速度 0、速度 40、相同起终点，以及 daemon 的 IPC 原生拖动均正确到达终点并释放左键。测试同时确认 Input/Play 发送模式恢复、提前停止模式恢复，以及左键按住期间触发急停后跳过下一动作。异常路径未人为触发，仅检查了嵌套 `finally` 清理代码；真实游戏、多 DPI 和多显示器未在本次回放中覆盖。

Release 管理器的动作交互检查也已通过：1280×900、900×640 及紧凑窗口展开路线库时，七类添加按钮均可见。新建原生拖动的速度默认 2，界面修改为 37 后，经保存、缓存生成、YAML 切换、切换点位、重新打开路线及重启管理器仍保持正确。

## 视觉与实现

| 用途 | 色值 |
|---|---|
| 工作台底色 | `#EDF2F4` |
| 编辑工作面 | `#FFFFFF` |
| 主文字 | `#173F4A` |
| 选择、主操作和焦点 | `#236B73` |
| 未保存提示 | `#AA650E` |
| 错误与删除 | `#B43E4B` |

中文正文使用 Microsoft YaHei UI；点位序号使用 Bahnschrift；源码使用 Cascadia Mono。点位的执行顺序是主要视觉元素。没有新增自动播放的装饰动画。

[manager.rs](../crates/game-helper-desktop/src/manager.rs) 保留文档、后台任务和动作分发，[view.rs](../crates/game-helper-desktop/src/manager/view.rs) 负责各个界面区域，[workspace_theme.rs](../crates/game-helper-desktop/src/workspace_theme.rs) 定义共享工作台主题。主题在管理器与独立取点模式初始化，probe 沿用原有主题。

搜索、折叠和确认焦点是内存中的界面状态，不标记文档为脏，不写配置。输入实体不会在渲染时重新创建。路线仍使用 `route/v1`，新生成的 YAML / 缓存移除点位层 `pos`，旧字段兼容读取并忽略；配置和 AHK IPC 保持不变。取点工具独立持久化采集位置，复制后可粘贴到动作坐标框；新增动作优先使用已有点击或拖动的坐标，无空间动作时默认窗口中心。

路线切换使用独立加载状态：旧文档保持显示，透明交互遮罩阻止加载期间的编辑，控件不切换全局禁用配色；成功后一次性替换工作区，失败保留旧内容。放弃后切换不再先显示欢迎页，点击当前路线不会重复加载。保存及服务操作仍沿用原有忙碌状态。

## 本次动作名称与紧凑布局验收

以下结果来自当前 Debug/Release 构建和 96 DPI 的原生 Release 窗口，所有交互均使用隔离数据，不请求游戏回放。

| 检查 | 结果 |
|---|---|
| 工作区测试 | 79 passed、0 failed；另有 1 个由父测试启动的私有子进程入口保持 ignored |
| Clippy、格式、diff 空白检查 | 通过；Clippy 保留既有文档与风格警告 |
| Debug / Release 构建 | 通过，统一启动入口优先使用本次 Release |
| 长列表添加及布局 | 36 个预置混合动作，滚至末尾仍可添加六类动作；新增后标题和名称框可见；默认、900×640 及紧凑展开路线库的工具栏控件均位于可见边界内 |
| 折叠与点位信息 | 展开/收起不写文件、不产生脏状态；信息展开后随内容滚走，收起不渲染表单；首次打开、调整窗口、切换路线及重启时首动作可见 |
| 名称与参数保留 | 中文、引号等名称及原参数经过折叠、上下移、删除、保存、YAML 往返、切换点位/路线及进程重启后保留；最终 41 个动作逐字段比对缓存通过 |
| 空列表及键盘焦点 | 删除唯一动作后，真实 Enter 可打开点位信息，确认焦点已恢复到可见按钮 |
| 既有控件回归 | 中文下拉所有选项、JSON 类型、动作坐标精度、起始点恢复/默认/保存/范围提示、切换路线的保存/放弃/取消及读取失败保留内容均通过 |

对应脚本为[动作交互检查](../scripts/smoke-manager-actions.ps1)与[原有控件回归](../scripts/smoke-manager-controls.ps1)。真实游戏回放、系统 IME 组合输入及多 DPI/多显示器尚未在本次验收中覆盖。

## 初版工作台验收（历史记录）

本轮在 Windows、96 DPI（100%）下运行，UI 检查均使用关闭自动启动的隔离数据目录。未请求生产路线回放。

| 检查 | 结果 |
|---|---|
| `cargo check --workspace --locked` | 通过 |
| `cargo test --workspace --locked` | 62 passed、0 failed；另有 1 个 ignored 子进程服务入口，由父测试显式启动 |
| `cargo clippy --workspace --all-targets --locked` | 通过；保留已有的文档与风格警告 |
| `cargo fmt --all -- --check` | 通过 |
| Debug / Release 构建 | 通过；原生入口优先使用已更新的 Release |
| 搜索与新建区 | 文件名 / 名称匹配、无结果、清空恢复、展开与收起通过 |
| 点位与窗口布局 | 首尾排序禁用、空动作、900×640 关键控件边界与路线库折叠 / 重开通过 |
| 中文编辑与保存 | 表单 / YAML 双向中文保留、坏源码切换失败后保留输入、显式保存与缓存检查通过 |
| 脏关闭 | 拦截、取消保留源码、再次关闭与显式放弃退出通过 |
| 额外边界 | 空路线库、零点位、坏文件错误反馈、120 个长名称点位及只查看不产生脏状态通过 |
| 键盘与可访问性 | 动作参数稳定 ID 与中文名称、源码编辑器名称、真实 Escape 取消离线执行确认通过 |

可重复运行的 UI 检查脚本：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\prepare-smoke.ps1
# 启动 target\debug\game-helper.exe --app-root "<项目>\.tools\acceptance sandbox"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\smoke-manager-readonly.ps1 -TargetProcessId <pid>
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\smoke-manager-layout.ps1 -TargetProcessId <pid>
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\smoke-manager.ps1 -TargetProcessId <pid>
```

最后一个脚本会修改隔离 Smoke 路线并关闭窗口。布局脚本会移动和调整指定窗口，并在 `.tools` 下保存截图。不要对生产数据窗口运行这些验收脚本。

## 本次界面精简回归

工作区 62 项测试通过，另有 1 个由父测试启动的子进程入口保持 ignored；覆盖无点位坐标的新格式、旧字段兼容读取、动作坐标保留和新增动作默认位置。编译、格式检查和 Clippy 通过，Clippy 保留已有文档及风格警告。默认及 900×640 布局检查、原有中文表单/YAML 编辑保存和脏关闭检查通过。

新增 [smoke-manager-controls.ps1](../scripts/smoke-manager-controls.ps1) 自建关闭自动服务的隔离根目录，覆盖中文下拉框缺省值、全部选项、键盘提交与 Escape 取消、动作排序、点位切换、YAML 往返及 JSON 类型；检查点位层坐标不再保存、动作坐标保留、起始点回显/默认值/保存/越界提示，以及切换路线时保存、放弃、取消和读取失败的内容保留。脚本不会请求试跑或热键回放：

```powershell
pwsh -File scripts\smoke-manager-controls.ps1
pwsh -File scripts\smoke-manager-controls.ps1 -Executable target\release\game-helper.exe
```

取点回归脚本 [smoke-picker.ps1](../scripts/smoke-picker.ps1) 自建隔离目录和原生 `--probe` 目标，覆盖失焦全局采点、坐标复制、自动保存、标题隔离、重启恢复与错误处理；无需先运行 `prepare-smoke.ps1`。[smoke-picker-coordinates.ps1](../scripts/smoke-picker-coordinates.ps1) 验证点击取点坐标复制后，用真实 Ctrl+V 粘贴至管理器的点击坐标框，保存路线/缓存并重启管理器后坐标保留。均不请求回放，可传 `-Executable target\release\game-helper.exe` 复验。

## 实际原生窗口截图

以下为初版工作台的历史截图；本次精简后的控件和标题以当前实现及上述回归脚本为准。

展示路线为现有路线的隔离副本，仅在副本中设置了说明性名称。截图来自 Release 原生窗口，不是 HTML 原型。

![默认管理器](assets/native-manager.png)

[紧凑窗口](assets/native-manager-compact.png) · [YAML 编辑](assets/native-manager-yaml.png) · [空路线库](assets/native-manager-empty.png)

本轮使用 Windows 可访问性树与控件坐标检查可见区域和交互。当前模型的图片读取通道不支持图像输入，未宣称已完成截图的逐像素目视验收。150% / 200% DPI、系统 IME 组合输入和真实 AHK 完整回放仍需继续按[迁移记录](rust-migration.md)验收。
