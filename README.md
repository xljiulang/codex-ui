# codex-ui

基于 **Rust + Tauri 2** 的 Codex CLI Windows 桌面客户端。`codex-ui.exe` 是对 `codex` CLI 的桌面包装：启动后拉起 `codex app-server --stdio`，用与 VS Code Codex 插件相同的 JSON-RPC 协议驱动 codex，提供完整的中文桌面界面。

> `codex-ui.exe` 启动时所在目录即 codex 的工作目录；历史会话按目录记录，点击任一历史会话会使用**该会话记录的工作目录**继续执行。

## 功能

- **聊天流**：流式 Markdown（表格、任务列表、代码块复制按钮、链接悬停显示完整 URL、图片、ANSI 彩色输出）、思考过程（流式展开、完成折叠、耗时显示）、进行中阶段徽标与记忆引用、计划/待办、上下文压缩提示；回合进行中顶部流光进度线 + “思考中”发光提示。
- **滚动体验**：流式输出时上滑查看历史不会被拉回底部，右上角“回到底部”按钮一键恢复跟随。
- **工具卡片**：命令执行显示真实命令、工作目录、实时输出、**实时耗时**、退出码；文件变更带 diff；网络搜索（结构化结果列表）、MCP/动态工具、子代理协作。
- **交互弹窗**：提权审批（批准 / 拒绝 / 本次会话批准 / 批准并记住此规则）、选项提问、MCP 表单、**计划已就绪确认**（仿 VS Code/CLI：计划模式回合完成后弹出“执行计划 / 待在计划 / 退出计划模式”，执行计划自动发送 `PLEASE IMPLEMENT THIS PLAN:` 消息并切到执行模式）；弹窗出现时播放提示音并置前窗口；回合可随时停止并提示“已停止生成”；自定义右键菜单（复制/粘贴/打开链接，无默认“刷新”）。
- **历史会话**：面板**常驻右侧且宽度可拖拽调节（264px ~ 窗口一半）**，展示全部目录的会话并**按目录分组为文件夹（目录按名称 A-Z，会话按置顶+最近时间倒序；默认收起，单条会话不建夹、不显示路径）**；**全量加载**、重命名、固定置顶（行首显示钉子图标）、搜索（带摘要，搜索框右侧刷新按钮）、删除确认，会话行**右键菜单（加载/重命名/置顶固定、取消固定/删除会话）**，目录（文件夹）行右键可**新建会话（预置该分组目录）**或**在资源管理器中打开**；新建会话后自动聚焦输入框；打开为只读，发消息才恢复；点击会话不自动关闭、点击当前会话不重载；继续执行使用会话记录的工作目录，历史中的命令/工具详情完整加载。
- **会话资源**：右侧面板底部 Tab（历史会话 / 会话资源）；以**当前会话工作目录**为根的文件树，**懒加载、默认只展开第一层**，名称以 `.` 开头的目录与 `node_modules` 无条件隐藏，**根节点悬停显示完整路径**；**文件行显示 Windows 系统文件类型图标**（懒加载 + 按扩展名缓存，失败回退内置图标）；头部搜索框递归搜索文件/目录名；**文件变化自动刷新**（`notify` 监听、300ms 防抖）；文件单击或右键「打开」**按类型分发**：文本文件在左侧标签页以文本编辑器打开（语法高亮、可编辑），`.pdf` 与图像文件（png/jpg/jpeg/gif/webp/bmp/svg/ico/avif）打开只读预览标签（PDF 用 pdf.js 渲染、支持翻页缩放；图像经 asset 协议直显），其余非文本提示无法打开；文本编辑器内置**自定义右键菜单**（撤销/重做/剪切/复制/粘贴/全选/查找替换，右键定位光标），`.md`/`.markdown` 文件支持**「预览/编辑」切换**；右键菜单管理（打开/复制/粘贴、重命名、删除、属性、在资源管理器中打开、添加为会话附件），支持从系统剪贴板粘贴文件/文件夹，属性弹窗显示大小与时间；切换 Tab 保留展开状态。
- **左侧标签页（多标签编辑区）**：主窗口左侧为标签页编辑区，第一个固定「会话」主标签（不可关闭），其后可打开文件（CodeMirror 编辑）、diff、PDF/图像预览与 **PowerShell 终端**标签；标签 `v-show` 常驻挂载，切换标签不销毁编辑状态/进程；**标签右键菜单**统一为「关闭所有标签 / 关闭左边所有标签 / 关闭右边所有标签」——未保存（脏）文件跳过并提示数量、终端批量关闭时结束进程，会话标签固定在首位且不可关闭（左边项恒隐藏）。
- **终端标签**：资源管理器「在此打开终端」在标签区启动 `powershell.exe`（Windows ConPTY，UTF-8 编码），支持同目录多开、切走不中断；`useTerminalEvents` 事件桥在 spawn 前注册全局监听并按 id 缓冲启动输出（含 ConPTY 光标位置查询，xterm 自动应答），面板挂载后回放，修复首个终端空白；spawn 完成后自动补一次 fit/resize。
- **Git 更改**：右侧面板底部 Tab（历史会话 / 会话资源 / **Git 更改**），后端内置 **gix**（纯 Rust，状态/提交/分支/合并等零系统 git 依赖）；以**当前会话工作目录**为根检测所在仓库，列出**已更改文件**（新增/修改/删除/未跟踪圆形状态图标，悬停提示状态文字 + 分支名；`node_modules` 无条件忽略），点击文件在主窗口左侧标签页打开 diff 标签查看（统一对比 HEAD，暂存 + 未暂存合并；未跟踪文件以空内容对比）；**分支管理**：点击分支名弹出下拉弹层，支持**切换 / 新建 / 删除本地分支**（切换由 gix 进程内完成：同步工作区文件并重建索引，有已跟踪改动或会覆盖未跟踪文件时拒绝；删除禁止删除当前分支）；**提交历史**（最近 50 条，显示主题/作者/时间/7 位短哈希，底部「…」无边框图标按钮逐批加载更早提交，状态变化自动回到最新一页）；**变更自动刷新**（监听仓库目录与 `.git`，300ms 防抖，`.gitignore` 规则参与事件过滤）；**拉取/推送**：拉取（快进优先 `--ff-only`，分叉时提示先手动合并，绝不留下冲突状态）与推送（首次自动 `-u` 设置上游）均调用系统 git，未检测到系统 git 时对应按钮禁用并提示安装，非快进/认证失败给出中文提示；目录不在仓库时提供**“添加到 Git”一键初始化**（仅 `git init`，不自动提交）。
- **标题自动总结**：新会话首条消息（去掉文件引用后 > 15 字）发出后，后台用临时线程 + 默认模型生成短标题并写回（仿 VS Code）；与主回合并行、失败静默保留原标题；支持 `experimentalApi` 才执行，旧版协议自动回退/跳过。
- **权限模式**：请求批准 / 帮我批准 / 完全访问权限（对话进行中按钮禁用）。
- **任务模式**：执行 / 计划 / 目标（目标对话框）；计划模式回合产出计划后弹出“计划已就绪”确认（纯前端 UX，与 VS Code/CLI 一致）。
- **模型与上下文**：模型下拉选择（`model/list`）、推理强度跟随模型动态生成（如 `low/high/max`）、模型按钮显示“名称(强度)”、上下文窗口百分比椭圆框（悬停显示“上下文已用 xx K，共 yy K”）。
- **输入区**：通栏输入框 + 悬浮靠底按钮；`+` 菜单、`@`/`$` 菜单；**上下键选择输入历史**（内存）；**Ctrl+V 粘贴 / 文件拖放添加附件**（截图/图片落盘到临时目录，复制文件用原始路径）；**输入框高度可拖拽调节**（96px ~ 窗口一半）；本地图片附件在消息中直接渲染（输入区带缩略图）；发送/停止二合一按钮（带文字）；**新建会话前可选项目目录**。`@` 文件引用以 VS Code 扩展同款 `# Files mentioned by the user:` 文本段发送，`$` 技能以 `[$name](path)` 文本链接 + 结构化技能项发送（服务端注入技能内容），回显时渲染为 `@名称` / `$名称` 引用标签。
- **头部**：六边形 Logo 在左；右侧是“新建会话（与设置同形态的 32px 圆角方形图标按钮，加号 18px 加粗等比放大的强调色以区分主次；点击先弹文件夹选择器选定本次会话工作目录，取消则沿用默认目录，选后仍新建并聚焦输入框）+ 设置（中性灰 16px 图标按钮）”；会话进行中切换会话会弹出确认。
- **窗口标题**：主窗体标题跟随工作目录与会话：启动/新建会话显示工作目录文件夹名，产生对话标题后显示 `文件夹名 - 对话标题`，新建会话选中目录后标题立即跟随。
- **设置**：模态对话框（“保存”/“取消保存”，保存后关闭）；**三套主题（蓝夜/曜黑/晨光）点击卡片即时预览、保存后生效**；codex 路径、Enter 快捷发送、跟进处理方式（调整方向 / 加入队列）、提示音开关（权限模式、模型、推理强度为进程级，在输入区按钮菜单中配置）。

## 架构

```
codex-ui.exe（Tauri 2 窗口）
├── Rust 后端  src-tauri/src/codex/
│   ├── app_server.rs  codex.exe app-server --stdio（换行分隔 JSON-RPC）
│   ├── commands.rs    Tauri 命令层
│   ├── session_fs.rs  会话资源文件系统命令 + notify 文件监听
│   ├── terminal.rs    ConPTY 终端会话（portable-pty 启动 PowerShell，输出事件流/输入/缩放/kill，支持多开）
│   ├── diff.rs        diff 预览行构建
│   ├── path_util.rs   路径清洗（\\?\ 前缀剥离）
│   ├── file_icon.rs   系统文件类型图标提取（SHGetFileInfo → PNG data URI）
│   ├── git.rs         Git 更改：状态/diff/初始化/分支/合并（gix 进程内），拉取/推送调用系统 git
│   └── settings.rs    设置读写
└── Vue 3 前端  src/
    ├── composables/useCodex.ts  状态 + 事件订阅 + 协议调用
    ├── composables/useEditorTabs.ts  左侧多标签状态（文件/diff/预览/终端标签打开、关闭与批量关闭）
    ├── composables/useTerminalEvents.ts  终端事件桥（spawn 前监听 + 按 id 缓冲/回放）
    ├── composables/useSessionFs.ts  会话资源状态（懒加载/搜索/监听/图标缓存）
    ├── composables/useGitChanges.ts  Git 更改状态（检测/刷新/初始化/监听）
    └── components/              界面组件（RightPanel + HistoryView + ResourceView、EditorPane 多标签编辑区：TextEditorPane/DiffPane/PreviewPane/TerminalPane 等）
```

后端负责 codex 子进程生命周期（断线自动重连）、JSON-RPC 请求/响应分发、服务端通知流式转发（`item/started`、`item/completed`、`item/agentMessage/delta`、`item/commandExecution/outputDelta`、`interaction:request` 等）与交互响应回写。前端通过 Tauri invoke/event 通信。

## 环境要求

- Windows 10/11（自带 WebView2）
- Rust stable-msvc（rustup）+ VS C++ 构建工具
- Node.js 18+
- `codex` CLI（可在设置页指定路径；验证基线 0.146.0-alpha.9.2）
- 拉取/推送功能需要系统已安装 **Git**（其余功能零 git 依赖；未安装时对应按钮禁用并提示安装）

## 开发

```powershell
npm install
npm run tauri dev        # Vite 热更新 + Tauri
```

## 构建

```powershell
npm install
npm run build            # 构建前端到 dist/
cargo build --release --manifest-path src-tauri\Cargo.toml
# 产物：src-tauri\target\release\codex-ui.exe
```

安装包（NSIS/MSI）：

```powershell
npm run tauri build
```

> 注意：`Cargo.toml` 中 `tauri` 依赖已启用 `custom-protocol` 与 `protocol-asset` 特性。前者保证生产窗口加载打包的前端（否则会去连 `localhost:5173` 显示“拒绝连接”），后者用于 asset 协议加载本地图片。

## 测试

```powershell
npm test                                   # 前端单元测试
cargo test --manifest-path src-tauri\Cargo.toml   # Rust 集成测试（真实 codex app-server 全流程）
```

端到端探针脚本（`scripts/`）：`verify-approval.mjs`（“批准并记住此规则”）、`check-mode-lock.mjs`（回合中模式按钮禁用）、`probe-approval*.mjs`。

## 使用说明

- **工作目录**：从哪个目录启动 codex-ui，新建会话默认归到该目录；点击头部「新建会话」会先弹出文件夹选择器，可选本次会话的工作目录（取消则沿用默认）。历史会话显示各自记录的目录，继续执行用记录目录。
- **历史会话**：面板顶部即搜索框；按目录分组（目录按名称 A-Z、会话按置顶+时间倒序），文件夹默认收起，支持全量加载、重命名、固定置顶、删除确认。点击仅查看（点击当前会话不重载），输入消息后才会恢复并执行；新会话首条消息会自动生成短标题。带**活跃目标**的会话恢复后会按目标模式自动持续执行——切出目标模式会自动清除目标。
- **会话资源**：右侧面板底部切换到“会话资源”Tab；根为当前会话工作目录（无会话时用新会话已选目录，再兜底启动目录），根节点显示文件夹名、默认展开第一层；`.git`/`.codegraph` 等点目录不显示；文件/目录右键管理，文件变化自动刷新；切换会话后根随之切换。
- **权限模式**：请求批准会询问所有外部写操作；“批准并记住此规则”会把规则写入 codex 策略。完全访问不受限。权限/模型/推理强度为**进程级**，重启后恢复默认。
- **设置文件**：`%APPDATA%\com.codexui.app\settings.json`，仅保存 codex 路径、Enter 快捷发送、跟进处理方式、提示音开关、主题与权限模式初始值。
- **登录**：界面不提供登录入口，请使用其它入口（如 `codex login` 或 API Key）完成认证。

## 协议兼容性

以 `codex app-server generate-ts --experimental` 输出的协议绑定为参考（验证基线 0.146.0-alpha.9.2）。后端只实现本项目所需字段，未知通知忽略并记录日志。codex CLI 升级后如协议变化，可重新生成绑定核对：

```powershell
codex app-server generate-ts --out <dir> --experimental
```

**置顶协议随 codex 版本变化**，应用启动后首次点击置顶时自动探测并选择对应协议：

| codex 时代 | 版本示例 | 使用的协议 |
| --- | --- | --- |
| `isPinned` 元数据 | 0.146.0-alpha.9.2 | `thread/metadata/update { isPinned }` |
| 分区 + `sectionId` | 0.147.0-alpha.1.2 | `thread/metadata/update { sectionId }` |
| 分区 + `threadSection/move` | v0.147.0 及更新 | `threadSection/move { sectionId }` |

探测逻辑（`codex_pin_capability`）：先试 `threadSection/list`，存在分区则继续试 `threadSection/move`（方法不存在则回退 `metadata/update { sectionId }`）；无分区接口则试 `metadata/update { isPinned }`，字段不被认（报 "must include at least one field"）时视为不支持置顶。探测仅读取能力、不修改任何会话状态。

**标题自动总结**依赖临时线程协议，启动后由 `codex_title_helper_capability` 只读探测（创建内存线程后立即释放）：支持 `experimentalApi` + `ephemeral` 时用内存临时线程；支持 `experimentalApi` 但不支持 `ephemeral` 时用普通线程总结后删除；不支持 `experimentalApi` 时跳过标题总结。

## 常见问题

- **启动后“localhost 拒绝连接”**：生产构建缺少 `tauri/custom-protocol` 特性（或前端未构建）。确认 `Cargo.toml` 已启用该特性并重新 `cargo build --release`。
- **历史显示全部而非当前目录**：当前版本按需求设计为展示全部会话并标注目录；如仍异常，确认运行的是最新构建（旧版存在初始化时序 bug）。
- **点击历史后发送/停止按钮反复交替**：该会话带活跃目标且被自动恢复。新版打开会话为只读、不会触发；如已发生，清掉该会话目标或删除会话。
- **“批准并记住此规则”后仍被拒**：确认权限模式为“请求批准/帮我批准”（沙箱为 workspace-write）；早期版本误用 read-only 沙箱导致该问题。
- **本地图片不显示**：确认构建启用了 `protocol-asset` 特性且 `tauri.conf.json` 配置了 `assetProtocol`。
- **继续历史会话报 `invalid_request_error: you passed .`**：旧版会向服务端发送空模型；更新到最新版本（修复后自动回退到默认模型）。
- **拉取/推送提示“未检测到系统 git”**：拉取与推送依赖系统 Git；请安装 Git（git-scm.com）或将其加入 PATH，安装后重启应用。其余功能（状态/提交/分支合并等）不需要系统 git。
- **拉取提示“本地与远端已分叉”**：拉取采用快进优先（`--ff-only`），分叉时不自动合并；请先提交本地改动，再在 GitView 分支管理中合并远端分支后重新拉取。
- **右键菜单里的“刷新”**：已用自定义右键菜单取代默认菜单，页面不会被意外重载。

## 目录结构

```
src/            Vue 3 前端（组件、状态、样式、测试）
src-tauri/src/  Rust 后端（协议层、命令层、设置）
src-tauri/tests/ 集成测试
scripts/        端到端探针
docs/           计划、素材说明与素材图片
```

## 文档

- [实施计划与实现说明](docs/计划.md)
- [变更记录](docs/变更记录.md)
- [素材与参考说明](docs/素材说明.md)
- [引用与附件协议说明（@ / $ / 图片）](docs/引用与附件.md)
