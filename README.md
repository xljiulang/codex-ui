# codex-ui

基于 **Rust + Tauri 2** 的 Codex CLI Windows 桌面客户端。`codex-ui.exe` 是对 `codex` CLI 的桌面包装：启动后拉起 `codex app-server --stdio`，用与 VS Code Codex 插件相同的 JSON-RPC 协议驱动 codex，提供完整的中文桌面界面。

> `codex-ui.exe` 启动时所在目录即 codex 的工作目录；历史会话按目录记录，点击任一历史会话会使用**该会话记录的工作目录**继续执行。

## 功能

- **聊天流**：流式 Markdown（表格、任务列表、代码块复制按钮、链接悬停显示完整 URL、图片、ANSI 彩色输出）、思考过程（流式展开、完成折叠、耗时显示）、进行中阶段徽标与记忆引用、计划/待办、上下文压缩提示；对话正文中的文件链接点击在应用内打开（按类型进文本编辑/预览标签，不支持的格式降级为资源管理器定位），`@` 文件引用点击在资源管理器中定位；回合进行中顶部流光进度线 + “思考中”发光提示。
- **滚动体验**：流式输出时上滑查看历史不会被拉回底部，右上角“回到底部”按钮一键恢复跟随。
- **工具卡片**：命令执行显示真实命令、工作目录、实时输出、**实时耗时**、退出码；文件变更带 diff；网络搜索（结构化结果列表）、MCP/动态工具、子代理协作。
- **交互提示**：提权审批（批准 / 拒绝 / 本次会话批准 / 批准并记住此规则）、选项提问、MCP 表单**内嵌在聊天消息流中**（待处理交互气泡，回答时上方上下文完整可见；交互挂起时隐藏“思考中”；CODEX 会话标签显示待处理计数角标，切到文件/diff 标签也不会错过）；**计划已就绪确认**同为消息流内嵌气泡（仿 VS Code/CLI：计划模式回合完成后在消息流末尾弹出“执行计划 / 待在计划 / 退出计划模式”，完整计划见上方消息、气泡不重复展示，执行计划自动发送 `PLEASE IMPLEMENT THIS PLAN:` 消息并切到执行模式）；交互出现时播放提示音；回合可随时停止并提示“已停止生成”；自定义右键菜单（复制/粘贴/打开链接，无默认“刷新”）。
- **历史会话**：面板**常驻右侧且宽度可拖拽调节（300px ~ 窗口一半）**，展示全部目录的会话并**按目录分组为文件夹（目录按名称 A-Z，会话按置顶+最近时间倒序；首个目录默认展开、其余默认收起，单条会话不建夹、不显示路径）**；**全量加载**、重命名、固定置顶（行首显示钉子图标）、搜索（带摘要，搜索框右侧刷新按钮）、删除确认；会话行右键菜单**未打开时为「打开 / 重命名 / 置顶固定 / 删除会话」，已打开（有会话标签）时为「重命名 / 置顶固定」**（已打开会话行显示「已打开」标记，后台运行中显示呼吸点）；目录（文件夹）行右键可**新建会话（预置该分组目录）**、**在此打开终端（以分组目录启动）**、**在资源管理器中打开**、**删除所有会话（组内全部已打开时隐藏该项；确认后仅删除未打开的会话，已打开跳过并提示数量）**；新建或打开会话后聚焦输入框，右侧面板保持当前 Tab、不中断后台回合；打开为只读，发消息才恢复；点击会话行：未打开则新标签打开、已打开则聚焦对应标签（点击当前会话不重载）；继续执行使用会话记录的工作目录，历史中的命令/工具详情完整加载。
- **会话资源**：右侧面板底部 Tab（会话 / 资源 / Git）；以**活动标签的工作区**为根的文件树（会话标签→会话工作目录；文件/diff/预览/终端标签→各自打开时的工作区），**懒加载、默认只展开第一层**，名称以 `.` 开头的目录与 `node_modules` 无条件隐藏，**根节点悬停显示完整路径**；**文件行显示 Windows 系统文件类型图标**（懒加载 + 按扩展名缓存，失败回退内置图标）；头部搜索框递归搜索文件/目录名；**文件变化自动刷新**（`notify` 监听、300ms 防抖）；文件单击或右键「打开」**按类型分发**（已打开的文件右键菜单不显示「打开」，单击仍激活对应标签）：文本文件在左侧标签页以文本编辑器打开（语法高亮、可编辑），`.pdf` 与图像文件（png/jpg/jpeg/gif/webp/bmp/svg/ico/avif）打开只读预览标签（PDF 用 pdf.js 渲染、支持翻页缩放；图像经 asset 协议直显），其余非文本提示无法打开；文本编辑器内置**自定义右键菜单**（撤销/重做/剪切/复制/粘贴/全选/查找替换、**代码格式化**，右键定位光标），「代码格式化」覆盖全部已识别语言：JS/TS/JSON/JSONC/CSS/HTML/YAML/Markdown 走 Prettier 完整美化（可展开单行），XML/SVG 走自研语法树美化，Python/Go/Rust/C++/Java 等其余语言复用 CodeMirror 缩进规则重排（仅修正缩进）；格式化单次事务可撤销，语法错误 toast 提示；`.md`/`.markdown` 文件支持**「预览/编辑」切换**；**文件/目录行悬停显示「@」按钮，可一键添加为会话附件**；**文件/目录可拖拽到目录或根节点完成移动**（同卷 rename、跨盘自动降级复制+删除，禁止移入自身/子目录）；右键菜单管理（**新建文本文件/新建文件夹**、打开/复制/粘贴（剪贴板无文件时隐藏「粘贴」）、重命名、删除、属性、在资源管理器中打开、添加为会话附件），新建文本文件/新建文件夹后自动进入行内重命名；支持从系统剪贴板粘贴文件/文件夹，属性弹窗显示大小与时间；切换 Tab 保留展开状态。
- **左侧标签页（多标签编辑区）**：主窗口左侧为标签页编辑区，**多个会话标签（可关闭、允许 0 个）**——会话标签标题为 `工作区目录名 / 会话标题`（新对话为 `… / 新建会话`），带进行中呼吸圆点与待处理交互计数角标；标签栏常驻显示、末尾「+」可快速新建空会话标签；允许关闭到 0 个会话标签（主区域显示“当前还没有任何打开的项”空状态）；其后可打开文件（CodeMirror 编辑）、diff、PDF/图像预览与 **PowerShell 终端**标签；标签 `v-show` 常驻挂载，切换标签不销毁编辑状态/进程；会话标签右键「关闭所有标签 / 关闭其它会话标签」，文件/diff/预览/终端标签右键「关闭所有标签 / 关闭左边 / 关闭右边（文件类另有「在资源管理器中打开」）」——未保存（脏）文件与**运行中的终端/会话**跳过并提示数量，空闲终端批量关闭时结束进程；关闭运行中的会话标签或终端需确认。
- **终端标签**：资源管理器文件夹与历史会话分组的「在此打开终端」在标签区启动 `powershell.exe`（Windows ConPTY，UTF-8 编码），支持同目录多开、切走不中断，标签标题固定为 **PowerShell**（无 ToolTip）；`useTerminalEvents` 事件桥在 spawn 前注册全局监听并按 id 缓冲启动输出（含 ConPTY 光标位置查询，xterm 自动应答），面板挂载后回放，修复首个终端空白；spawn 完成后自动补一次 fit/resize；标签激活时自动聚焦 xterm（点击或切换回终端标签即可直接输入）；**命令执行中标签显示呼吸圆点**（后端每次回到提示符注入隐藏标记 OSC 133;D + 前端回车/标记判定，回到提示符自动熄灭）；`Invoke-WebRequest`/`wget` 默认 `-UseBasicParsing`（2025-12 安全更新 CVE-2025-54100 新增的“脚本执行风险”确认不再弹出）；**关闭运行中的终端需确认**（终止并关闭/取消），批量关闭跳过运行中的终端并提示数量；终端配色深度适配三套主题（完整 16 色 ANSI + 光标前景，浅色主题下输入回显、提示符与彩色输出清晰可读），切换主题即时生效。
- **Git 更改**：右侧面板底部 Tab（会话 / 资源 / **Git**），后端全部功能调用系统 **git.exe** 子进程（隐藏控制台窗口；启动时执行一次 `git --version` 探测并缓存，未安装时整个会话报“未检测到系统 git”）；以**活动标签的工作区**为根检测所在仓库，列出**已更改文件**（行首文件类型图标 + 行末状态字母徽标 A/M/D/R/U/C（绿/黄/红配色），悬停提示状态文字 + 分支名；`node_modules` 无条件忽略），点击文件在主窗口左侧标签页打开 diff 标签查看（统一对比 HEAD，暂存 + 未暂存合并；未跟踪文件以空内容对比；**完整/简要切换**）；**分支管理**：点击分支名弹出「本地 / 远程 / 远端管理」三分区下拉弹层——本地：**切换 / 新建 / 删除 / 合并**（`git switch` 标准语义：无冲突即可切换且未提交改动随分支携带，本地改动/未跟踪文件将被覆盖时拒绝；删除用 `git branch -D` 无条件删除（不校验是否合并），禁止删除当前分支）；远程：列出远端跟踪分支、**拉取刷新**、检出为本地跟踪分支、删除远程分支（当前上游时二次确认）；远端管理：**添加 / 删除远端**、把当前分支上游切换为目标远端同名分支（已是当前上游时禁用）；**提交历史**（最近 50 条，显示主题/作者/时间/7 位短哈希，底部「…」无边框图标按钮逐批加载更早提交，状态变化自动回到最新一页）；**变更自动刷新**（监听仓库目录与 `.git`，300ms 防抖，避免频繁创建 git 子进程）；**拉取/推送**（合并为胶囊按钮）：拉取（快进优先 `--ff-only`，分叉时提示先手动合并，绝不留下冲突状态）与推送（首次自动 `-u` 设置上游），非快进/认证失败给出中文提示；**合并冲突一律转成合并失败并自动 `git merge --abort`，不生成/残留冲突文件**；**更改/暂存更改分区标题带数量徽章**（仅大于 0 时显示）；目录不在仓库时提供**“添加到 Git”一键初始化**（仅 `git init`，不自动提交）。
- **标题自动总结**：新会话首条消息（去掉文件引用后 > 15 字）发出后，后台用临时线程 + 默认模型生成短标题并写回（仿 VS Code）；与主回合并行、失败静默保留原标题；支持 `experimentalApi` 才执行，旧版协议自动回退/跳过。
- **权限模式**：请求批准 / 帮我批准 / 完全访问权限（对话进行中按钮禁用）。
- **任务模式**：执行 / 计划；目标不再作为任务模式，改由输入栏目标旗子挂载/清除（线程级 `thread/goal` 目标，服务端围绕目标自动续跑）。计划模式回合产出计划后弹出“计划已就绪”确认（纯前端 UX，与 VS Code/CLI 一致）。
- **模型与上下文**：模型下拉选择（`model/list`）、推理强度跟随模型动态生成（如 `low/high/max`）、模型按钮显示“名称 (强度)”（权限/任务/模型按钮弹出层点击外部自动关闭、三个弹出层互斥）、修改模型/推理强度应用后立即通过 `thread/settings/update` 同步到当前会话（无会话时由新建会话的 `thread/start` 携带）、上下文窗口百分比椭圆框（悬停显示“上下文已用 xx K，共 yy K”，点击二次确认后手动压缩上下文 `thread/compact/start`）。
- **输入区**：通栏输入框 + 悬浮靠底按钮；`+` 菜单、`@`/`$` 菜单；**上下键选择输入历史**（内存）；**Ctrl+V 粘贴 / 文件拖放添加附件**（截图/图片落盘到临时目录，复制文件用原始路径）；**输入框高度可拖拽调节**（120px ~ 窗口一半）；本地图片附件在消息中直接渲染（输入区带缩略图）；发送/停止二合一按钮（带文字）；**新建会话前可选项目目录**。`@` 文件引用以 VS Code 扩展同款 `# Files mentioned by the user:` 文本段发送，`$` 技能以 `[$name](path)` 文本链接 + 结构化技能项发送（服务端注入技能内容），回显时渲染为 `@名称` / `$名称` 引用标签。
- **头部**：六边形 Logo 在左；右侧是“新建会话（与设置同形态的 32px 圆角方形图标按钮，加号 18px 加粗等比放大的强调色以区分主次；点击先弹文件夹选择器选定本次会话工作目录，取消则流程直接结束；选定后新建会话标签并聚焦输入框，右侧面板保持当前 Tab）+ 设置（中性灰 16px 图标按钮）”；切换会话标签不中断后台回合（仅关闭运行中的标签或关闭应用时确认）。
- **窗口标题**：主窗体标题固定为 **Codex UI**，与当前会话无关。
- **关闭守卫**：关闭主窗口时若仍有工作中的标签（会话回合/目标续跑、终端命令执行中），先弹确认，确认后统一停止全部工作会话（含清目标）并终止运行中终端，再关闭窗口；未保存文件会先提示保存。
- **设置**：模态对话框（“保存”/“取消保存”，保存后关闭）；**三套主题（蓝夜/曜黑/晨光）点击卡片即时预览、保存后生效**；codex 路径（留空自动查找，设置页显示当前检测到的路径、仅展示不写入配置）、Enter 快捷发送、跟进处理方式（调整方向 / 加入队列）、提示音开关、**记忆管理**（记忆模式关闭/启用，默认关闭，新建会话自动应用、保存时同步当前会话；「重置记忆」二次确认后清空全部已保存记忆）（权限模式、模型、推理强度为进程级，在输入区按钮菜单中配置）。

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
│   ├── git.rs         Git 更改：全部调用系统 git.exe（状态/diff/初始化/分支/合并/拉取/推送），
│   │                  启动探测 git --version 并缓存，300ms 防抖监听
│   ├── mod.rs         模块声明
│   └── settings.rs    设置读写
└── Vue 3 前端  src/
    ├── composables/useCodex/  状态中心（目录，按领域拆分：store/事件桥/会话/回合/历史等，入口 index.ts 再导出）
    ├── composables/useEditorTabs.ts  左侧多标签状态（文件/diff/预览/终端标签；统一关闭入口 closeAnyTab）
    ├── composables/useTerminalEvents.ts  终端事件桥（spawn 前监听 + 按 id 缓冲/回放）
    ├── composables/useSessionFs.ts  会话资源状态（懒加载/搜索/监听/图标缓存）
    ├── composables/useGitChanges.ts  Git 更改状态（检测/刷新/初始化/监听）
    ├── lib/tabs.ts  标签公共基类与统一判定（EditorTabBase / TabKind / TabIcon / isTabWorking）
    └── components/              界面组件（RightPanel + HistoryView + ResourceView、EditorPane 多标签编辑区：TextEditorPane/DiffPane/PreviewPane/TerminalPane 等）
```

后端负责 codex 子进程生命周期（断线自动重连）、JSON-RPC 请求/响应分发、服务端通知流式转发（`item/started`、`item/completed`、`item/agentMessage/delta`、`item/commandExecution/outputDelta`、`interaction:request` 等）与交互响应回写。前端通过 Tauri invoke/event 通信。

## 环境要求

- Windows 10/11（自带 WebView2）
- Rust stable-msvc（rustup）+ VS C++ 构建工具
- Node.js 18+
- `codex` CLI（可在设置页指定路径；验证基线 0.146.0-alpha.9.2）
- 全部 Git 功能需要系统已安装 **Git**（启动时探测 `git --version` 并缓存；未安装时功能禁用/报错并提示安装）

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
npm test                                   # 前端单元测试（vitest，等价 npm run test:unit）
npm run test:typecheck                     # vue-tsc 类型检查
npm run test:coverage                      # 前端单测 + 覆盖率（v8；门槛 lines≥80 / functions≥75 / statements≥75 / branches≥70）
npm run test:rust                          # Rust 单元测试（cargo test --lib）
$env:CODEX_BIN='codex'; npm run test:rust:integration
                                           # Rust 真实 app-server 集成测试（握手/回合、置顶、目标全生命周期、记忆模式、线程设置同步、回合列表 full、会话搜索）
npm run test:all                           # 单元 + 类型 + Rust（未设置 CODEX_BIN 时集成用例自动跳过）
npm run test:e2e                           # E2E 一键编排（见下）
```

### 端到端（E2E）

E2E 驱动真实 release UI（WebView2 CDP）+ 少量真实模型调用，需要已构建 release 与可用的 `codex` CLI：

```powershell
node scripts/run-e2e.mjs --build           # 构建（npm build + cargo release）后串行跑 core/diff/goal/mentions/audit(static)
node scripts/run-e2e.mjs --only goal       # 只跑目标探针（逗号分隔多个）
node scripts/run-e2e.mjs --continue        # 单个失败后继续运行剩余探针
node scripts/run-e2e.mjs --audit-chat      # 主题审计改用 chat 阶段（默认 static）
```

探针清单（`scripts/`，共享基础设施在 `scripts/lib/e2e.mjs`）：

- 自启应用：`verify-core.mjs`（核心冒烟/置顶/切换停止）、`verify-goal.mjs`（线程级目标全流程）、`verify-diff.mjs`（独立 diff 窗口）、`verify-mentions.mjs`（@ 文件 / 插件 / $ 技能）、`audit-themes.mjs`（三主题截图取证）。
- 附着运行中的应用（需先以 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 启动，支持 `--port` 与 `CODEX_E2E_PORT`）：`verify-approval.mjs`（审批，含“批准并记住此规则”，需 `CODEX_E2E_TARGET`）、`check-mode-lock.mjs`（回合中模式按钮禁用）。
- 协议级探针（直连 `codex app-server --stdio`）：`probe-approval*.mjs`。

E2E 探针自建临时目录与会话，结束时自动清理；CDP 端口被占用或应用启动失败时给出明确诊断，可用 `CODEX_E2E_PORT` 换端口。

## 使用说明

- **工作目录**：从哪个目录启动 codex-ui，新建会话默认归到该目录；点击头部「新建会话」会先弹出文件夹选择器，可选本次会话的工作目录（取消则沿用默认）。历史会话显示各自记录的目录，继续执行用记录目录。
- **多会话标签**：标签栏末尾「+」直接新建空会话标签（默认工作目录）；头部「+」先选目录再新建。会话标签可关闭，允许关闭到 0 个（主区域显示空状态提示）。
- **历史会话**：面板顶部即搜索框；按目录分组（目录按名称 A-Z、会话按置顶+时间倒序），文件夹默认收起，支持全量加载、重命名、固定置顶、删除确认。点击会话行：未打开则新标签打开、已打开则聚焦对应标签（点击当前会话不重载）；已打开会话行显示「已打开」标记、后台运行中显示呼吸点；切换标签不中断后台回合；关闭窗口时若有工作会话/终端会先确认。输入消息后才会恢复并执行；新会话首条消息会自动生成短标题。带**活跃目标**的会话恢复后会由服务端围绕目标自动续跑回合；点击目标旗子旁的 × 清除目标即停止。
- **会话资源**：右侧面板底部切换到“资源”Tab；根为**活动标签的工作区**（会话标签→会话工作目录；文件/diff/预览/终端标签→各自打开时的工作区），根节点显示文件夹名、默认展开第一层；`.git`/`.codegraph` 等点目录不显示；文件/目录右键管理，文件变化自动刷新；切换标签后根随之切换。
- **权限模式**：请求批准会询问所有外部写操作；“批准并记住此规则”会把规则写入 codex 策略。完全访问不受限。权限/模型/推理强度为**进程级**，重启后恢复默认。
- **设置文件**：`%APPDATA%\com.codexui.app\settings.json`，仅保存 codex 路径、Enter 快捷发送、跟进处理方式、提示音开关、主题、权限模式初始值与记忆模式。
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
- **提示“未检测到系统 git”**：全部 Git 功能依赖系统 Git；请安装 Git（git-scm.com）或将其加入 PATH，安装后重启应用（启动探测失败后整个会话视为未安装）。
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
