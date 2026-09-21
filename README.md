# codex-ui

基于 **Rust + Tauri 2** 的 Codex CLI Windows 桌面客户端。`codex-ui.exe` 是对 `codex` CLI 的桌面包装：
启动后拉起 `codex app-server --stdio`，用与 VS Code Codex 插件相同的 JSON-RPC 协议驱动 codex，提供完整的中文桌面界面。

> `codex-ui.exe` 启动目录是 codex 子进程的工作目录；
> 应用内「工作区」以**当前会话或编辑器标签记录的目录**为准，无确定工作区时（未打开/创建任何会话）资源与 Git 面板显示「暂无工作目录」、不监听任何目录；
> 历史会话按目录记录，点击任一历史会话会使用**该会话记录的工作目录**继续执行。

系统托盘常驻：关闭主窗口时应用隐藏到系统托盘继续运行（codex/微信保持在线，不弹确认、不停止工作）；
左键点击托盘图标显示并聚焦主窗口，右键菜单「退出」会先安全收尾（静默停止工作会话、终止运行中终端）再退出应用，未保存文件不自动保存。
发布版同一时刻仅允许一个实例，重复启动会聚焦已有窗口（含从托盘恢复），托盘仅保留一个图标；开发版不限制多开。

## 功能

### 聊天流

- 流式 Markdown（表格、任务列表、代码块复制按钮、链接悬停显示完整 URL、图片、ANSI 彩色输出；
  源码里的尖括号内容（原始 HTML、注释、CDATA 等）一律按字面文本显示，不解析为标签）、
  思考过程（标题固定「思考过程」、默认折叠、折叠态单行预览实时跟随最新一行且超宽时右侧对齐露出最新字符、完成后显示耗时）、
  进行中阶段徽标与记忆引用、计划/待办、上下文压缩提示；
- 对话正文中的文件链接点击在应用内打开（按类型进文本编辑/预览标签，不支持的格式降级为资源管理器定位），`@` 文件引用点击在资源管理器中定位；
- 回合进行中顶部流光进度线 + “等待响应”实时计时发光提示（x.y 秒固定一位小数）；
- **复制统一化**：计划/命令卡片右上角、助手最终答复与用户消息悬浮右上角均提供统一的「复制」按钮（复制所指内容的原始 Markdown 或正文），不再散落“复制计划/复制命令”等文案。

### 滚动体验

- 流式输出时上滑查看历史不会被拉回底部，右上角“回到底部”按钮一键恢复跟随；
- 历史会话打开后先在后台预热全文，首次预热结束后滚动条位置出现导航按钮（完全贴右、与滚动条同宽，四条横线图标；若本次预热结束时布局尚未稳定——大历史会话下正文仍在落地——会在后台按间隔继续重试，不影响按钮出现时机）——
- 悬停按钮即在按钮左侧弹出回合导航卡片（全高铺满聊天区高度、宽 280px、无圆角，条目少时垂直居中、多时内部滚动）、移出自动收起（键盘 Enter/Space 也可开合），
- 卡片按时间正序以单行紧凑列出每个回合（纯数字序号 + 标题单行省略（不设字数截断）+ 时间，悬停条目直接在卡片旁预览该条用户消息气泡，当前回合高亮；用户消息为“执行计划”指令时标题直接取该计划标题），
- 点条目即跳转到该回合起点（目标消息对齐视口顶部并短暂高亮），落地后会在有限窗口内（最长约 1.5s）持续按实时布局对准目标——大历史会话的正文（Markdown/代码高亮/图片）仍在陆续落地、量取时的高度与最终高度不同，一次性落点会漂移；期间一旦自己滚动或按键，跟随立即让位。跳转后不会被流式输出拉回底部，直到自己滚到底部或点「回到底部」。

### 工具卡片

- 命令执行显示真实命令、工作目录、实时输出、**实时耗时**、退出码；
- 文件变更带 diff；
- 网络搜索（结构化结果列表）、MCP/动态工具、子代理协作。

### 交互提示

- 提权审批（批准 / 拒绝 / 本次会话批准 / 批准并记住此规则）、选项提问、MCP 表单**内嵌在聊天消息流中**（待处理交互气泡，回答时上方上下文完整可见；交互挂起时隐藏“等待响应”；CODEX 会话标签显示待处理计数角标，切到文件/diff 标签也不会错过）；
- **计划已就绪确认**同为消息流内嵌气泡（仿 VS Code/CLI：计划模式回合完成后在消息流末尾弹出
  “执行计划 / 待在计划 / 退出计划模式”，完整计划见上方消息、气泡不重复展示，
  执行计划自动发送 `PLEASE IMPLEMENT THIS PLAN:` 消息并切到默认模式，气泡按钮聚焦时 Space 不触发默认点击、防误执行）；
- 交互（提权审批 / 提问 / MCP 表单）与「计划已就绪」出现、回合正常完成、以及会话报错时，若窗口没有前台焦点则发 Windows 系统通知（默认开启，可在设置 → 个性化关闭）；
- 回合可随时停止并提示“已停止生成”；
- 非编辑区自定义右键菜单（复制/打开链接），文本输入控件剪切/复制/粘贴/全选，均无默认菜单；
- 右键菜单不被会话流式更新的吸底滚动误关闭（仅当被滚动的容器包含菜单锚点时才关）。

### 输入区

#### 输入与附件

- 通栏输入框 + 悬浮靠底按钮；
- `+` 菜单、`@`/`$` 菜单；
- **上下键选择输入历史**（内存）；
- **Ctrl+V 粘贴 / 文件拖放到输入区添加附件**（截图/图片落盘到临时目录，复制文件用原始路径）；
- **文件拖放到输入区以外的窗口任意位置直接打开文件**（文本/代码 → 编辑器标签，PDF/图片/视频/音频/表格/DOCX/PPTX → 对应预览标签，多文件依次打开、最后一个聚焦）；仅拖到输入区（当前活动会话的输入框）才作为附件，拖到非输入区（头部、资源树、空白处等）一律打开文件；
- **输入框高度可拖拽调节**（120px ~ 窗口一半）；
- 本地图片附件在消息中直接渲染（输入区带缩略图）。
- **点击缩略图打开图片灯箱**：全屏预览，点任意位置或 `Esc` 关闭，光标统一普通箭头。
- 灯箱内**右键可「复制图像」**——按本地路径或 data URL 解码后以 CF_DIB（+ CF_BITMAP）写入 Windows 剪贴板，可直接粘贴到聊天/文档。
- 远程 http(s) 图片不做下载，会提示先另存到本地。
- 发送/停止二合一按钮（带文字）；
- **新建会话前可选项目目录**。
- `@` 文件引用以 VS Code 扩展同款 `# Files mentioned by the user:` 文本段发送，`$` 技能以 `[$name](path)` 文本链接 + 结构化技能项发送（服务端注入技能内容），回显时渲染为 `@名称` / `$名称` 引用标签。

#### 权限模式

- 只读访问 / 请求批准 / 帮我批准 / 完全访问（对话进行中按钮禁用）。

#### 协作模式

- 默认 / 计划；
- 目标不再作为协作模式，改由输入栏目标旗子挂载/清除（线程级 `thread/goal` 目标，服务端围绕目标自动续跑）。
- 计划模式回合产出计划后弹出“计划已就绪”确认（纯前端 UX，与 VS Code/CLI 一致）。
#### 模型与上下文

- 模型下拉选择（`model/list`）、推理强度跟随模型动态生成（如 `low/high/max`）、模型按钮显示“名称 (强度)”。
- 权限/协作模式/模型按钮弹出层点击外部自动关闭、三个弹出层互斥。
- 修改模型/推理强度应用后立即通过 `thread/settings/update` 同步到当前会话（无会话时由新建会话的 `thread/start` 携带）。

- 上下文占用与 token 用量由输入区的**上下文圆环**承载——输入栏右下角模型选择按钮前显示圆环进度（30px，与模型芯片同高；进度=上下文占用百分比，环心只显示数字）。
- 悬停向上弹出「上下文与 Token」菜单——圆环正上方居中：细进度条与 `[百分比│压缩图标]` 合并胶囊（`thread/compact/start`，压缩中禁用）、会话累计输入/输出/合计与缓存读取/缓存写入/推理输出细分。
- 数据来自 `thread/tokenUsage/updated` 的 `total`、`last` 与 `modelContextWindow`。

### 会话列表

- 面板**常驻右侧且宽度可拖拽调节（300px ~ 窗口一半）**，展示全部目录的会话并**按目录分组为文件夹
  （目录按名称 A-Z，会话按置顶+最近时间倒序；目录默认全部收起，仅启动恢复最后活跃会话时展开其所属分组，
  单条会话不建夹、不显示路径）**；
- **全量加载**、重命名、固定置顶（行首显示钉子图标）、搜索（带摘要，搜索框右侧刷新按钮）、删除确认；
- 会话行右键菜单**未打开时为「打开 / 重命名 / 分叉会话 / 置顶固定 / 删除会话」，已打开（有会话标签）时为「重命名 / 分叉会话 / 置顶固定」**，
  其中**分叉会话**把源会话完整复制为一个新线程（服务端维护 `forkedFromId`/`sessionId`），
  成功后新标签打开新会话、沿用源名称、聚焦输入框，可直接在新分支上继续
  （已打开会话行显示「已打开」标记，后台运行中显示呼吸点）；
- **打开历史会话即恢复线程**（无活跃目标时）：右上角立即显示该会话累计输入/输出 token 用量，线程随之加载到服务端；
- **带活跃目标的会话仍保持只读**，发消息才恢复（避免「看一眼」就触发服务端围绕目标自动续跑）；
- 目录（文件夹）行右键可**新建会话（预置该分组目录）**、**在此打开终端（以分组目录启动）**、**在资源管理器中打开**、**删除所有会话（组内全部已打开时隐藏该项；确认后仅删除未打开的会话，已打开跳过并提示数量）**；
- 新建或打开会话后聚焦输入框，右侧面板保持当前 Tab、不中断后台回合；
- 点击会话行：未打开则新标签打开、已打开则聚焦对应标签（点击当前会话不重载）；
- 继续执行使用会话记录的工作目录，历史中的命令/工具详情完整加载。

### 标题自动总结

- 新会话首条消息（去掉文件引用后 > 15 字）发出后，后台用临时线程 + 默认模型生成短标题并写回（仿 VS Code）；
- 与主回合并行、失败静默保留原标题；
- 支持 `experimentalApi` 才执行，旧版协议自动回退/跳过。

### 启动恢复最后活跃会话

- 应用启动（`init()` 完成后）读取配置文件持久化的最后活跃会话 id（`settings.json` 的 `last_session_id`）；
- 该线程仍在历史中则**打开该会话标签并聚焦输入框**，同时在会话列表**仅展开其所属目录分组**（其余分组收起）；
- 配置无记录（或会话已删除 / 打开失败）则**所有分组均不展开**并回退**打开设置标签**（设置页恒在标签栏最后，可手动关闭）；
- 不再展示「欢迎」介绍标签。
- 会话 id 平时仅记录在内存（会话标签激活时更新，**不写配置文件**），托盘「退出」收尾时才落盘。

### 会话资源

- 右侧面板底部 Tab（会话 / 资源 / Git）；
- 以**活动标签的工作区**为根的文件树（会话标签→会话工作目录；文件/diff/预览/终端标签→各自打开时的工作区），
  **懒加载、默认只展开第一层**，名称以 `.` 开头的目录与 `node_modules` 无条件隐藏，**根节点悬停显示完整路径**；

#### 文件树与图标

- **文件行显示文件类型图标**（常见代码/配置类型优先用内置 VS 风格彩色 SVG 图标
  （`lib/fileTypeIcons.ts`，覆盖 TS/JS/Python/Rust/Go/Java/C 系/Web/数据配置等约 30 类扩展名、
  图片/视频/音频媒体扩展名与 Dockerfile/Makefile/.gitignore/.env 等特殊文件名，矢量在高清屏下更清晰），
  其余类型提取 Windows 系统图标；懒加载 + 按扩展名缓存，资源树/编辑器标签/Git 面板共用同一份缓存）；

#### 搜索与显示

- 头部搜索框递归搜索文件/目录名，并在捆绑 `rg.exe` 可用时并行搜索文件内容
  （内容命中显示首个命中行摘要；rg 缺失时静默仅按文件名/目录名搜索）；
  搜索框右侧为**显示态切换按钮**，在「显示时间」（默认，文件行显示相对修改时间）与「显示大小」之间切换，
  图标呈现当前态（时间=时钟图标、大小=「Aa」字形，与时钟同色）、悬停提示为「切换显示：修改时间/文件大小」，
  只影响文件行右列（目录/根行恒为项数徽章），偏好在本次运行内保留；

#### 自动刷新

- **文件变化自动刷新**（`notify` 监听、300ms 防抖；切换标签与工作区切换时同样重载）——
  头部不再提供手动刷新按钮；

#### 按类型打开

- 文件单击或右键「打开」**按类型分发**（已打开的文件右键菜单不显示「打开」，单击仍激活对应标签）：
  文本文件在左侧标签页以文本编辑器打开（语法高亮、可编辑），
  `.pdf` 与图像文件（png/jpg/jpeg/gif/webp/bmp/svg/ico/avif）打开只读预览标签
  （PDF 用 pdf.js 渲染、支持翻页缩放；图像经 asset 协议直显），
  视频/音频文件（mp4/webm/mkv/mov/avi/m4v/ogv、mp3/wav/ogg/flac/m4a/aac/opus 等）打开播放标签
  （`<video>` / `<audio>` 经 asset 协议流式加载，播放/暂停/进度/音量用原生控件，加载失败提示无法播放），
  表格类文件（`.xlsx` / `.xlsm` / `.xlsb` / `.xls` / `.ods` / `.csv` / `.tsv`）打开只读表格预览
  （工作表切换、内容可选中复制；CSV/TSV 按 UTF-8 或带 BOM 的 UTF-16 解码），
  `.docx` 打开 **Word 排版预览**（docx-preview 按 Word 原版式分页渲染：
  标题/字体/颜色/对齐/缩进/行距/页眉页脚尽量还原，只读，支持缩放：工具栏 ±/百分比/适应宽度与 Ctrl+滚轮，
  与 PDF 预览同款交互，内容可选中复制），
  .pptx 打开 **演示文稿预览**（pptx-preview 按幻灯片版式纵向平铺渲染，只读，缩放交互与 Word/PDF 预览同款），
  其余非文本提示无法打开；

#### 文本编辑与格式化

- 文本编辑器内置**自定义右键菜单**（撤销/重做/剪切/复制/粘贴/全选/查找替换、**代码格式化**，右键定位光标），
  「代码格式化」覆盖全部已识别语言：JS/TS/JSON/JSONC/CSS/HTML/YAML/Markdown 走 Prettier 完整美化（可展开单行），
  XML/SVG 走自研语法树美化，Python/Go/Rust/C++/Java 等其余语言复用 CodeMirror 缩进规则重排（仅修正缩进）；
- 格式化单次事务可撤销，语法错误 toast 提示；
- `.md`/`.markdown` 文件支持**「预览/编辑」切换**，预览宽度随窗口自适应（不再受 920px 阅读上限约束）；

#### 会话附件与拖拽

- **文件/目录行悬停显示「@」按钮，可一键添加为会话附件**；
- **文件/目录可拖拽到目录或根节点完成移动**（同卷 rename、跨盘自动降级复制+删除，禁止移入自身/子目录）；

#### 右键菜单管理

- 右键菜单管理（**新建文本文件/新建文件夹**、打开/复制/粘贴（剪贴板无文件时隐藏「粘贴」）、重命名、删除、属性、在资源管理器中打开、添加为会话附件），新建文本文件/新建文件夹后自动进入行内重命名；
- 支持从系统剪贴板粘贴文件/文件夹，属性弹窗显示大小与时间；
- 切换 Tab 保留展开状态。

### 左侧标签页（多标签编辑区）

- 主窗口左侧为标签页编辑区，**多个会话标签（可关闭、允许 0 个）**——会话标签标题为 `工作区目录名 / 会话标题`（新对话为 `… / 新建会话`），带进行中呼吸圆点与待处理交互计数角标；
- 标签栏常驻显示、末尾「+」可快速新建空会话标签；
- 允许关闭到 0 个会话标签（主区域显示“当前还没有任何打开的项”空状态）；
- 其后可打开文件（CodeMirror 编辑）、diff、PDF/图像预览与 **PowerShell 终端**标签；
- 标签 `v-show` 常驻挂载，切换标签不销毁编辑状态/进程；
- 会话标签右键「关闭所有标签 / 关闭其它会话标签」，文件/diff/预览/终端标签右键「关闭所有标签 / 关闭左边 / 关闭右边（文件类另有「在资源管理器中打开」）」——未保存（脏）文件与**运行中的终端/会话**跳过并提示数量，空闲终端批量关闭时结束进程；
- 关闭运行中的会话标签或终端需确认。

### 终端标签

- 资源管理器文件夹与历史会话分组的「在此打开终端」在标签区启动 `powershell.exe`（Windows ConPTY，UTF-8 编码），支持同目录多开、切走不中断，标签标题固定为 **PowerShell**（无 ToolTip）；
- `useTerminalEvents` 事件桥在 spawn 前注册全局监听并按 id 缓冲启动输出（含 ConPTY 光标位置查询，xterm 自动应答），面板挂载后回放，修复首个终端空白；
- spawn 完成后自动补一次 fit/resize；
- 标签激活时自动聚焦 xterm（点击或切换回终端标签即可直接输入）；
- **命令执行中标签显示呼吸圆点**（后端每次回到提示符注入隐藏标记 OSC 133;D + 前端回车/标记判定，回到提示符自动熄灭）；
- `Invoke-WebRequest`/`wget` 默认 `-UseBasicParsing`（2025-12 安全更新 CVE-2025-54100 新增的“脚本执行风险”确认不再弹出）；
- **关闭运行中的终端需确认**（终止并关闭/取消），批量关闭跳过运行中的终端并提示数量；
- 终端配色深度适配三套主题（完整 16 色 ANSI + 光标前景，浅色主题下输入回显、提示符与彩色输出清晰可读），切换主题即时生效；
- **终端背景与 tab 内容区背景同色**（终端与四周留白均用 `--bg`，为不透明色，避免半透明双层合成色差），三主题下四周无接缝；
- **支持系统剪贴板复制/粘贴**（右键菜单「复制/粘贴」，Ctrl+V 直接粘贴，含多行），粘贴文本由 `clipboard_read_text` 后端命令读取系统剪贴板（不再触发 WebView2「查看复制到剪贴板」权限确认框），按回车输入到命令。

### Git 更改

- 右侧面板底部 Tab（会话 / 资源 / **Git**），后端全部功能调用系统 **git.exe** 子进程（隐藏控制台窗口；启动时执行一次 `git --version` 探测并缓存，未安装时整个会话报“未检测到系统 git”）；

#### 状态列表与 diff

- 以**活动标签的工作区**为根检测所在仓库，列出**已更改文件**。
- 行首文件类型图标 + 行末状态字母徽标 A/M/D/R/U/C（绿/黄/红配色），悬停提示状态文字 + 分支名；`node_modules` 无条件忽略。
- 点击文件在主窗口左侧标签页打开 diff 标签查看：统一对比 HEAD，暂存 + 未暂存合并；未跟踪文件以空内容对比。
- **完整/简要切换**：行数超 2 000 行自动进简要显示、超过 10 000 行截断并提示；行内高亮在单行超 4 000 字符或总量超 200 000 字符时关闭；逐行预览对 > 2 MB 的文件回退为原始 diff 文本。

#### 分支管理

- **分支管理**：点击分支名弹出「本地 / 远程 / 远端管理」三分区下拉弹层。
- 本地：**切换 / 新建 / 删除 / 合并**（`git switch` 标准语义：无冲突即可切换且未提交改动随分支携带，本地改动/未跟踪文件将被覆盖时拒绝；删除用 `git branch -D` 无条件删除（不校验是否合并），禁止删除当前分支）。
- 远程：列出远端跟踪分支、**拉取刷新**、检出为本地跟踪分支、删除远程分支（当前上游时二次确认）。
- 远端管理：**添加 / 删除远端**、把当前分支上游切换为目标远端同名分支（已是当前上游时禁用）。

#### 提交历史

- **提交历史**（最近 50 条，显示主题/作者/时间/7 位短哈希，底部「…」无边框图标按钮逐批加载更早提交，状态变化自动回到最新一页）。

#### 拉取推送与冲突

- **变更自动刷新**（监听仓库目录与 `.git`，300ms 防抖，避免频繁创建 git 子进程）；
- **拉取/推送**（合并为胶囊按钮）：拉取（快进优先 `--ff-only`，分叉时提示先手动合并，绝不留下冲突状态）与推送（首次自动 `-u` 设置上游），非快进/认证失败给出中文提示；
- **合并冲突一律转成合并失败并自动 `git merge --abort`，不生成/残留冲突文件**；

#### 分区标题交互

- **分区标题为内容根行**（更改/暂存更改/提交历史：图标位于折叠箭头与文字之间，整行 hover 高亮，Enter/Space 可切换折叠，分区内容统一挂在标题之下的子容器并整体缩进，与资源根节点/会话目录行同形态）与**数量徽章**（仅大于 0 时显示）；
- **分区标题右键菜单**——更改区「暂存（全部暂存）/撤消更改」、暂存更改区「取消暂存（全部取消暂存）/撤消更改」（撤消需确认后逐文件丢弃，含已暂存/未暂存部分）；

#### 添加到 Git

- 目录不在仓库时提供**“添加到 Git”一键初始化**（仅 `git init`，不自动提交）。

### 定时任务

- **仅能由 codex 在对话中创建**（动态工具 `codexui_add_scheduled_task`，**不弹前端确认框**——任务可能经微信等无人值守渠道发起；由 codex 在对话中向用户复述任务名/触发时间/prompt 全文获得同意后创建，无手动创建入口）。

#### 创建与授权

- 任务**绑定创建时的会话（仅存 threadId）**，到点由应用在该会话中自动发送任务指令开启新回合，结果直接出现在原会话；

#### 调度与执行

- 调度为 6 字段「秒 分 时 日 月 周」cron 表达式（本地时区，7 字段末尾年份表示单次），**最小触发间隔 1 分钟**（过小在创建时被拒绝，建议 ≥5 分钟）；
- 到点时若会话忙按任务级策略**跳过本次（默认，等到下一个触发点）或顺延（空闲后立即执行，期间多次到点合并为一次）**；
- 权限/模型/推理强度**执行时按会话实时读取**（未设置过则回退会话默认）；

#### 数据与通知

- 执行记录按任务分开查看（状态/耗时/结果摘要/错误，全量保存、分页加载），可一键打开绑定会话；
- 单次任务完成后归入「已完成」归档分组；
- 删除会话级联删除其定时任务，删除任务级联删除执行记录。
- 任务数据存于应用自有 SQLite 库（`%APPDATA%\com.codexui.app\codexui.sqlite`）；
- **调度依赖应用运行（托盘常驻即可），应用未运行期间错过的触发点记「已错过」不补跑**；
- 任务**真正开始执行**与**执行结束（成功/失败）**时各发一条 Windows 系统 toast（操作中心通知，非托盘 tooltip；跳过/错过/顺延不发），点击 toast 上的「打开会话」聚焦应用并打开该任务绑定的会话。

### 对话内管理（codexui 动态工具）

见「设置」→「动态工具」。

### 应用外壳

#### 头部（自绘标题栏）

- 主窗口隐藏系统原生标题栏，顶栏即自绘标题栏——左侧六边形 Logo + **CODEX** 文字；
- 右侧依次为：**「设置」**（中性灰 16px 图标按钮）、**「切换布局」**（左右布局 ⇄ 隐藏面板两态图标，点击在显示/隐藏右侧面板间切换，隐藏时编辑区占满全宽且面板内容状态保留，仅本次运行生效不持久化）、**「最小化 / 最大化(还原) / 关闭」**（关闭悬停变红）；
- 标题栏空白处可按住拖动窗口、双击最大化/还原，点击设置/窗口按钮不会误触拖动；
- 点击「关闭」与原生 X 语义一致——仅隐藏到系统托盘。
- 切换会话标签不中断后台回合（仅关闭运行中的标签或关闭应用时确认）。

#### 窗口标题

- 任务栏标题跟随当前活动会话标题（无活动会话回退 `Codex UI vX.Y.Z`）；
- 自绘标题栏本身只显示品牌，不显示会话标题。

#### 关闭与退出

- 关闭主窗口仅隐藏到系统托盘（不弹确认、不销毁窗口、不停止工作，未保存内容仍驻留内存）；
- 托盘「退出」先静默收尾——停止工作会话（含清目标）并终止运行中终端——再退出应用，未保存文件不自动保存。

## 设置

- 模态对话框（“保存”/“取消保存”，保存后关闭）。

### 主题与外观

- **三套主题（蓝夜/曜黑/晨光）点击卡片即时预览、保存后生效**，主题统一为 Raycast/Linear 式光影质感（悬浮阴影、毛玻璃、主色辉光与背景氛围光，浅色主题保持通透明亮）。
- **毛玻璃特效（设置 → 个性化 → 「毛玻璃主题外观」右侧开关，默认开启）**：Windows 10/11 使用系统 Acrylic（Windows 11 下由 DWM 提供系统毛玻璃背景），窗口底层与列表/布局容器透出系统模糊，下拉选择/菜单/对话框/气泡等弹层叠加 `backdrop-filter` 背景模糊，消息、输入框、编辑器与弹层等文字承载面保持近不透明。
- 关闭开关即恢复纯色主题外观。

### 基础设置（通用设置）

- 保存后需重启 codex-ui 才能生效、Enter 快捷发送、跟进处理方式（调整方向 / 加入队列）；
- **两个系统通知开关**（会话错误 / 会话提权·交互·完成，均默认开启且只在窗口没有前台焦点时生效）；
- **记忆管理**（记忆模式关闭/启用，默认关闭，新建会话自动应用、保存时同步当前会话；「删除记忆」二次确认后清空全部已保存记忆）；
- 权限模式、模型、推理强度为会话级，在每个会话标签的输入区按钮菜单中配置，新会话取默认。

### 模型快照

- **独立 Tab**：模型快照从「模型配置」Tab 里的首张卡片独立为左侧导航分类，位置在「模型配置」之前（区标题「模型快照」，卡片小标题「已保存快照」）；还原快照仍会自动重读「模型配置」Tab 的卡片内容。
- 在 `CODEX_HOME/codex-ui/<名称>.json` 保存单个 JSON 模型快照，不再使用独立快照目录；旧的 `<名称>/` 配置快照目录会保留在磁盘但不列出。
- 快照包含：
  - 模型相关字段（`model`、`model_reasoning_effort`、`model_reasoning_summary`、`personality`、`model_verbosity`、`model_provider`、`preferred_auth_method`、`forced_login_method`）
  - 全部提供方原始配置
  - `model_catalog_json` 字段及模型目录文件的 JSON 内容。
- 提供方密钥也会写入，分享前请确认。
- **还原**：直接读取快照并只覆盖 config.toml 中的上述模型字段与 `model_providers` 表，保留其它配置、注释与格式；模型目录文件通过临时文件原子替换写入。
- 还原成功后**自动重读一次模型配置**（「模型配置」Tab 的卡片立即显示快照里的模型/提供方/模型目录内容），toast「已应用模型快照「x」，重启 codex-ui 后生效」，不会调用 app-server 热重载。
- **目录路径**：快照目录内容存在时优先使用原 `model_catalog_json` 路径；原路径在目标环境不可用（目录不存在/目标是目录/指向快照文件本身）时回退写入 `CODEX_HOME/model_catalog.json` 并把字段改为 `model_catalog.json`。
- **删除**：确认后删除对应 `.json` 文件。
- **打开**：在编辑器打开该快照 JSON。
- **新建/覆盖（卡片头「+」→ 输入名称弹窗 →「取消 / 确定」）**：直接读取当前 `config.toml` 与模型目录文件写入快照，同名已存在则覆盖更新。
- 快照名需为合法 Windows 文件名（自动忽略末尾 `.json`）。

### 模型配置与标量字段

- codex 路径（留空自动查找，设置页显示当前检测到的路径、仅展示不写入配置）、模型配置内置 DeepSeek / GLM 接入文档链接。
- **模型目录 `model_catalog_json` 路径自动管理**：无需手填，按 config 实际值解析、未配置时默认 `CODEX_HOME/models.json` 绝对路径；保存时校验目录 JSON 结构，目录为空则删除该配置键。
- 目录编辑框不可拖拽缩放；模型目录标题本身即文件链接（悬停显示完整路径），文件存在时可点击在编辑器中打开，文件不存在时标题置灰、tooltip 提示保存时将新建。
- **模型名称输入框点击展开全量候选**：由目录 `slug`（缺失回退 `id`）解析、不按已填值过滤，点选即回填。
- 回复风格 `personality`（friendly / pragmatic / none）、推理摘要 `model_reasoning_summary`（auto / concise / detailed / none）
  与输出详细程度 `model_verbosity`（low / medium / high）均支持默认不写入，
  随配置经 `config/batchWrite` 保存（**GPT 系只返回加密推理 + 摘要，把「推理摘要」设为 auto 才有「思考过程」内容**）。
- **标量字段顺序**：`model → 推理强度 → 推理摘要 → 认证方式 → 强制登录 → personality → verbosity`——「推理强度」与「推理摘要」相邻，两个一起调整时不必来回找。
- **推理强度的空档位语义**：目录里的 `supported_reasoning_levels` 为空表示"未知"而不是"明确无档位"——会话里切到这类模型
  （如 Zen 的 `big-pickle`）**保留当前推理强度**，新建会话则沿用提供方的 `model_reasoning_effort` 作默认强度；
  只有模型明确声明了档位且不含当前值才回落默认，避免静默变成 `none`、整轮不请求推理。

#### 生成模型目录

- **模型目录区块位于「模型提供方」列表之后、「model（模型标识）」标量区之前**，生成按钮（圆圈+向下箭头）挂在**模型提供方行的第一个按钮位**（「编辑」之前）。
- 只有**当前选中的提供方行**且该提供方同时填写 `base_url` 与 `experimental_bearer_token` 时才渲染，其余行与未选择提供方时都不渲染——可用性用**显示/隐藏**表达，不再有禁用态与原因提示。
- 生成/加载/保存期间按钮以 `disabled` 表达忙态，重复点击由内部守卫忽略。
- 点击获取 `/models`，保留现有选择弹窗、搜索、勾选、三态全选和确认步骤，默认全不选。
- 候选分为可生成、不兼容、未匹配、校验失败，后三类置灰并展示原因。
- **用户只选模型，不需要补参数**：各来源独立查找，系统按匹配准确度、来源权威性逐字段合并；精确资料优先于近似资料，缺失值不清空已有结果。
- 同 ID 的 models.dev 多份记录按规范模型、明确原厂、其他提供方处理，同级冲突自动由其他来源或模板补缺；不按目标 `base_url` 猜 provider，本地代理和 `big-pickle` 等专有 ID 仍可用。

#### 生成参数与来源

- **生成参数与来源**：官方条目池由本机 `codex debug models --bundled` 导出和内置第三方条目构成，原始 ID 精确匹配优先于唯一规范化匹配，歧义时自动转字段源。
- 整条复用保留能力参数与自带提示词，第三方中转只收敛 WebSocket 偏好；官方池与模板在一次生成中共享同一份快照。
- **提示词策略**：命中官方条目池（含唯一规范化匹配）的模型保留条目自带提示词；
  其余模型（多源合并 + 模板渲染）的 `base_instructions` 与 `model_messages.instructions_template`
  都写成 `resources/model_catalog_template.json` 里的精简提示词（约 380 字符）——
  模板基底取自官方条目，其提示词约 18K 字符，会把第三方小窗口模型顶到上限
  （实测表现为模型突然停止调用工具、回合静默结束，随后上游回 500/400）。
- 其他条目经模板补键、自动派生和结构/上下文校验，保留自定义推理档位与音频模态；非法条目单独隔离。
- **推理摘要**：推理条目（含复用的 GPT 官方条目）统一写 `default_reasoning_summary=auto` 并清掉会挡掉摘要的 `supports_reasoning_summary_parameter`，非推理条目写 `none`——否则 GPT 系只回加密推理，「思考过程」卡片空白。
- 第三方资料里的 `status`（`beta` / `deprecated`）不再参与判定：提供方 `/models` 列出的模型都能生成，只有明确声明 `tool_call=false` 的模型仍标为不兼容；模糊继承仍可生成，但近似模型声明的工具限制不会直接禁用目标模型。
- 弹窗只展示候选模型 ID、名称与状态原因，不提供参数输入或冲突确认。
- 点击确定仅把选中条目按候选原顺序回填编辑框，**不重新请求资料、不自动保存**；取消或失败保留原内容。详见 [docs/model_catalog_json.md](docs/model_catalog_json.md)。

#### 模型配置空值语义

- **模型配置空值语义**：提供方列表首项为固定的「**不使用模型提供方**」（选中表示不激活任何提供方）——
  **选择它不会删除既有提供方**，因为 `model_provider` 留空时按空值删键写 `null` 而不是写空串
  （实测：空串会让 codex 判定整份配置无效、用户层从 `config/read` 消失，
  设置页因此读到空提供方列表，下次保存把 `model_providers` 整表覆盖为空）；
- **`wire_api` 仅支持 `responses`**（codex 0.149.x 已不支持 `chat`），历史 `chat` 值在提供方行显示行内错误、保存时校验阻断，编辑弹窗只提供 `responses` 且打开时回填该值，保存一次即修正；
- 推理强度留空表示「未配置」（写 `null` 删键，写空串会被 codex 以 `reasoning_effort must not be empty` 拒绝保存）；

### 动态工具（设置页「动态工具」Tab）

- 新建会话通过 `thread/start.dynamicTools` 注册 `codexui` 命名空间工具，agent 可在对话内直接调用：
  - `codexui_get_usage`（查询当前会话 token 消耗与上下文窗口占用）；
  - `codexui_compact_context`（压缩上下文）与 `codexui_add_scheduled_task`（创建定时任务，见「定时任务」）。
- 工具调用经 `item/tool/call` 回由 codex-ui 应答，设置页「动态工具」区块可逐个启停（实验性、依赖 `experimentalApi`，仅新建会话注入）。

### MCP 管理（设置页「MCP 管理」Tab）

- 配置 stdio / Streamable HTTP 服务器（名称、command/cwd/args 或 url、env/http_headers、Bearer 令牌环境变量、`omit_tools_from` 工具暴露面）；
- 每行「信息」图标打开详情弹窗（标题即服务器名），按「服务器信息 / 工具 / 资源」Tab 展示 codex 实际连接的能力——服务器信息含传输类型、启动命令/url、`serverInfo` 标题/版本/描述/网站与认证状态；
- 工具含名称、描述与可折叠输入参数 JSON；
- 资源与资源模板含 URI/描述/mimeType（资源 URI 可一键复制）；
- 服务器就绪后自动刷新能力清单。

### 插件管理（设置页「插件管理」Tab）

- 顶部「已安装插件」卡片跨市场汇总已安装插件（图标、版本、状态、来源市场，可直接卸载），无已安装插件时显示空态；
- 下方插件市场按市场分组列出插件，每个插件展示接口图标（远程 URL 优先、本地路径其次，缺失或加载失败时以品牌色 + 首字母占位）；
- 支持安装/卸载、添加/移除市场；
- **chrome 插件（openai-bundled）安装后自动「浏览器桥接自愈」**——重装只恢复版本目录、不重建 `latest` junction，安装完成后自动检查并修复链接与原生宿主注册（`browser_bridge_repair`），toast 提示桥接就绪或失败原因；
- **安装与卸载前都会检测浏览器桥接进程**（`extension-host.exe` / `node_repl.exe` 运行时会锁住插件缓存目录，导致安装备份缓存或卸载报 os error 5）——安装时弹确认框、确认后一键结束这两个进程再安装（会中断当前浏览器控制会话，Chrome 扩展下次使用自动重连），卸载时提示先完全退出 Chrome；
- 任何插件安装若仍因缓存占用失败（os error 5）都会给出中文指引而非原始英文错误。

### 定时任务（设置页「定时任务」Tab）

- 查看/启停/切换忙时策略（顺延/跳过）/立即执行/删除由 codex 创建的定时任务，按任务展开执行记录（状态、耗时、结果摘要、错误，可加载更多）并一键打开绑定会话；
- 单次任务完成后归入「已完成」分组。

### Zen 代理

- **Zen 代理（设置 → Zen 代理）**：在 `127.0.0.1` 开放一个 OpenAI Responses API 端点，内部翻译为 Chat Completions 并转发到 OpenCode Zen（`https://opencode.ai/zen/v1`），让 codex 无需 `wire_api="chat"` 即可使用 Zen 免费模型；
- **翻译入口路径 = 模型提供方 base_url 的路径 + `/responses`**（`…/zen/v1` → `/zen/v1/responses`，base_url 无路径 → `/responses`，一律不锁死 `/v1`；
  本地 provider 的 base_url 路径必须与「模型提供方的 base_url」一致，只把 host 换成本机代理），判定不看方法以外的任何猜测；
- **角色映射**：Responses 的 `developer` 角色在 Chat Completions 侧统一降级为 `system`
  （codex 用 `developer` 下发开发者消息，而 DeepSeek 等兼容端点只认 `system`/`user`/`assistant`/`tool`，会直接 400 `unknown variant developer`），其余角色原样透传；
- **除该入口外的所有路径/方法都零路径转换透传**——只把 scheme/host/port 换成上游，入站 path 与 query 原样发出（如上游无路径时 `GET /models` → `{上游}/models`），保留 method/query/body/Authorization 等请求信息并附加 opencode 识别头，非 2xx 原样透传；
- 请求走了翻译还是透传由日志回答：`zen_proxy.passthrough method=… path=… status=…`（2xx info、其余 warn），客户端把 `/responses` 打到非期望路径时另记 `zen_proxy.path_unmatched path=… expected=…`（本地 provider 路径与上游不一致时直接给出期望值）；
- **上游地址末尾带不带 `/` 都兼容**（`https://a/` 与 `https://a` 等价，路由派生与重启判定都按归一化后的值），且**在设置页保存「模型提供方的 base_url」或端口即重启生效**，无需重启 codex-ui；端口默认 `18080` 可在设置页修改，运行状态实时展示；
- Zen base URL 与 User-Agent（`opencode/1.18.29 ai-sdk/provider-utils/4.0.23 runtime/node.js/24`）固定，API Key 不固定——读取客户端请求的 `Authorization` 头原样转发，
  因此在模型配置页为 provider 填 `experimental_bearer_token = "public"` 即可；
- 代理向上游附加 opencode 客户端识别头（`x-opencode-client: desktop`、`x-opencode-project: global`、每条上游请求随机的 `x-opencode-request: msg_*`、
  `x-opencode-session`）——**两者都按真实 opencode 客户端的 ID 规则生成**：`msg_` / `ses_` 之后固定 26 位 `[0-9A-Za-z]`（前 12 位是时间戳低 6 字节的小写十六进制、后 14 位随机，复刻 `sst/opencode` 的 `Identifier.create`）；
  Zen 免费层会校验这个形状（旧写法 `ses_` + 裸 codex 线程 id、18 位随机串会被判成「非 opencode 客户端」并以 `403 FreeTierError: OpenCode's free tier can only be used from within OpenCode` 拒绝）；
  `x-opencode-session` 由客户端请求头 `session-id`（codex 线程 id）**经进程内双向映射表**得到：同一线程在代理生命周期内恒定发同一个 `ses_*`，不同线程互不相同，可反查回线程 id（代理重启后重新分配）；
  请求头缺失、空白或非可见 ASCII 时回落到代理启动时生成、生命周期内稳定的 `ses_*`；
- **请求体形状补丁（免费层门禁的第二道）**：2026-09-18 起，识别头已经合法的请求仍会被以同一条 `403 FreeTierError` 拒绝——门禁搬到了请求体，于是**翻译路径**给发往 Zen 的 `chat/completions` 再补两样东西（透传路径与其它上游一个字段都不动）：
  - 在与 `messages` 同级的位置补 `max_tokens: 32000`——**仅当入站请求没有自己的输出预算**时补（codex 实测不下发 `max_output_tokens`，故真实请求等价于一律补；其它 Responses 客户端显式给了预算就尊重它）；上游若在 4xx 错误体里指名 `max_tokens`，按既有「可选字段」规则摘掉后重试一次（`zen_proxy.optional_fields_dropped fields=max_tokens`）；
  - 往 `tools` **末尾**补 opencode 客户端内置的 6 个工具名（`bash` / `edit` / `glob` / `grep` / `read` / `write`，描述写「这是弃用的工具，请勿调用」、参数为空对象 schema）——只补缺失的名字：真实工具（含命名空间扁平名与自由格式工具）保留在前、顺序不变，同名时以客户端声明为准；模型真去调用这些假工具时代理照常回译成 `function_call`，由 codex 判 `unsupported call`（代理不做特例过滤）；**同步把一段“弃用工具声明”教学并入发往上游的 `messages`**（首条是 system 消息就追加到它末尾，没有就另起一条 system 消息放在最前；幂等，已有则不重复），只列举本次**实际被追加**的工具名——客户端已声明的同名工具不进教学，教学与 description 形成双重约束。**教学只能写 `messages`**：上游是 chat/completions，没有 `instructions` 字段，入站的 Responses `instructions` 在翻译阶段就已经变成那条 system 消息；
  - **生效范围只看上游 host，且对上面两项一视同仁**：上游 host（小写化后）含有 `opencode` 时才补 `max_tokens` 与这 6 个工具名（同一个开关，做不到只开其中一项），把 base_url 换成自建/第三方兼容端点（DeepSeek 等）时两项都自动关闭；**客户端自己**通过 `max_output_tokens` 指定的预算不受此限——那是既有的标量映射，照旧发往任意上游；逐请求可在 `zen_proxy.forward` 行的 `zen_body=on|off` 核对，补丁后的实发请求体见 `logs/zen/*.request.json`；
  - 代价：每个请求多约数百字节的恒定工具声明（位置固定在尾部，可被上游缓存）与一小段动态教学文本；无工具声明的后台请求（会话标题、压缩/摘要）同样会看到这些工具，弱模型存在误调假工具的残余风险——所以有工具 description + system 消息教学双重劝退；
- **流式健壮性**：
  - ① 上游返回的工具调用参数被截断/非法（或工具名为空）时，整轮**不发出任何 function_call**、改发 `response.failed`（codex 以明确失败结束，不再静默按完成收尾），并记 `zen_proxy.malformed_tool_call`；
  - ② 上游随流下发的 `error` 与读取中断同样转成 `response.failed`，不再"按完成收尾"；
  - ③ 回放历史时自动净化——非法参数改写为 `{}`、有工具调用却没有结果时补一条合成 `tool` 消息（`zen_proxy.history_repaired`），使已中毒的会话无需重启即可继续；
  - ④ 流式请求默认带 `stream_options.include_usage` 取回 token 用量并映射进 `response.completed.usage`，
    **明细一并翻译**（`prompt_tokens_details.cached_tokens` → `input_tokens_details.cached_tokens`、
    `cache_write_tokens` → `input_tokens_details.cache_write_tokens`、
    `completion_tokens_details.reasoning_tokens` → `output_tokens_details.reasoning_tokens`，
    并兼容顶层 `cache_read_input_tokens` / `cache_creation_input_tokens` / `reasoning_tokens`）；
  - **收尾顺序**：`finish_reason` 只做记录，须**等到尾部分片**（`include_usage` 的 usage 就是 finish_reason 之后的独立分片，提前收尾会把它丢掉）再发 `response.completed`，最多等 3 秒宽限、超时记 `zen_proxy.usage_timeout` 并按现状收尾，不会挂住回合；
  - 每次收到 usage 另记 `zen_proxy.usage`（截断的原始用量），`zen_proxy.request` 记 `input_chars` / `instructions_chars`、
    收尾记 `zen_proxy.stream_summary`（`finish_reason` / `reasoning_chars` / `text_chars` / `call_count` / `failed` /
    **`usage=present|none`** / **`delta_keys=…`**）——`usage` 说明上游是否真的上报用量、`delta_keys` 说明推理以什么字段名到达，
    便于判断是映射问题还是上游没给；
  - ⑤ **可选请求字段**：`reasoning.effort` → `reasoning_effort`，`parallel_tool_calls` / `prompt_cache_key` / `service_tier` 同名转发，
    `tool_choice` 转换字符串与 `{type:"function",name}` 两种形态；
    上游以 4xx **指名**拒绝其中某字段时自动摘掉重试一次（记 `zen_proxy.optional_fields_dropped`），
    未指名任何可选字段则照旧直接报错、不重复请求；
  - ⑥ **图片透传**：消息里的 `input_image` 翻成 chat 多模态 parts（`{type:"image_url",image_url:{url}}`，`detail` 仅透传 auto/low/high），纯文本消息仍用字符串 content；`input_file` / `input_audio` 暂不透传；
  - ⑦ **思考过程**：上游 `reasoning_content`（DeepSeek 系）、`reasoning`（字符串，或带 `content` / `text` / `summary` 的对象）、
    `reasoning_details`（数组项的 `text` / `summary`，跳过加密项）增量翻成
    `response.reasoning_summary_part.added|done` 与 `response.reasoning_summary_text.delta|done`；
  - **前提是让 codex 真的去请求推理**——在「模型配置」把该提供方的 `model_reasoning_effort` 设为非 `none`（如 `medium`）、`model_reasoning_summary` 设为 `auto`，否则目录里 `supported_reasoning_levels` 为空的模型（Zen 专有 ID 常见）不会被请求，卡片自然为空；
  - ⑧ **截断语义**：`finish_reason = "length"` 以 `response.incomplete`（`incomplete_details.reason = "max_output_tokens"`）结束而非假装 completed，失败仍优先发 `response.failed`；
  - ⑨ **思考模式历史回传**：codex 会在 `input` 里回放 `reasoning` item（其 `summary[].text` 就是上一条流里翻出去的思维链原文），
    代理据此给该轮的 assistant 消息补 `reasoning_content`——**带 `tool_calls` 的和带 `content` 的（codex 记的"进度文本"）都要补**：
    上游声明 tools 时（DeepSeek 实测）校验会一路查到那条文本消息，只补工具调用那条不够；
    该轮没有回放思维链时补空串（实测同样接受）。
  - 是否回传由**学习式开关**决定——上游第一次以"缺 `reasoning_content`"报错时自动打开并内部重试一次（记 `zen_proxy.reasoning_content_enabled`），开关粘滞到本次代理实例结束，且**一次请求内最多翻转一次**（上游若反过来不接受该字段则自动关闭，记 `zen_proxy.reasoning_content_disabled`）；
  - 失败尝试逐条记 `zen_proxy.forward_attempt`（`attempt`/`status`/错误原文 + `messages`/`assistant_with_tools_rc`/`assistant_with_content_rc` 形态统计），`zen_proxy.forward` 另有 `reasoning_rc=on|off` 与 `zen_body=on|off`（本次是否按 Zen 形状打了请求体补丁）供逐请求核对；
  - 启用后在「模型配置」页手动添加 provider：
    `base_url = http://127.0.0.1:{端口}{模型提供方 base_url 的路径}`
    （默认上游是 `https://opencode.ai/zen/v1`，故填 `http://127.0.0.1:{端口}/zen/v1`；
    上游填 DeepSeek 官方地址这类无路径的地址时直接填 `http://127.0.0.1:{端口}`）、
    `wire_api = "responses"`、`experimental_bearer_token = "public"`，模型名填 Zen 支持的 ID（原样透传）；

#### Zen 代理：命名空间工具翻译

- **Zen 代理的工具声明翻译（命名空间工具）**：codex 用 `{"type":"namespace","name":"codexui","tools":[…]}` 声明分组工具（`codexui` 动态工具与多智能体分组都是这种形态），而 Chat Completions 没有命名空间概念、codex 也只认「子工具名 + 命名空间」的调用。
- 代理据此把命名空间**展开成 `{命名空间}_{子工具}` 扁平函数**发给上游
  （说明与参数取自子工具；扁平名重复时只保留一个），回译模型调用时还原成 codex 期望的
  `{"type":"function_call","name":"add_scheduled_task","namespace":"codexui"}`
  （`response.output_item.added` 与 `done` 两处一致），历史回放里的命名空间调用同样按扁平名发出。
- 修复前命名空间被拍平成一个无参数的同名函数（如 `codexui`），模型只能瞎调它并被 codex 以 `unsupported call: codexui` 拒绝——这正是「创建定时任务」在 Zen 代理下跑不通、模型转而翻文档，最后「说要查文档却没有工具调用」直接收尾（回合提前结束）的起因。

#### Zen 代理：自由格式工具翻译

- **Zen 代理的工具声明翻译（自由格式工具）**：codex 对 `apply_patch_tool_type="freeform"` 的模型声明
  `{"type":"custom","name":"apply_patch","description":"…FREEFORM…","format":{grammar…}}`
  （补丁文本是自由格式，不包 JSON），而 Chat Completions 只有函数调用。
- 代理把它**暴露成单参数函数**（`input` 字符串必填；
  描述里的 FREEFORM/「do not wrap in JSON」措辞会被替换为
  「Edit files (apply_patch patch language).」+「This channel is a function call: put the full patch text in the JSON "input" field.」，
  `format.grammar` 不下发），回译模型调用时发出完整的 `custom_tool_call` 事件序列
  （`response.output_item.added` → `response.custom_tool_call_input.delta` → `.done` →
  `response.output_item.done`，`input` 取 `{"input":…}` 字段或补丁原文），非流式输出 `custom_tool_call` item；
- 历史里的 `custom_tool_call` / `custom_tool_call_output` 会还原成 assistant 的 function_call 与 `role:"tool"` 消息（此前被整段丢弃）。
- **补丁语法说明（模型可见文案统一英文）**：codex 的补丁语法只存在于 `format.definition`（lark grammar）里，
  description 本身只有「可编辑文件 + 别包 JSON」两句，而 Chat Completions 没有语法约束能力——
  grammar 被丢掉后上游模型（实测 mimo 系）根本不知道 `@@` 的语义，会自创 `@@ 中文描述 @@` 形态的 hunk 头，
  被 codex 判 `apply_patch verification failed: Failed to find context '… @@'` 后反复换写法重试，
  最后回退到 PowerShell 字符串替换（实测 09-14 一条 mimo 会话：16 次 apply_patch、5 次失败、203 次 exec_command）。
- 现在描述里**补上从 grammar 提炼的英文语法规范**
  （骨架 `*** Begin Patch` → `*** Update File:` → `@@ <原文锚点行>` → 上下文/`-`/`+` 行 →
  `*** End Patch`，文件块 `Add File` / `Delete File` / `Move to` / `End of File`，
  以及「hunk 头只能是 `@@` 或文件里逐字存在的一行，禁止 `@@ 描述 @@`；上下文行必须逐字复制、含缩进」），
  并**在历史里出现补丁失败时追加一条英文纠错提示**
  （只对这个历史里的自定义工具调用生效，且输出命中
  `apply_patch verification failed` / `Failed to find context` / `Invalid patch` / `Invalid Context`；
  只改写发往上游的那条工具结果，codex 侧记录与 rollout 不变）。
- 诊断：`zen_proxy.request` 增 `patch_failures=<n>`（本次历史里已失败的补丁调用条数），`zen_proxy.stream_summary` 同名字段，`suspicious` 新增 `patch_retry`。
- **实测**：走 `function_call` 形态会被 codex 判 `Fatal error: tool apply_patch invoked with incompatible payload`，
  `custom_tool_call` 形态可正常执行补丁。非 JSON 参数按补丁原文接受（空内容仍判畸形）；
  `type:"web_search"` 内建联网工具仍不暴露给上游（代理无法代执行）。

#### Zen 代理：口嗨检测与自动续跑

- **Zen 代理的口嗨检测与自动续跑**：弱模型（Zen 免费模型尤其明显）常以「接下来我会…/Now update the test …」这类承诺句结束回合、却一次工具都没调用，codex 于是把回合当完成收尾，表现为 agent loop 提前退出。
- 代理在**流式翻译路径**里补了一道终局检查：当一个上游响应以「**纯文本 + 零工具调用 + `finish_reason=stop`**」结束时（正是 codex 会就此收尾的那一刻），按「请求来源 + 协作模式」分流处理。
- **默认模式：判定搬进原对话，只认模型自己回的 `<zen_task_completed>` 标签**（不再有独立的后台判定请求——那条 `chat/completions` 会被 Zen 免费层以 `FreeTierError: OpenCode's free tier can only be used from within OpenCode` 拒掉，判定退化成「拿不准按已完成」）：
  代理把提醒作为**合成 user 消息**追加到发给上游的历史里（历史尾部先补一条本轮助手文本，再补提醒），并**再打一次上游，把第二轮的事件续在同一个 SSE 流**上——工具调用照常下发给 codex，`response.completed` 只发一次，对 codex 与应用完全透明，注入的提醒**不会进入 codex 记录**（聊天里看不到任何伪造消息）。
  提醒是**两条路径强收尾**（编号给出，模型只需二选一）：① **仍有可执行的剩余工作** → **必须实际调用工具完成剩余工作**，不得用文字描述、计划或承诺代替工具调用，也不得直接甩出完成标签逃避执行；② **任务已结束、无需继续或无法继续**（已完成 / 无需改动 / 用户已放弃 / 确实做不下去）→ 不复述上一条回复正文、不重复此前已执行过的工具操作（例如已 git commit/push 的，不得再次提交/推送），**整轮只回一行 `<zen_task_completed>已完成</zen_task_completed>`**（标签里只写这四种短词之一；标签外不得有任何其他字符——收尾轮再补正文只会是噪音，上一轮正文已经展示过）。
  终局文本出现该标签（大小写不敏感 + 前缀匹配，缺闭合标签同样命中；**不解析载荷**）即视为任务已终结、直接收尾（`nudge_skipped 原因=task_completed 模式=default`），不再注入、不触发后续 pass。
  **命名与结构不变量**：自研标签统一 `zen_` 前缀——本标签固定 `<zen_task_completed>`，计划模式的两个出口是 `<zen_plan_cancelled>` / `<zen_plan_unachievable>`；`<proposed_plan>` 是唯一例外（codex 侧约定，改名后 codex 不再生成计划条目）。必须成对闭合、独占一行、不换行、一整轮最多一个、不放进代码块、不改写标签。**「标签外不得有任何其他字符」只在催办提醒里**：催办轮上一轮正文已经展示过，收尾就该只有这一行标签；首轮教学契约不写这一条（否则会把模型的结论正文一起禁掉），只要求**「结论正文写在标签之前，标签那一行不得再有任何其他字符」**。**载荷是固定短词**（已完成／无需改动／已放弃／做不下去；计划模式出口分别是已放弃／无法计划），只用于日志阅读，不参与剥离、也不供分桶。**这三个标签会被代理按结构剥离**（见下条），所以聊天与 codex 会话历史里都看不到它们。**旧的无前缀写法（`<task_completed>` / `<cancelled_plan>` / `<unachievable_plan>`）一律不再被识别**（改名当轮若命中旧写法不会被当结束，模型会被催办一次、按新标签自愈）。
  **行为收紧（有意）**：默认模式下「给出方案 / 等你确认」不再被算作完成——模型要么动手调工具，要么把「已给方案、等你确认」写进标签自己宣告结束。
- **计划模式**下**连判定都不发，本轮怎么收尾完全由代码判据给出**（模式怎么判定见下面四条）：
  - 终局文本含 `<proposed_plan>` → 视为计划已交付、直接收尾（`nudge_skipped 原因=plan_output 模式=plan`）；
  - 终局文本含 **`<zen_plan_cancelled>已放弃</zen_plan_cancelled>`**（用户中途放弃计划的约定标签，提示词里教给模型；标签里只写「已放弃」）→ 视为计划话题已终结、直接收尾（`nudge_skipped 原因=plan_cancelled 模式=plan`）——不再催它给一份已被放弃的方案，**也不生成计划条目、不弹「计划已就绪」**；标签本身会被剥离，聊天里只剩模型那句结论。判定与计划标签同口径（大小写不敏感 + 前缀匹配），计划标签优先；
  - 终局文本含 **`<zen_plan_unachievable>无法计划</zen_plan_unachievable>`**（问题本身无法或无需产出实现计划的约定标签，如「1+1=？」这类事实问题、纯查询或闲聊，提示词里教给模型；标签里只写「无法计划」）→ 视为计划话题已终结、直接收尾（`nudge_skipped 原因=plan_unachievable 模式=plan`）——不再逼它在计划模式里强行给一份实现方案，也不生成计划条目、不弹「计划已就绪」。判定与别的计划标签同口径（大小写不敏感 + 前缀匹配）。
  - 三者都没有 → 视为未交付，注入**计划专用提醒**并续跑，同样受单请求催办次数上限约束——不会再注入「必须动手」逼模型在计划模式里改代码。
    提醒是**排除式三选一**：先让模型判断「用户已放弃」（`<zen_plan_cancelled>`）与「问题无法/无需产出实现计划」（`<zen_plan_unachievable>`）两种不需要给方案的情况是否成立，都不成立才**必须**把完整方案写进 **`<proposed_plan>` / `</proposed_plan>` 骨架**（`# 计划标题` + 步骤项，并要求标签原样保留、成对闭合、各自独占一行、不换行、本轮最多只写一个、不要放进代码块、不要改写标签）；即便结论是「无需改动、保持现状或原计划已认可」，也要把该结论（或重申原计划）写进结构里收尾——那是评估结论、不是「无法/无需计划」。弱模型常把计划写成普通 Markdown，而 codex 只在终局文本出现该包裹时才生成计划条目（应用里才有「计划已就绪」），骨架式提醒能显著提高交付率。
- **首轮教学（两种模式都有）**：代理在首次转发之前，把**契约**追加到入站 Responses 请求的 `instructions` 末尾（原提示词逐字保留、契约在末尾，对上游 prompt cache 友好；只改 `instructions`、不动 `input`，所以协作模式判据与 `模式来源` 不受影响）。该字段随后在翻译成 Chat Completions 请求时成为 `messages` 里的那条 system 消息——上游本身没有 `instructions` 字段，契约正是靠这一步才到达模型，让模型第一轮就按约定收尾：
  - 默认模式：`【回合收尾约定】…任务已全部完成就把结论写在正文里，然后用一行 <zen_task_completed>已完成</zen_task_completed> 收尾——标签里只写这四个词之一：已完成／无需改动／已放弃／做不下去，不要写别的说明；标签必须成对闭合、独占一行、不换行；本轮最多一个；不得放入代码块；不得改写标签；标签那一行不得再有任何其他字符（结论正文写在标签之前）。还有没做完的工作必须实际调用工具继续做，不要只写「接下来我会…」这类承诺；不要重复执行更早回合已经用工具做过的操作。`
  - 计划模式：只前置两个**出口标签**（`<zen_plan_cancelled>` / `<zen_plan_unachievable>`）并写清结构不变量（成对闭合、独占一行、不换行、本轮最多一个、不得放入代码块、不得改写标签、标签那一行不得再有任何其他字符）与边界（「无需改动、保持现状、原计划已认可」属于评估结论、要走 `<proposed_plan>`、不算 unachievable）；**不前置**「必须给完整方案」——计划模式的正常形态是 chat your way，强制口径留在催办提醒里，免得把中间闲聊回合逼出假方案。
  - 门槛与催办同源：**流式 + 请求声明了工具 + 非会话标题线程**才注入（非流式没有催办路径可消费；压缩/摘要类后台请求通常不带工具；标题线程只产出标题）。
  - 收益与代价：守约定的模型首轮就带回标签 → 常规回合**零额外上游往返**（此前每条「纯文本 + 零工具」回复都要多打一轮换标签）；代价是每个合格请求多约 190~200 字提示词。标签本身不出现在聊天里（被剥离），载荷也压成一个短词。
  - 催办提醒里的 **N 选一原样保留**（默认二选一、计划排除式三选一），作为「模型不吃首轮教学」时的第二道。
- **标签剥离（不污染 codex 会话）**：`<zen_task_completed>` / `<zen_plan_cancelled>` / `<zen_plan_unachievable>` 在代理层按**结构**剪掉——「开标签前缀 → 闭标签」，**载荷写什么、写多长、是不是固定短词都不参与匹配**（模型不守约定照删）；`<proposed_plan>`、推理摘要、代理自己发出的教学与提醒都不剥离。实现要点：`TextTrack` 并存原始文本（判据、续跑轮回显、字数统计用）与可见文本（真正下发给 codex），流式增量与 `output_text.done` / `content_part.done` / `output_item.done` 三处终局都用可见文本；半个标签跨分片由 holdback 压住，标签独占一行时连行尾换行一起删，未闭合的标签丢到结尾（与「缺闭合标签也命中」的判据一致），弱模型写错闭标签也容错闭合。非流式翻译路径做同样的一次性剥离。**被删掉的载荷只能从日志回看**：`nudge_skipped 标签内容=`（截断 60 字）。
- **模式判定优先取协议登记表**：`CodexServer` 把 `turn/start` / `thread/settings/update` 参数与 `thread/settings/updated` 通知里的 `collaborationMode.mode` 登记成「线程 id → 模式」，代理用入站请求头 `session-id`（= codex 线程 id，`ses_` 前缀自动剥离）查表——命中即**完全不做关键词扫描**，`模式` 一栏的 `模式来源=registry` 说明来自登记表；查不到（其它客户端、子代理线程等）才退回关键词判据，且判据取**最后一个**协作模式块的标题行（`模式来源=heuristic`）。登记表只存内存、上限 1024 条（超出整体清空，模式每轮 `turn/start` 都会重新登记）。
- 关键词兜底判据（`模式来源=heuristic`）从协作模式块标签 `<collaboration_mode>` 之后取**第一个非空行**（codex 0.154 是 `# Plan Mode (Conversational)`，旧版 `# Collaboration Mode: Plan` 仍兼容），且**只认标题行**：默认模式文案的标题是 `# Collaboration Mode: Default`，正文第一句却写着「… for other modes (**e.g. Plan mode**) are no longer active.」——按子串匹配会把默认模式误判成计划模式（默认模式的正常编码回合就会被注入「请给出计划」）。
- 为什么必须以最后一块为准：codex 会把**历次**协作模式块都留在请求历史里，切回默认模式后历史里仍有旧的 `<collaboration_mode># Plan Mode…</collaboration_mode>`——按「任一命中即计划模式」必然误判（实测某线程 codex 侧记为 `collaboration_mode_kind=default` 的回合，代理仍按 plan 分流并注入「请给出计划」）。
- **会话标题生成请求**（应用 `autoTitleThread` 的后台临时线程，末条 user 文本以固定前缀「给下面用户消息生成一个不超过 30 字的中文会话标题」开头）**整轮放行**：不催办、不注入（`nudge_skipped 原因=title_task`）——标题必然「纯文本 + 零工具调用」结束，注入只会白花一轮上游。
- 边界与开关：**单次入站请求最多注入 4 次续跑提醒**（= 最多 5 次上游调用；`NUDGE_MAX_INJECTIONS` 是唯一旋钮，原来的「同一会话连续 2 次 / 10 分钟窗口」跨请求计数与「最多 4 个 pass」兜底都已删除，因为循环内每次迭代要么收尾要么注入，两者本就等价）；跨请求**不再有任何静默期**——每个「纯文本 + 零工具 + 无收尾标签」的终局都会被催。请求里没有声明任何工具时既不催办也不注入契约；**错误/失败路径、历史净化与工具翻译逻辑均不受影响**。
- 诊断日志（**nudge 家族的键名是中文、值保留英文 token**，事件名保留英文便于 grep）：`zen_proxy.request` 的 `模式=plan|default`、`模式来源=registry|heuristic`（本次识别出的协作模式及其来源，排查「默认模式被注入计划提醒」或「默认模式被当成计划模式」先看这两格）与 **`首轮教学=off|default|plan`**（本次是否注入首轮教学契约；`instructions_chars` 记的是**注入后**实际发给上游的长度，便于对照体积变化）、
  `zen_proxy.nudge_injected`（`会话`／`轮次`／`本请求催办次数=1/4`／`模式`／`助手字数`／`说明`）、
  `zen_proxy.nudge_skipped`（`原因`：task_completed／plan_output／plan_cancelled／plan_unachievable／title_task／上游错误/状态码/转发失败，另带 `轮次`／`模式`／可选 `状态`、`详情` 与中文 `说明`；三个标签短路分支还带 **`标签内容=`**——标签已被剥离，这里是回看模型结论的唯一入口）、
  `zen_proxy.nudge_limited`（`原因=max_injections` + `轮次`／`模式`／`说明`）；收尾摘要多一列 `stripped_chars`（原始 − 可见，= 被剥离的标签字符数）；内容级诊断另开请求记录（call_id 带 `-nudge<n>` 后缀，原来的 `-judge<n>` 已随看门狗删除）。
- 未覆盖：不检测「调用过工具但没做对」的情况，也不回溯历史回合；判定完全在注入的那一轮对话里完成，因此不再有「判定期间静默 60 秒」的等待（原看门狗的超时与解析路径已整体删除）。模式登记表不持久化，应用重启后第一个回合由 `turn/start` 重新登记。

#### Zen 代理：指标口径

- **Zen 代理的指标口径**：上游 usage 里的 `cache_write_tokens` 在 OpenAI 兼容端点上恒为 0（上游明确上报该字段，属正常；有效指标是「缓存读取」与「输入缓存命中率」）；
- 「推理输出 / 思考过程」取决于 codex 是否请求推理——把该提供方的 `model_reasoning_effort` 设为非 `none`、`model_reasoning_summary` 设为 `auto`；
- 再用 `zen_proxy.request` 的 `reasoning_effort` 与 `zen_proxy.stream_summary` 的 `delta_keys` 核对（前者是 codex 请求的档位，后者是上游实际下发的推理字段）。

#### Zen 代理：模型报「工具 update_plan 不可用」（codex 侧问题，已核实）

- **现象**：弱模型按 codex 的 base instructions 去调用 `update_plan`，codex 回 `unsupported call: update_plan`，模型遂把它转述成「工具 update_plan 不可用」。**默认模式与计划模式都会出现**。
- **根因（0.154.0 实测抓包）**：请求的 `tools` 数组里**没有** `update_plan`（只有 `exec_command` / `write_stdin` / goal 三件套 / 若干命名空间工具 / `web_search` 等），但同一请求的 `instructions` 里仍有两处承诺它存在（`## Planning` 段与文末 ``## `update_plan` `` 整节），计划模式还多一段 `## Plan Mode vs update_plan tool`——属 codex 侧「提示词承诺了一个未注册的工具」。
- **代理侧不做特例、也无法适配**：代理原样翻译转发客户端的工具声明；codex 的工具注册表在回合开始时即固定，代理无法让 codex 接受一个它没注册的工具（替模型吞掉调用、自行合成工具结果会让计划不再显示在应用里，语义也脏）。同一现象在不走 Zen 的 provider 上同样复现，与 Zen 无关。
- **当前处理**：不改代码 / 提示词，作为已知问题记录；完整取证（mock 抓包命令、rollout 次数统计）与两个未实施的备选方案见 [docs/变更记录.md](docs/变更记录.md) 2026-09-17 条目。

### Zen 代理诊断日志（内容级，默认关闭，用 `CODEXUI_ZEN_TRACE` 开启）

#### 开关与取值

- **开关**：默认**不写**内容日志（避免长期累积占盘）。需要排查时设 `CODEXUI_ZEN_TRACE=1` 并**重启 codex-ui** 才生效（环境变量只在启动时读取一次）：

```powershell
$env:CODEXUI_ZEN_TRACE='1'; .\build-dev.bat     # 或先设置系统环境变量再启动应用
```

- 取值：`1` / `true` / `on` / `yes` 为开启，`0` / `false` / `off` / `no` 为关闭（大小写不敏感）；写错（如 `=abc`）按关闭处理，并在 `session-*.log` 里记一条 `env.flag_invalid` 便于发现。
- 关闭后**已有文件保留、不再清理**，需要时手动删除 `%APPDATA%\com.codexui.app\logs\zen\*`。

#### 日志位置与文件组

- **位置**：`%APPDATA%\com.codexui.app\logs\zen\`（与 `session-*.log` 分目录存放）；**只覆盖翻译路径**（`{模型提供方 base_url 路径}/responses`），`/models` 等透传请求仍只记 `zen_proxy.passthrough` 一行。
- **每次上游尝试一组文件**（含重试的每次 attempt）：
  - `<yyyyMMdd-HHmmssSSS>-<call_id>-a<n>.request.json`（发往上游的 chat/completions 请求体）；
  - `.response.sse`（上游响应原文逐行，保留 `data:` 前缀与 `[DONE]`；非流式为 `.response.json`）；
  - `.response.txt`（上游非 2xx 原文）；
  - `.summary.txt`（收尾摘要）。

#### 摘要字段与可疑标记

- **摘要字段**：`call_id` / `attempt` / `model` / `stream` / `upstream_url` /
  `x_opencode_session`（实发的 opencode 形状会话 id）/ `session_codex`（由双向映射表反查出的 codex 线程 id，
  回落会话记 `-`，两者对照即可定位是哪个线程）/ `x_opencode_request` /
  `authorization=present|absent`（**从不落盘 API Key**）/ `message_count` / `tool_count` /
  `dropped_fields` / `reasoning_rc` / `elapsed_ms` / `upstream_status` / `finish_reason` /
  `text_chars` / `reasoning_chars` / `call_count` / `failed` / `usage` / `delta_keys` /
  `suspicious` / `translated_terminal_event`（发回 codex 的 `response.completed|failed|incomplete` 原文）；
- 流未收尾就被中断时摘要记 `ended=dropped_without_finish`。
- **可疑结束标记** `suspicious`（命中多项逗号连接，无标记为 `-`）：
  - `stop_without_output`（`finish_reason=stop` 且无文本/推理/工具调用——最贴近「任务未完成就结束」）；
  - `stop_without_tool_call`（只输出文本、没有工具调用）；
  - `failed`；
  - `truncated`（`finish_reason=length`）；
  - `finish_reason_missing`；
  - `usage_missing`；
  - `usage_timeout`；
  - `upstream_http_error`。
- 标记只用于筛选，不改变代理行为；`zen_proxy.request` 带 `call_id`、`zen_proxy.stream_summary` 带 `suspicious`，可与内容日志互相定位。

#### 保留与清理

- **保留与上限**：单文件 8MB（超出即停止写入并追加 `[truncated: …]`，摘要记 `truncated=true`）、目录总量 256MB、保留 3 天；按文件名时间戳**整组清理**（无法解析的文件名不删），清理在开始新调用前执行且最多每 60 秒一次。

#### 风险与检索

- **风险提示**：内容日志包含完整提示词、工具参数与输出、代码片段（不含 API Key），分享前请确认。
- **检索可疑调用**（PowerShell）：

```powershell
Select-String -Path "$env:APPDATA\com.codexui.app\logs\zen\*.summary.txt" -Pattern 'suspicious=(?!-)' |
  Select-Object Path, Line
```

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
    └── components/              界面组件（RightPanel + SessionView + ResourceView、EditorPane 多标签编辑区：TextEditorPane/DiffPane/PreviewPane/TerminalPane 等）
```

后端负责 codex 子进程生命周期（断线自动重连）、JSON-RPC 请求/响应分发、服务端通知流式转发
（`item/started`、`item/completed`、`item/agentMessage/delta`、`item/commandExecution/outputDelta`、`interaction:request` 等）与交互响应回写。
前端通过 Tauri invoke/event 通信。

## 环境要求

- Windows 10/11（自带 WebView2）
- Rust stable-msvc（rustup）+ VS C++ 构建工具
- Node.js 18+（仅开发/构建需要；运行端不需要）
- `codex` CLI（可在设置页指定路径；验证基线 codex-cli 0.154.0（兼容 0.149.0+））
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

安装包（Inno Setup）：

```powershell
build-release.bat
```

产物：`setup\output\codex-ui-win-x64.exe`。脚本自动依次执行前端构建（`npm run build`）→ Rust release（`cargo build --release`）→ 把 exe 暂存到 `setup\` → Inno Setup 编译（`setup\setup.iss`）。

> 注意：`Cargo.toml` 中 `tauri` 依赖已启用 `custom-protocol` 与 `protocol-asset` 特性。前者保证生产窗口加载打包的前端（否则会去连 `localhost:5173` 显示“拒绝连接”），后者用于 asset 协议加载本地图片。

> `build-release.bat`（Inno Setup，`setup/setup.iss`）只把 `openai-bundled.tar.gz` 打进安装包（放到 `{app}\marketplaces`），
> 由 codex-ui 启动时用 Rust 原生 tar 后台解压到 `$CODEX_HOME\.tmp\bundled-marketplaces`，使 `openai-bundled` 插件离线可运行；
> `codex-primary-runtime` 不再随包，改由 codex-ui 启动后按需从 CDN 下载 `.tar.gz`（约 478MB、支持断点续传）到 `%USERPROFILE%\.cache\codex-runtimes` 并解压，
> 仅当 `runtime.json` 缺失或其 `bundleVersion` 旧于 `26.819.11345` 才下载。
> 因此安装包体积变小，但首次启用 `openai-primary-runtime` 依赖网络；下载/解压失败时该市场不注册、下次启动重试。

### 用 GitHub Actions 构建 release（推荐）

本机 `cargo build --release` 较慢，日常把这一步交给 CI（`.github/workflows/build.yml`，workflow 名 `build-release`）：

- 推送到 `main` 自动构建（只改 `*.md` / `docs/` 的提交不触发）；产物在 Actions 运行页的 **Artifacts** 里下载，名字是 `codex-ui-exe-<短提交号>`，压缩包内含 `codex-ui.exe` 与 `build-info.txt`（版本、提交、ref、run 号、UTC 时间），保留 90 天；
- 推 `v*` 标签会额外创建同名 GitHub Release，并附件 `codex-ui.exe` 与 `build-info.txt`（标签与 `Cargo.toml` 版本号不一致时只告警，不中断发布）；
- 也可以在 Actions 页面用该 workflow 的 **Run workflow** 手动触发。

CI 只做「前端构建（`npm run build`，含 `vue-tsc --noEmit`）+ Rust release」两件事：**不产出安装包**，也不接触 `setup\bin` 下的 sidecar 二进制。

拿到 CI 产物后，把它拷进 `setup\` 再只打安装包（跳过本机最耗时的 Rust 编译）：

```powershell
build-installer.bat                                          # 用现有的 setup\codex-ui.exe
build-installer.bat $env:USERPROFILE\Downloads\codex-ui.exe  # 指定下载下来的 exe（先拷到 setup\）
```

产物同样是 `setup\output\codex-ui-win-x64.exe`。该脚本只调 Inno Setup（`setup\setup.iss`），要求 `setup\codex-ui.exe`、`setup\bin`、`setup\marketplaces` 已就位；`setup\codex-ui.exe` 缺失时脚本会提示从 Actions 下载 artifact 或改跑 `build-release.bat`。

> exe 内的版本号来自 `src-tauri/Cargo.toml`（与 `tauri.conf.json` 中的 `version` 一致），安装包版本号由 Inno Setup 从 exe 自身读取，与 tag 无关。

## 测试

```powershell
npm test                                   # 前端单元测试（vitest，等价 npm run test:unit）
npm run test:typecheck                     # vue-tsc 类型检查
npm run test:coverage                      # 前端单测 + 覆盖率（v8；门槛 lines≥80 / functions≥75 / statements≥75 / branches≥70）
npm run test:rust                          # Rust 单元测试（cargo test --lib）
$env:CODEX_BIN='codex'; npm run test:rust:integration
                                           # Rust 真实 app-server 集成测试（握手/回合、置顶、目标全生命周期、记忆模式、线程设置同步、回合列表 full、会话搜索）
                                           # CODEX_BIN 可填裸命令名（npm 的 .cmd/.ps1 shim 会自动解析为真实 codex.exe）
                                           # 或 codex.exe 完整路径；未设置 CODEX_BIN 时集成用例自动跳过
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
- 附着运行中的应用（需先以 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 启动，支持 `--port` 与 `CODEX_E2E_PORT`）：
  `verify-approval.mjs`（审批，含“批准并记住此规则”，需 `CODEX_E2E_TARGET`）、`check-mode-lock.mjs`（回合中模式按钮禁用）。
- 协议级探针（直连 `codex app-server --stdio`）：`probe-approval*.mjs`。

E2E 探针自建临时目录与会话，结束时自动清理；CDP 端口被占用或应用启动失败时给出明确诊断，可用 `CODEX_E2E_PORT` 换端口。

## 环境变量（`CODEXUI_*`）

codex-ui 自有环境变量统一使用 **`CODEXUI_` 前缀**，后接大写下划线的「组件_用途」功能段（如 `CODEXUI_ZEN_TRACE`；将来新增的 `CODEXUI_ZEN_TRACE_MAX_MB`、`CODEXUI_WECHAT_DEBUG` 等沿用同一前缀）。约定如下：

- **取值**：大小写不敏感、自动 trim；真值 `1` / `true` / `on` / `yes`，假值 `0` / `false` / `off` / `no`；未设置或空串按「关」处理。
- **生效时机**：进程启动时读取一次，改完需重启 codex-ui。
- **写错的值**：既不是真值也不是假值（如 `abc`）按「关」处理，同时往 `%APPDATA%\com.codexui.app\logs\session-<日期>.log` 写一条 `env.flag_invalid`，字段含变量名与原始取值。

| 变量 | 默认 | 作用 |
|---|---|---|
| `CODEXUI_ZEN_TRACE` | 关 | 是否把 Zen 代理的内容诊断日志（请求体/上游响应原文/收尾事件）写到 `logs\zen\`，见「Zen 代理诊断日志」小节 |

> 说明：`CODEX_BIN`（Rust 集成测试的 codex 路径）、`CODEX_E2E_PORT`（E2E 端口）等属于本仓库自有的**开发/测试**变量（不带前缀，供脚本与 CI 使用），与上面的运行期开关是两套。

## 使用说明

- **工作目录**：应用内工作区以当前会话/编辑器标签的目录为准，**不再把启动目录当作默认工作区**；未打开/创建任何会话时工作区为空，资源/Git 面板显示「暂无工作目录」、不监听目录。点击头部「新建会话」先弹文件夹选择器选定本次会话目录（初始定位为当前工作区，没有则最近一次、再无则空），取消则流程结束。历史会话显示各自记录的目录，继续执行用记录目录。
- **多会话标签**：标签栏末尾「+」菜单可新建空会话（使用活动标签的工作区，无则空）、打开文件（系统文件选择器多选，文本/预览类型应用内打开，不支持的类型提示无法打开；初始定位为当前工作区）或新建终端（无确定工作区时回退到应用自身目录）；头部「+」先选目录再新建。会话标签可关闭，允许关闭到 0 个（主区域显示空状态提示）。多开会话时**文件拖放仅当前活动会话的输入框接收为附件**（隐藏标签输入区不响应），拖到其它任意位置仍按「打开文件」处理。
- **会话列表**：面板顶部即搜索框；按目录分组（目录按名称 A-Z、会话按置顶+时间倒序），文件夹默认收起，支持全量加载、重命名、**分叉会话**（把源会话完整复制为新线程、打开新标签并沿用源名称，可在此基础上另起分支继续）、固定置顶、删除确认。
  点击会话行：未打开则新标签打开、已打开则聚焦对应标签（点击当前会话不重载）；已打开会话行显示「已打开」标记、后台运行中显示呼吸点；切换标签不中断后台回合；关闭窗口时若有工作会话/终端会先确认。
  **打开无活跃目标的历史会话即恢复线程**，右上角立即显示该会话累计输入/输出 token 用量；
  **带活跃目标**的会话仍只读，发消息才恢复（服务端围绕目标自动续跑，点击目标旗子旁的 × 清除目标即停止）；
  新会话首条消息会自动生成短标题。
- **会话资源**：右侧面板底部切换到“资源”Tab；根为**活动标签的工作区**（会话标签→会话工作目录；文件/diff/预览/终端标签→各自打开时的工作区），根节点显示文件夹名、默认展开第一层；
  `.git`/`.codegraph` 等点目录不显示；文件/目录右键管理，文件变化自动刷新；
  头部搜索框同时按文件名/目录名与文件内容（捆绑 `rg.exe` 可用时）检索，内容命中显示行号摘要；切换标签后根随之切换。
- **权限模式**：只读访问（文件只读、不联网，不会修改任何文件）；请求批准允许联网、联网与外部写操作均会询问审批；帮我批准允许联网、仅对检测到的风险操作请求批准（自动评审）；完全访问不受限。权限/模型/推理强度为**会话级**，每个会话标签独立配置，新会话取默认（模型未显式选择时由模型列表默认值决定）。
- **设置文件**：`%APPDATA%\com.codexui.app\settings.json`，仅保存 codex 路径、Enter 快捷发送、跟进处理方式、
  两个系统通知开关（错误 `error_notify_enabled` / 提权·交互·完成 `interaction_notify_enabled`）、主题、毛玻璃特效开关、权限模式初始值、记忆模式、微信接入总开关与最后活跃会话 id（`last_session_id`，供下次启动恢复）。
- **登录**：界面不提供登录入口，请使用其它入口（如 `codex login` 或 API Key）完成认证。

## 微信接入（ClawBot · 实验性）

经**微信官方 ClawBot 通道**（ilink bot API）把你的 Codex 带进微信聊天——在历史会话上右键「微信接入」扫码绑定后，直接在 ClawBot 联系人里发消息，就会驱动该会话中的 Codex 完成回合，并把回复以绑定的账号被动发回，实现「人在外面、Codex 在家里干活」的远控体验。

### 启用步骤

1. 在会话列表面板右键目标会话 →「微信接入」；
2. 点击「扫码绑定」并扫描二维码 —— 完成。门禁为**谁扫谁白**：只有扫码绑定的这个微信号本人发来的消息会交给该会话，其他联系人的消息一律忽略并记录，无需填写任何 ID。

### 行为约定与限制

- **按会话绑定**：一个会话最多绑定一个微信账号，一个微信账号也只能绑定一个会话；支持多个会话各自绑定不同微信账号。删除已绑定会话会自动解除绑定并停止该账号接收。
- **文本、图片、文件与视频**：文本消息直接驱动回合；
  图片 / 文件 / 视频按微信 CDN 协议**流式**下载并 AES-128-ECB 解密（全程不整体读入内存、不限制单个附件大小，下载总时长上限 30 分钟），落到应用数据目录 `%APPDATA%\com.codexui.app\wechat\wechannel-data\media\<会话id>\`（扁平混放、永久保留、不自动清理，需要时自行清理该目录）；
  **只有已绑定会话的账号**才会下载附件，未绑定账号发来的附件直接跳过。
  交给 Codex 分两种方式：**图片**按魔数推断扩展名、以 `localImage` 项发送（codex 会把它包装成 `<image name=… path=…>` + `input_image`）；
  **文件与视频**（文件保留微信原文件名、视频用 `wechat-video-<时间戳>-<序号>.mp4`）改由**文本段承载**——
  `# Files mentioned by the user:` 下每行 `## 文件名: 路径`，用户文字接在 `## My request:` 之后，会话里这段会渲染成 `@文件名` 引用芯片（改用独立 `mention` 项会被 codex 整个丢弃、整条消息变成空消息，故不采用）。
  与文字同一条消息时一起发送；只发附件、没有文字时，`## My request:` 段自动补一句「（用户只发送了附件，没有文字说明，请查看附件内容）」。
  条数上限：单条消息图片 4 张 / 文件 4 个 / 视频 2 个；接收失败会在微信里回一条提示；语音仍只取转写文本（语音文件本身不落盘）。
  回复为最终 agent 文本，超长自动分段（约 1800 字/条）。
- **回复保留 Markdown 原文**：Codex 回复按原始 Markdown 直接发送（协议由 Rust 内置实现，不做任何剥离），由微信端/ClawBot 侧负责渲染；忙碌/失败等系统提示仍为纯文本。
- **回复兜底**：若回合正常结束却没捕获到助手文本（通知订阅偶发丢消息），会自动回查该回合的最终 `agentMessage` 文本作为回复；完成信号按「线程 + 回合 id」精确匹配，避免同线程残留的旧完成信号（后台压缩/重连遗留）误判新回合导致误报无输出；确无文本才返回中性提示（不再误报「执行失败：回合异常结束」）。
- **被动回复窗口**：平台要求回复携带收到消息时的 `contextToken`，且 24 小时有效——只能回复来过消息的联系人，无法主动推送；过期后需要对方再发一条新消息重新开启窗口。
- **「对方正在输入」指示**：收到消息进入回合即向 ClawBot 返回正在输入状态（`sendtyping status=1`），回合期间每 8 秒续发保持可见，回复送达/结束/失败/超时后取消（`status=2`）；全程 best-effort，失败仅记日志不影响回复。
- **免审批完整能力 + 本人门禁**：微信触发的回合固定使用「完全访问 + 免审批」（文件与网络不受限）；由于门禁只放行扫码绑定账号本人，请勿将该微信出借他人使用。
- **在线范围**：bot 仅在 codex-ui 运行期间在线；窗口关闭即停止长轮询并使 bot 离线。绑定会话意外退出时会静默重启一次并恢复已绑定账号的接收。
- **凭据存储**：登录 token 由应用保存在 `%APPDATA%\com.codexui.app\wechat\wechannel-data\`（应用数据目录内）；`会话 ↔ 微信账号` 绑定存于同目录 `bindings.json`。
  该目录下的 `media\<会话id>\` 存放微信发来的图片 / 文件 / 视频；旧版按 `media\<年-月>\` 落盘的文件保留原位，程序不再写入也不再读取。
- **排队规则**：同一联系人的消息串行执行（FIFO，上限 16 条），溢出时回一条忙碌提示；单回合最长等待 10 分钟，超时按失败回复。

### 排查提示

若绑定后聊天无响应而绑定弹窗状态已显示「已连接」，查看应用日志中「忽略非绑定消息：<ID>」——如该 ID 就是你的使用账号（个别版本平台可能不回传统一 userId），属协议行为差异；可在 GitHub 反馈补充场景信息。

微信协议由 Rust 内置实现（`wechat_client`），无需 Node sidecar 或额外运行时。

## 协议与版本支持

### 版本兼容与协议核对

当前版本**验证基线 codex-cli 0.154.0**（兼容 0.149.0+）。应用启动时会探测 `codex --version`：
版本**低于 0.149.0** 时在状态栏提示“未适配”、但照常运行；0.149.0 及更高版本不提示。
高版本不警告意味着未来 codex 协议变化时应用可能静默异常，升级 codex 后请留意功能是否正常。
0.154.0 相对 0.149.0 的协议差异核对见 [docs/app-server.md](docs/app-server.md) §1.1
（`ServerRequest` 无变化，新增方法与通知均为可选、不使用即不受影响）。
**已知不一致（0.154.0）**：base instructions 仍教模型使用 `update_plan`，但请求的 `tools` 里已无该工具，模型调用会被 codex 拒为 `unsupported call: update_plan`（表现为模型说「工具 update_plan 不可用」，默认/计划模式皆然）；不影响应用功能，详见「Zen 代理」小节的同名条目与 `docs/变更记录.md`。

以 `codex app-server generate-ts --experimental` 输出的协议绑定为参考。后端只实现本项目所需字段，未知通知忽略并记录日志。codex CLI 升级后如协议变化，可重新生成绑定核对：

```powershell
codex app-server generate-ts --out <dir> --experimental
```

### 错误与警告日志

`error`/`warning` 通知统一写入 `%APPDATA%\com.codexui.app\logs\codex-YYYY-MM-DD.log`（保留 7 天）；DEBUG 构建下两者都弹 toast，Release 构建下仅 `error` 弹，`warning` 只落盘不提示。

### 会话系统通知

**会话的系统通知**（设置 → 个性化，两项均默认开启；应用内提示行为一律不变，只有**主窗口没有前台焦点时**——切到别的程序、最小化、隐藏到托盘——才发 Windows 通知）：

- **会话错误时发系统通知**（`error_notify_enabled`）：把 codex 产生的 error 级内容发通知——来源包括 `codex/message` 的 error 通知、会话内 error 条目、回合失败（`turn/completed` failed）与发送/续跑回合失败；
  标题「会话错误 · <会话名>」（取不到会话名时仅「会话错误」），正文为友好中文错误文本并截断。
  拿不到会话（无 threadId）的错误不发；
  节流为「同一错误正文 10 秒内只发一条」（`Reconnecting… 1/5`…`5/5` 这类同文连报收敛为一条）加「同一会话回合 2 秒内只发一条」（压制同一次失败的 error 通知 + turn failed 双报，窗口取短以免压掉同回合内稍后出现的真正错误）。
- **会话提权/交互/完成时发系统通知**（`interaction_notify_enabled`）：审批（命令/文件变更/权限）、提问、MCP 表单与「计划已就绪」各发一条，标题分别是「需要审批 / 需要输入 / MCP 表单 / 计划已就绪 · <会话名>」，正文为对应的通用说明（不含命令与问题原文）；**不去重**，每条交互请求都发，避免漏掉需要立即处理的确认。
- **会话正常完成时发系统通知**（同一开关 `interaction_notify_enabled`）：`turn/completed` 且状态为正常完成（`completed`）时发一条「会话完成 · <会话名>」，正文「会话已完成，可以查看结果」；中断（`interrupted`）与失败（`failed`）不发（失败已由会话错误通知覆盖，避免同一次结束双报）；计划模式的回合若产出计划则只发「计划已就绪」，不叠加发「会话完成」；拿不到会话（无 threadId）与后台临时线程不发。

两类通知都带「打开会话」按钮，点击聚焦窗口并打开对应会话；均在带消息泵的主线程按 AUMID 归属显示（开发版回退 PowerShell，安装版用 `com.codexui.app`），与定时任务通知同一套实现。

### 置顶与标题自动总结

**置顶**（0.149.x 固定协议）：`threadSection/list` 定位内置 `Pinned` 分区 → `thread/section/move { sectionId }` 置顶 / `{ sectionId: null }` 取消。`threadSection/list` 失败时回退内置 Pinned 分区常量 id。

**标题自动总结**（0.149.x 固定协议）：新建会话后恒用 ephemeral 临时线程总结首条消息并立即注销，失败静默保留默认标题。

## 常见问题

- **启动后“localhost 拒绝连接”**：生产构建缺少 `tauri/custom-protocol` 特性（或前端未构建）。确认 `Cargo.toml` 已启用该特性并重新 `cargo build --release`。
- **历史显示全部而非当前目录**：当前版本按需求设计为展示全部会话并标注目录；如仍异常，确认运行的是最新构建（旧版存在初始化时序 bug）。
- **点击历史后发送/停止按钮反复交替**：该会话带活跃目标且被自动恢复。新版打开会话为只读、不会触发；如已发生，清掉该会话目标或删除会话。
- **“批准并记住此规则”后仍被拒**：确认权限模式为“请求批准/帮我批准”（沙箱为 workspace-write）；若选了“只读访问”（沙箱为 read-only）则无法写入文件属预期（早期版本误用 read-only 沙箱导致该问题）。
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
