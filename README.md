# game_helper

基于 **Rust + GPUI Kit** 的原生 Windows 路线管理器和取点工具，使用 **AutoHotkey v2** 回放路线。项目仅维护 Rust 客户端。

## 启动

```bat
rem 打开管理器
start.bat

rem 打开独立取点工具
start.bat --picker
```

`start.bat` 是统一启动入口，优先使用 `target/release/game-helper.exe`，其次使用 `target/debug/game-helper.exe`；缺少产物时会提示构建命令。

运行客户端无需安装 Rust、WebView 或 AutoHotkey。项目与便携包内置普通 x64 AutoHotkey v2.0.31；服务优先使用应用目录下的 `ahk/runtime/AutoHotkey64.exe`，缺少内置解释器时才回退到 `config.json` 的 `ahkPath` 和系统安装探测。Windows 还需要 Microsoft Visual C++ x64 运行库（`VCRUNTIME140.dll`）。

## 从源码构建

需要 Windows 10+、Visual Studio 2022 C++ 桌面工具与 Windows SDK。项目固定 Rust 1.99.1 MSVC 和 GPUI Kit 0.9.0，使用项目内工具链与编译缓存。

```bat
scripts\cargo.cmd build -p game-helper-desktop --release --locked
scripts\cargo.cmd test --workspace --locked
scripts\cargo.cmd fmt --all -- --check
```

便携打包：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\bundle.ps1
```

发布目录和 zip 输出到 `dist/`，包含 AHK 解释器、完整许可证、对应源码归档与校验信息；请携带整个目录，不能只复制管理器 exe。新包默认随管理器启动执行服务，默认不包含用户路线与热键绑定；需要携带时传入 `-IncludeUserData`，保留用户自启设置并移除构建机的 `ahkPath`。完整构建、进程约定和验证记录见[原生客户端说明](docs/rust-migration.md)。

## 功能与使用

- **路线管理**：新建、搜索、导入导出、删除，以及点位与操作编辑；支持表单和 YAML 源码双视图、未保存修改保护、磁盘冲突检测和原文备份。
- **独立取点**：窗口库搜索或延时准星选窗，分栏查看屏幕、客户区及归一化坐标，通过全局 `Ctrl+Alt+P` 或可取消的延时按钮采点。点位按进程名、窗口类和完整标题分别自动保存；支持上移、下移调整顺序，重启后恢复；点击坐标或复制按钮即可粘贴到路线管理的坐标框。见[取点工具说明](docs/picker-design.md)。
- **七类动作编排**：点击、按下/松开、按住、滚轮、拖动、等待和原生拖动；每个动作可填写名称并展开/收起。拖动动作左侧手柄即可排序，点击手柄可输入目标序号，也支持手柄聚焦后 Alt+↑/↓。顶部“添加动作”追加到末尾，列表开头可直接插入，每条动作的“在下方添加”可在原位置之后选择类型插入。新增动作自动展开并定位。点位支持复制到原点位下方，再独立修改动作。
- **编辑状态记忆**：每条动作的展开/收起状态自动保存到独立的 `state/manager-ui.json`，切换点位、路线或重启后恢复；不会写入路线、导出内容或执行缓存，也不要求先保存路线。点位名称与备注仍通过“点位信息”按需展开。路线格式见[route/v1 规范](docs/route-format.md)。
- **路线快捷键**：保存路线后点击“设置快捷键”，在监听弹窗直接按下按键或组合键，捕获后自动保存：新绑定默认关闭，修改已有绑定保留原开关状态，已开启时自动关闭其他同键路线；清除按钮解除绑定。路线列表每项右侧的 Switch 控制该快捷键是否有效，状态随配置保存并在重启后恢复。同一快捷键可保存给多条路线，开启其中一条时其他同键路线自动关闭；关闭后不会自动恢复旧路线。旧配置中缺少 `enabled` 的已有绑定默认开启。
- **路线执行**：热键单步执行与“试跑选中点”均通过执行服务完成；服务未启动时试跑按钮禁用，先在服务控制中启动。快捷键关闭不影响服务就绪时的按钮试跑。“保存起始点”独立保存单步起始位置；管理器提供服务启停、重载和单步进度。点击动作的鼠标按键使用“左键 / 右键 / 中键”下拉选择，按下/松开动作使用“按下 / 松开”下拉选择。
- **急停**：默认全局 `F12`，实际按键由 `config.json` 的 `hotkeys.stopAll` 决定。

热键绑定使用 daemon 单步模式：每按一次执行一个点位，最后一个点成功后回到设定的起始点循环；执行失败或急停时游标不前进。保存、试跑选中点或开启路线 Switch 都会生成路线 JSON 缓存；已有 YAML 路线设置快捷键后，开启 Switch 时会校验已保存路线并生成缓存。点位列表和编辑区均不显示点位坐标；新建或结构化保存的 YAML 与执行缓存不再包含点位层 `pos`，坐标仅存于点击、拖动动作中。取点工具独立保存采集坐标，复制后由管理器写入所需动作。旧路线的点位层 `pos` 兼容读取并忽略，动作坐标保持有效；源码模式仍保留用户原文。

管理器采用路线库、点位序列和动作编辑分栏的浅粉紧凑工作台，支持路线搜索和单行执行栏；管理器与取点工具共享以 `#FBA1DB` 为核心的主题。见[界面设计与实际截图](docs/manager-design.md)。

```text
Rust 管理器
    │ 校验并保存 YAML，生成执行缓存
    ▼
routes/_cache/*.json ──预载──▶ AHK daemon
                                 ▲
                                 │ Win32 IPC：Ping / Reload / Stop / Run
                                 │
                            Rust 管理器
```

## 目录

```text
crates/
  game-helper-core/       路线、配置、校验、文档与文件存储
  game-helper-windows/    窗口、DPI、全局热键、AHK 进程与 IPC
  game-helper-desktop/    GPUI Kit 管理器、独立取点工具与原生 exe
ahk/                     daemon / JSON / 执行核心与内置 runtime
scripts/                 Rust 构建、便携打包和隔离 UI 检查
routes/                  路线 YAML、_cache/ 执行缓存与 .backups/ 备份
docs/                    路线规范、原生客户端说明和界面设计
logs/                    执行日志
state/                   daemon 单步游标、取点 picker.json 与管理器 manager-ui.json
config.json              AHK 路径、热键绑定与 daemon 设置
start.bat                默认 Rust 启动入口，可传 --picker
```

Rust 单元和契约测试位于各 crate 的 `src/`、`tests/` 中。

## 运行约定与验证边界

- 若目标程序以管理员身份运行，管理器及 AHK 执行进程也需以相应权限运行，才能发送输入。
- 取点和执行使用窗口客户区归一化坐标，物理坐标换算采用 `(width-1)/(height-1)`。
- 源码保存保留已验证原文；结构化编辑会规范化 YAML，可能改变注释与排版，保存前会提示并留备份。
- 关闭管理器会保留执行服务；停止服务会结束服务进程。所有回放都由服务完成，全局急停键用于中断回放。
- `logs/run_*.log` 记录动作和延迟；路线执行异常时可据此排查。

当前客户端仍处于原生预览阶段。真实 AHK 六类操作完整闭环、系统 IME、多 DPI 与多显示器场景仍有待验收；默认入口切换不代表这些验收已完成。详见[待完成事项](docs/rust-migration.md#仍待完成的验收)。
