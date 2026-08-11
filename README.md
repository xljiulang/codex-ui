# codex-ui

基于 **Rust + Tauri 2** 的 Codex CLI Windows 桌面客户端。`codex-ui.exe` 是对 `codex` CLI 的桌面包装：启动后拉起 `codex app-server --stdio`，用与 VS Code Codex 插件相同的 JSON-RPC 协议驱动 codex，提供完整的中文桌面界面。

> `codex-ui.exe` 启动时所在目录即 codex 的工作目录；历史会话按目录记录，点击任一历史会话会使用**该会话记录的工作目录**继续执行。

## 功能

- **聊天流**：流式 Markdown（表格、任务列表、代码块复制按钮、链接悬停显示完整 URL、图片、ANSI 彩色输出）、思考过程（流式展开、完成折叠、耗时显示）、进行中阶段徽标与记忆引用、计划/待办、上下文压缩提示；回合进行中顶部流光进度线 + “思考中”发光提示。
- **滚动体验**：流式输出时上滑查看历史不会被拉回底部，右上角“回到底部”按钮一键恢复跟随。
- **工具卡片**：命令执行显示真实命令、工作目录、实时输出、**实时耗时**、退出码；文件变更带 diff；网络搜索（结构化结果列表）、MCP/动态工具、子代理协作。
- **交互弹窗**：提权审批（批准 / 拒绝 / 本次会话批准 / 批准并记住此规则）、选项提问、MCP 表单；弹窗出现时播放提示音并置前窗口；回合可随时停止并提示“已停止生成”；自定义右键菜单（复制/粘贴/打开链接，无默认“刷新”）。
- **历史会话**：展示全部目录的会话并标注目录；**加载更多**分页、重命名、固定置顶（带“置顶”徽章）、搜索（带摘要）、删除确认；打开为只读，发消息才恢复；继续执行使用会话记录的工作目录，历史中的命令/工具详情完整加载。
- **权限模式**：请求批准 / 帮我批准 / 完全访问权限（对话进行中按钮禁用）。
- **任务模式**：执行 / 计划 / 目标（目标对话框）。
- **模型与上下文**：模型下拉选择（`model/list`）、推理强度跟随模型动态生成（如 `low/high/max`）、模型按钮显示“名称(强度)”、上下文窗口百分比椭圆框（悬停显示“上下文已用 xx K，共 yy K”）。
- **输入区**：通栏输入框 + 悬浮靠底按钮；`+` 菜单、`@`/`$` 菜单；**上下键选择输入历史**（内存）；**Ctrl+V 粘贴图片/文件添加附件**（截图/图片落盘到临时目录，复制文件用原始路径）；本地图片附件在消息中直接渲染（输入区带缩略图）；发送/停止二合一按钮（带文字）；**新建对话前可选项目目录**。`@` 文件引用以 VS Code 扩展同款 `# Files mentioned by the user:` 文本段发送，`$` 技能以 `[$name](path)` 文本链接 + 结构化技能项发送（服务端注入技能内容），回显时渲染为 `@名称` / `$名称` 引用标签。
- **头部**：六边形 Logo、工作目录可点击在资源管理器中打开、历史/设置/新建对话 Tab 式导航。
- **设置**：codex 路径、Enter 快捷发送、跟进处理方式（调整方向 / 加入队列）、提示音开关（权限模式、模型、推理强度为进程级，在输入区按钮菜单中配置）。

## 架构

```
codex-ui.exe（Tauri 2 窗口）
├── Rust 后端  src-tauri/src/codex/
│   ├── app_server.rs  codex.exe app-server --stdio（换行分隔 JSON-RPC）
│   ├── commands.rs    Tauri 命令层
│   └── settings.rs    设置读写
└── Vue 3 前端  src/
    ├── composables/useCodex.ts  状态 + 事件订阅 + 协议调用
    └── components/              界面组件
```

后端负责 codex 子进程生命周期（断线自动重连）、JSON-RPC 请求/响应分发、服务端通知流式转发（`item/started`、`item/completed`、`item/agentMessage/delta`、`item/commandExecution/outputDelta`、`interaction:request` 等）与交互响应回写。前端通过 Tauri invoke/event 通信。

## 环境要求

- Windows 10/11（自带 WebView2）
- Rust stable-msvc（rustup）+ VS C++ 构建工具
- Node.js 18+
- `codex` CLI（可在设置页指定路径；验证基线 0.146.0-alpha.9.2）

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

- **工作目录**：从哪个目录启动 codex-ui，新建会话默认归到该目录；新建对话前可点输入区上方的“项目目录”选择其它目录（点击目录可在资源管理器中打开）。历史会话显示各自记录的目录，继续执行用记录目录。
- **历史会话**：面板顶部即搜索框；支持加载更多、重命名、固定置顶、删除确认。点击仅查看，输入消息后才会恢复并执行。带**活跃目标**的会话恢复后会按目标模式自动持续执行——切出目标模式会自动清除目标。
- **权限模式**：请求批准会询问所有外部写操作；“批准并记住此规则”会把规则写入 codex 策略。完全访问不受限。权限/模型/推理强度为**进程级**，重启后恢复默认。
- **设置文件**：`%APPDATA%\com.codexui.app\settings.json`，仅保存 codex 路径、Enter 快捷发送、跟进处理方式、提示音开关。
- **登录**：界面不提供登录入口，请使用其它入口（如 `codex login` 或 API Key）完成认证。

## 协议兼容性

以 `codex app-server generate-ts --experimental` 输出的协议绑定为参考（验证基线 0.146.0-alpha.9.2）。后端只实现本项目所需字段，未知通知忽略并记录日志。codex CLI 升级后如协议变化，可重新生成绑定核对：

```powershell
codex app-server generate-ts --out <dir> --experimental
```

## 常见问题

- **启动后“localhost 拒绝连接”**：生产构建缺少 `tauri/custom-protocol` 特性（或前端未构建）。确认 `Cargo.toml` 已启用该特性并重新 `cargo build --release`。
- **历史显示全部而非当前目录**：当前版本按需求设计为展示全部会话并标注目录；如仍异常，确认运行的是最新构建（旧版存在初始化时序 bug）。
- **点击历史后发送/停止按钮反复交替**：该会话带活跃目标且被自动恢复。新版打开会话为只读、不会触发；如已发生，清掉该会话目标或删除会话。
- **“批准并记住此规则”后仍被拒**：确认权限模式为“请求批准/帮我批准”（沙箱为 workspace-write）；早期版本误用 read-only 沙箱导致该问题。
- **本地图片不显示**：确认构建启用了 `protocol-asset` 特性且 `tauri.conf.json` 配置了 `assetProtocol`。
- **继续历史会话报 `invalid_request_error: you passed .`**：旧版会向服务端发送空模型；更新到最新版本（修复后自动回退到默认模型）。
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
- [素材与参考说明](docs/素材说明.md)
- [引用与附件协议说明（@ / $ / 图片）](docs/引用与附件.md)
