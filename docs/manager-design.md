# 路线管理器界面重设计

Rust + GPUI Kit 0.9.0 原生管理器的浅色路线编排工作台。通过 `start.bat` 统一启动 Rust 客户端；Python 客户端与回退入口已删除。现有 AHK v2 daemon/runner 继续作为执行后端。

## 界面结构

- **路线库**：名称和文件名搜索，两行路线摘要、点数与绑定热键，明确的当前选择。新建区按需展开；新建仍是内存草稿，显式保存后才写入文件。坏文件可打开修复。
- **路线工作区**：原生窗口标题为“路线管理”，顶部不再显示品牌或“路线工作台”标题，仅保留路线库、服务状态与工具控件。工作区提供路线标题与保存状态、保存按钮、点位编辑 / YAML 切换。路线属性与文件操作分别展开，导出、定位文件和删除保持原有动作。
- **点位序列**：序号表示真实执行顺序，连接线展示点位关系；列表和当前点位编辑区独立滚动。首尾排序按钮禁用，零点位与零动作有引导。列表仅显示点位名称、序号及动作数量；编辑区提供名称、备注和动作，均不显示点位坐标。新生成 YAML / 缓存不保存点位层 `pos`，旧字段兼容读取并忽略，点击与拖动动作自身的坐标仍可编辑。
- **动作编辑**：点击、按下 / 松开、按住、滚轮、拖动、等待使用中文名称；参数标注单位与输入格式。点击的鼠标按键使用“左键 / 右键 / 中键”下拉框，按下 / 松开的按键动作使用“按下 / 松开”下拉框；保存时仍为 `left/right/middle` 字符串和 `up` 布尔值。参数控件提供包括动作序号的中文可访问名称和稳定 ID。
- **固定执行栏**：起始点、选中点试跑、热键绑定、单步进度及真实急停键；已移除管理器的“执行路线”按钮和整条路线连续执行入口。切换路线时起始点读取该路线绑定值，无绑定时显示 1。执行服务和独立单点试跑进程分别提示状态；受理请求不表示试跑完成。
- **确认区域**：保存 / 放弃 / 取消等原有分支使用覆盖式确认区域；焦点进入取消按钮，Escape 可取消。确认层在编辑器与调整面板之上处理点击。

布局设计基准为 1280×900；小于 1100px 时路线库默认折叠，可通过顶栏重新展开。用户手动选择的展开状态在本次会话中保留。最小客户区为 900×640，原生标题栏与边框另计；本机紧凑截图为 916×679。

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

## 本轮验证

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
