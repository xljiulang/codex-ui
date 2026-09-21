# AGENTS.md

> 面向 Codex 等 agent 协作工具的仓库操作指南，内容与当前仓库状态核对（2026-08-22）。
> 详细功能说明见 [README.md](README.md)；前端模块级约定见 [docs/前端模块地图.md](docs/前端模块地图.md)。
> docs/ 下另有协议盘点、引用与附件、测试报告等资料，按需查阅。

## 项目概览

codex-ui 是 Codex CLI 的 Windows 桌面客户端（Tauri 2）：启动后拉起 `codex app-server --stdio`，通过 JSON-RPC 协议驱动 codex，提供中文桌面界面。

- 后端：Rust，位于 `src-tauri/src/codex/`（子进程生命周期、JSON-RPC 分发、交互响应、会话资源/终端/Git/设置等）。
- 前端：Vue 3 + TypeScript，位于 `src/`（components / composables / lib / styles）。
- 平台：仅 Windows 10/11；代码注释、UI 文案、文档使用中文。

> 注意：本文件是仓库级的 agent 指令；应用自身在设置页管理的 `CODEX_HOME/AGENTS.md`（见 `src-tauri/src/codex/custom_instructions.rs`）是另一份运行期自定义指令，二者互不影响。

## 常用命令

```powershell
npm run dev                    # Vite 开发服务器（严格端口 5173）
npm run build                  # vue-tsc --noEmit + vite build
npm run tauri dev              # Vite 热更新 + Tauri 窗口
build-dev.bat                  # 开发入口：预检 + 依赖 + tauri dev
build-release.bat              # 发布打包（Inno Setup）→ setup\output\codex-ui-win-x64.exe

npm test                       # 前端单元测试（等价 npm run test:unit）
npm run test:unit              # vitest 单测
npm run test:typecheck         # vue-tsc 类型检查
npm run test:coverage          # 单测 + 覆盖率，门槛 lines≥80 / functions≥75 / statements≥75 / branches≥70
npm run test:rust              # Rust 单元测试（cargo test --lib）
npm run test:rust:kb           # 知识库 CLI 单元测试（cargo test -p knowledge-cli）
npm run test:rust:integration  # 真实 app-server 集成测试（需 $env:CODEX_BIN='codex'）
npm run test:all               # 单测 + 类型 + Rust（未设置 CODEX_BIN 时集成用例自动跳过）
npm run test:e2e               # E2E 编排（需 release 构建与 codex CLI，详见 README）
```

修改后至少运行对应层级测试：涉及前端跑 `test:unit` + `test:typecheck`，涉及 Rust 跑 `test:rust`，涉及知识库（`src-tauri/crates/knowledge-cli/`）跑 `test:rust:kb`。

## 架构速览

前端（`src/`）：

- `components/`：Vue 界面组件（RightPanel + HistoryView/ResourceView/GitView、EditorPane 系列：TextEditorPane/DiffPane/PreviewPane/TerminalPane 等）。
- `composables/`：状态中心。`useCodex/` 按领域拆分（store/事件桥/会话/回合/历史等，入口 `index.ts`）；`useEditorTabs.ts` 左侧多标签；`useSessionFs.ts` 会话资源；`useGitChanges.ts` Git 状态；`useTerminalEvents.ts` 终端事件桥。
- `lib/`：纯函数工具（路径、Markdown、格式、标签基类等）。
- 测试与源码同目录：`__tests__/*.spec.ts`（vitest 仅收集 `src/**/*.spec.ts`）。

后端（`src-tauri/src/codex/`）：

- `app_server.rs`：`codex app-server --stdio` 子进程与换行分隔 JSON-RPC。
- `model_catalog/`：模型目录生成管线（`sources/` 多数据源提取 → 合并 → 模板渲染，含官方条目整条复用），字段与匹配规则见 [docs/model_catalog_json.md](docs/model_catalog_json.md)。
- `commands.rs`：Tauri 命令层；`session_fs.rs` 文件系统 + notify 监听；`terminal.rs` ConPTY 终端；`git.rs` 调用系统 git.exe；`settings.rs` 设置读写（`%APPDATA%\com.codexui.app\settings.json`）；`path_util.rs` 路径清洗；`file_icon.rs` / `diff.rs` / `pdf_export.rs` / `skills.rs` / `mcp_servers.rs` / `model_config.rs` 等按职责拆分。

独立进程（`src-tauri/crates/knowledge-cli/`，workspace 成员）：

- `codexui-kb.exe`：知识库全部实现（抽取 → 切块 → 本地 ONNX 向量化 → SQLite 向量+FTS5 → RRF 混合检索）都在这里，由应用以一次性子进程调用，避免向量化会话与全量向量常驻 UI 进程（`src-tauri/src/codex/knowledge/` 只剩 `paths.rs` 数据目录约定、`cli.rs` 子进程调用层与 `commands.rs` 编排）。
- 改动知识库逻辑改 CLI crate；改动调用协议时两边必须同步：`crates/knowledge-cli/src/cli.rs` 的 `API_VERSION` 与 `src/codex/knowledge/cli.rs` 的 `API_VERSION`。
- 该 CLI 目录自包含：`codexui-kb.exe` + `onnxruntime.dll` + `model\bge-small-zh-v1.5\` 同目录（发布版 `{app}\bin\`、开发版 `src-tauri\target\debug\`），模型与 DLL 都由子进程按自身目录解析；应用数据目录 `%APPDATA%\com.codexui.app\knowledge\` 下只有 `kbs\`。本地开发用 `pwsh -File scripts/build-knowledge-model.ps1 -AlsoDev` 铺模型（`build-dev.bat` 缺模型时自动调用）。

集成测试位于 `src-tauri/tests/app_server_integration.rs`（真实 app-server 握手/回合、目标、记忆、线程设置等，需 `CODEX_BIN`）。

## 关键约定

- **Windows 路径**：比较/相对化必须走 `src/lib/path.ts` 的共享方法（`pathEquals` / `isPathUnderRoot` / `relPathOf`，Map/Set 键统一经 `normalizeFsPath` / `normalizePathKey`）；后端对外绝对路径统一反斜杠（`path_util::clean_path`）。
- **会话状态隔离**：会话级字段只读写会话标签对象（`tab.*`），不落全局 `store.*`；回合事件按 `threadId` 路由归属，禁止用 `activeSessionTab()` 给缺失身份的事件兜底。已移除的 store 字段清单与正确写法见 docs/前端模块地图.md。
- **语言与风格**：UI 文案、注释、文档用中文；LF 行尾（`.gitattributes`）；TypeScript `strict`（含 `noUnusedLocals` / `noUnusedParameters`）。
- **不提交**：`dist/`、`coverage/`、`src-tauri/target`、`src-tauri/gen`、`node_modules`、日志（已在 `.gitignore`）。
- **codex 协议**：验证基线 codex-cli 0.154.0（兼容 0.149.0+，启动时探测版本、仅低版本警告）；后端只实现所需字段，未知通知忽略并记日志，前端无需交互的服务端反向请求由后端在 `app_server.rs` 直接应答；协议变更后用 `codex app-server generate-ts --experimental` 重新生成绑定核对（详见 README 与 docs/app-server.md §1.1）。
- **修改功能流程**：前端改动通常联动「组件 + 对应 composable + 必要时后端命令」；涉及用户可见行为时同步更新 [README.md](README.md) 与 [docs/变更记录.md](docs/变更记录.md)。
