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
npm run tauri build            # 打包 NSIS/MSI 安装包

npm test                       # 前端单元测试（等价 npm run test:unit）
npm run test:unit              # vitest 单测
npm run test:typecheck         # vue-tsc 类型检查
npm run test:coverage          # 单测 + 覆盖率，门槛 lines≥80 / functions≥75 / statements≥75 / branches≥70
npm run test:rust              # Rust 单元测试（cargo test --lib）
npm run test:rust:integration  # 真实 app-server 集成测试（需 $env:CODEX_BIN='codex'）
npm run test:all               # 单测 + 类型 + Rust（未设置 CODEX_BIN 时集成用例自动跳过）
npm run test:e2e               # E2E 编排（需 release 构建与 codex CLI，详见 README）
```

修改后至少运行对应层级测试：涉及前端跑 `test:unit` + `test:typecheck`，涉及 Rust 跑 `test:rust`。

## 代码定位与搜索

- 仓库已建立 `.codegraph/` 索引：理解/定位代码时**优先** `codegraph explore "<符号或问题>"`（输出相关符号源码与调用路径），再退回常规搜索。
- 文本搜索：`rg`（ripgrep 15.0.0）。
- 文件查找：`fd`（fd 10.4.2）。
- AST 结构搜索/重写：`sg`（ast-grep 的旧命令名；当前环境未提供独立的 `ast-grep`，且 `sg` 运行时会提示弃用——在 ast-grep 可用前继续使用 `sg`）。

## 架构速览

前端（`src/`）：

- `components/`：Vue 界面组件（RightPanel + HistoryView/ResourceView/GitView、EditorPane 系列：TextEditorPane/DiffPane/PreviewPane/TerminalPane 等）。
- `composables/`：状态中心。`useCodex/` 按领域拆分（store/事件桥/会话/回合/历史等，入口 `index.ts`）；`useEditorTabs.ts` 左侧多标签；`useSessionFs.ts` 会话资源；`useGitChanges.ts` Git 状态；`useTerminalEvents.ts` 终端事件桥。
- `lib/`：纯函数工具（路径、Markdown、格式、标签基类等）。
- 测试与源码同目录：`__tests__/*.spec.ts`（vitest 仅收集 `src/**/*.spec.ts`）。

后端（`src-tauri/src/codex/`）：

- `app_server.rs`：`codex app-server --stdio` 子进程与换行分隔 JSON-RPC。
- `commands.rs`：Tauri 命令层；`session_fs.rs` 文件系统 + notify 监听；`terminal.rs` ConPTY 终端；`git.rs` 调用系统 git.exe；`settings.rs` 设置读写（`%APPDATA%\com.codexui.app\settings.json`）；`path_util.rs` 路径清洗；`file_icon.rs` / `diff.rs` / `pdf_export.rs` / `skills.rs` / `mcp_servers.rs` / `model_config.rs` 等按职责拆分。

集成测试位于 `src-tauri/tests/app_server_integration.rs`（真实 app-server 握手/回合、目标、记忆、线程设置等，需 `CODEX_BIN`）。

## 关键约定

- **Windows 路径**：比较/相对化必须走 `src/lib/path.ts` 的共享方法（`pathEquals` / `isPathUnderRoot` / `relPathOf`，Map/Set 键统一经 `normalizeFsPath` / `normalizePathKey`）；后端对外绝对路径统一反斜杠（`path_util::clean_path`）。
- **会话状态隔离**：会话级字段只读写会话标签对象（`tab.*`），不落全局 `store.*`；回合事件按 `threadId` 路由归属，禁止用 `activeSessionTab()` 给缺失身份的事件兜底。已移除的 store 字段清单与正确写法见 docs/前端模块地图.md。
- **语言与风格**：UI 文案、注释、文档用中文；LF 行尾（`.gitattributes`）；TypeScript `strict`（含 `noUnusedLocals` / `noUnusedParameters`）。
- **不提交**：`dist/`、`coverage/`、`src-tauri/target`、`src-tauri/gen`、`node_modules`、日志（已在 `.gitignore`）。
- **codex 协议**：仅适配 codex-cli 0.149.x（验证基线 0.149.0，启动时探测版本、仅低版本警告）；后端只实现所需字段，未知通知忽略并记日志；协议变更后用 `codex app-server generate-ts --experimental` 重新生成绑定核对（详见 README）。
- **修改功能流程**：前端改动通常联动「组件 + 对应 composable + 必要时后端命令」；涉及用户可见行为时同步更新 [README.md](README.md) 与 [docs/变更记录.md](docs/变更记录.md)。
