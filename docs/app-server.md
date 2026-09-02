# codex.exe app-server 协议完整参考

> 本文档基于本机 `codex.exe`（**codex-cli 0.149.0**，2026-08-23）实测生成：`codex app-server generate-ts --experimental` 与 `codex app-server generate-json-schema --experimental` 的产物，以及官方 `openai/codex` 仓库 `codex-rs/app-server/README.md`（rust-v0.149.0 分支）的协议说明。
> 协议为实验性（`[experimental]`），随 codex 版本演进；生成产物与运行版本一一对应，升级 codex 后应重新生成并核对。

---

## 1. 概述

`codex app-server` 是 Codex 用于驱动富客户端的接口，官方 VS Code Codex 插件即通过它工作。它暴露三个顶层原语：

- **Thread（会话/线程）**：用户与 Codex 的一次对话，包含多个 Turn。
- **Turn（回合）**：一次对话轮次，通常以用户消息开始、以智能体消息结束，包含多个 Item。
- **Item（条目）**：回合内持久化的用户输入与智能体输出（用户消息、推理、智能体消息、shell 命令、文件编辑等），也是后续回合的上下文。

协议特点：

- 双向 JSON-RPC 2.0（线上报文省略 `"jsonrpc":"2.0"` 头字段；服务端对请求的解析以 `id/method/params` 为准）。
- 默认 stdio 传输、换行分隔 JSON（JSONL），一请求一应答，服务端可随时主动下发请求与通知。
- 客户端每连接必须先 `initialize`，随后发 `initialized` 通知，之后才能调用其它方法。
- 审批、提问、MCP 表单等由**服务端反向发起 JSON-RPC 请求**，客户端应答后回合继续。
- 大量方法与字段由 `capabilities.experimentalApi` 门控（见第 10 节）。

## 2. 启动与传输

### 2.1 启动命令

```powershell
codex app-server --stdio                 # 等价于 --listen stdio://（默认）
codex app-server --listen unix://        # $CODEX_HOME/app-server-control/app-server-control.sock
codex app-server --listen unix://PATH    # 自定义 socket 路径
codex app-server --listen ws://IP:PORT   # websocket（实验性/不支持生产）
codex app-server --listen off            # 不暴露本地传输
```

常用启动选项（`codex app-server --help`）：

| 选项 | 说明 |
|---|---|
| `--stdio` | 使用 stdio 传输（等价 `--listen stdio://`） |
| `--listen <URL>` | `stdio://`（默认）/ `unix://` / `unix://PATH` / `ws://IP:PORT` / `off` |
| `--code-mode-host <WS_URL>` | 连接远程 code-mode host（`wss://` 远端，`ws://` 本地），与本进程的 `--listen` 相互独立 |
| `--analytics-default-enabled` | 默认开启分析（官方插件等第一方使用；用户可在 config.toml 中 `[analytics] enabled=false` 关闭） |
| `--ws-auth <MODE>` | 非 loopback websocket 监听器的鉴权：`capability-token` / `signed-bearer-token` |
| `--ws-token-file` / `--ws-token-sha256` | capability token 文件 / SHA-256 |
| `--ws-shared-secret-file` / `--ws-issuer` / `--ws-audience` / `--ws-max-clock-skew-seconds` | signed JWT bearer 配置 |
| `-c key=value` / `--enable` / `--disable` | 覆盖配置 / 开启、关闭 feature |

### 2.2 传输细节

- **stdio**：每条消息是一行 JSON（JSONL），读 stdout、写 stdin。业务日志走 stderr。
- **websocket**（实验性）：一个文本帧一条 JSON-RPC 消息；同一监听同时提供 `GET /readyz`（就绪返回 200）、`GET /healthz`（无 `Origin` 头返回 200）；携带 `Origin` 的请求一律 403。**不建议生产依赖**。
- **unix socket**：`$CODEX_HOME/app-server-control/app-server-control.sock`（或 `--listen unix://PATH`），连接采用标准 HTTP Upgrade 握手 + websocket 帧。供控制面客户端使用。
- **proxy**：`codex app-server proxy [--sock PATH]` 打开一条到控制 socket 的原始流连接，在 socket 与 stdin/stdout 间双向转发字节（含 websocket 升级握手）。
- **daemon 子命令**：`codex app-server daemon {bootstrap,start,restart,stop,version,enable-remote-control,disable-remote-control}`。**仅在 Unix 平台支持**（Windows 实测报 `codex app-server daemon lifecycle is only supported on Unix platforms`）；Windows 客户端直接以 `--stdio` 拉起进程。
- **背压**：服务端在传输入口、请求处理、出站写入之间使用有界队列；入口饱和时以 JSON-RPC 错误码 `-32001`、消息 `"Server overloaded; retry later."` 拒绝新请求，客户端应指数退避重试。
- **日志**：`RUST_LOG` 控制过滤级别；`LOG_FORMAT=json` 将 stderr 追踪日志输出为 JSON（每行一条）。

### 2.3 消息帧

```jsonc
// 客户端请求（id 必填，method 必填，params 可选，trace 可选）
{ "id": 1, "method": "thread/list", "params": { "limit": 50 } }

// 服务端成功应答
{ "id": 1, "result": { "data": [], "nextCursor": null, "backwardsCursor": null } }

// 服务端错误应答
{ "id": 1, "error": { "code": -32601, "message": "method not found", "data": null } }

// 通知（无 id，不期待应答）
{ "method": "turn/completed", "params": { "threadId": "thr_…", "turn": { … } } }
```

- `RequestId`：`string | number`。
- `JSONRPCRequest` 额外支持可选 `trace`（W3C Trace Context：`{ traceparent?, tracestate? }`）。
- `JSONRPCError`：`{ code: int, message: string, data?: any }`。
- 服务端通知可选带 `emittedAtMs`（Unix 毫秒，app-server 向各连接扇出前的时间戳；旧版本可能缺失）——即 `ServerNotificationEnvelope`。

> codex-ui 实测：本项目的 Rust 后端在请求报文中显式携带 `"jsonrpc":"2.0"`（如 `{"jsonrpc":"2.0","id":1,"method":…}`），真实 app-server 0.149.0 正常接受。

## 3. 生命周期

```
连接建立
  → initialize（请求）
  → initialized（通知）
  → thread/start（新会话）或 thread/resume（继续旧会话）或 thread/fork（分叉）
      → 应答返回 thread；服务端下发 thread/started 通知
  → turn/start（发送用户输入，立即返回 turn 对象）
      → turn/started 通知（status: inProgress）
      → item/started → item 专属 delta* → item/completed（每个条目）
      → turn/diff/updated / turn/plan/updated / thread/tokenUsage/updated 等
      → turn/completed 通知（completed / interrupted / failed）
  → thread/delete 或 thread/unsubscribe（停止订阅；无订阅 30 分钟后卸载并发 thread/closed）
```

要点：

- 订阅关系：`thread/start` / `thread/resume` / `thread/fork` 成功后自动订阅该线程的回合/条目事件；`thread/unsubscribe` 取消订阅。
- 分页：`thread/list`、`thread/search`、`thread/turns/list`、`thread/items/list` 均返回 `nextCursor`（向后翻）与 `backwardsCursor`（反向翻页锚点）。
- `thread/read` 不加载线程；`thread/turns/list` 可在不 resume 的情况下分页读取历史。分页线程（`historyMode=paginated`，codex CLI 创建）不支持 `thread/read(includeTurns=true)`，客户端须以 `includeTurns: false` 读元数据，历史统一用 `thread/turns/list` 分页读取。
- 目标（goal）挂在线程上，由服务端 auto-continuation 循环驱动自动续跑，直到完成/预算耗尽/暂停/清除（见 `thread/goal/*`）。

## 4. 初始化握手

### 4.1 initialize 请求

`initialize.params`（`InitializeParams`）：

```ts
type ClientInfo = { name: string; title: string | null; version: string };

type InitializeCapabilities = {
  experimentalApi: boolean;                    // 实验 API 门控开关
  requestAttestation: boolean;                 // 是否接收 attestation/generate 请求
  mcpServerOpenaiFormElicitation?: boolean;    // 是否允许 MCP openai/form 扩展表单
  optOutNotificationMethods?: Array<string> | null; // 精确方法名通知抑制列表
};

type InitializeParams = { clientInfo: ClientInfo; capabilities: InitializeCapabilities | null };
```

官方示例（VS Code 插件）：

```json
{ "method": "initialize", "id": 0, "params": {
    "clientInfo": { "name": "codex_vscode", "title": "Codex VS Code Extension", "version": "0.1.0" }
} }
```

带能力声明与通知抑制：

```json
{ "method": "initialize", "id": 1, "params": {
    "clientInfo": { "name": "my_client", "title": "My Client", "version": "0.1.0" },
    "capabilities": {
      "experimentalApi": true,
      "optOutNotificationMethods": ["thread/started", "item/agentMessage/delta"]
    }
} }
```

codex-ui 实际发送（Rust 后端 `src-tauri/src/codex/app_server.rs`）：

```json
{ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "clientInfo": { "name": "codex-ui", "title": "Codex UI", "version": "0.1.2" },
    "capabilities": { "experimentalApi": true, "requestAttestation": false }
} }
```

### 4.2 initialize 应答

```ts
type InitializeResponse = {
  userAgent: string;          // 服务端向上游展示的 user agent 字符串
  codexHome: string;          // $CODEX_HOME 绝对路径
  platformFamily: string;     // "unix" | "windows" 等
  platformOs: string;         // "macos" | "linux" | "windows" 等
};
```

### 4.3 initialized 通知

收到 initialize 应答后，客户端必须发送一次无 id 通知：

```json
{ "method": "initialized" }
```

这是唯一一个客户端 → 服务端通知（`ClientNotification`）。

### 4.4 握手错误与约束

- 未初始化就调用其它方法：`"Not initialized"` 错误。
- 同连接重复 initialize：`"Already initialized"` 错误。
- `capabilities.experimentalApi` 在初始化时一次性协商，进程生命周期内生效。
- 通知抑制为精确匹配（无通配符），未知方法名被接受并忽略；仅作用于服务端类型化通知（`thread/*`、`turn/*`、`item/*`、`rawResponseItem/*`），不影响请求/应答/错误。
- `clientInfo.name` 用于 OpenAI Compliance Logs Platform 客户端识别；企业级新集成需登记已知客户端列表。

## 5. 客户端 → 服务端请求（130 个，含实验）

> 下表 params 类型名即 `codex app-server generate-ts --experimental` 产物中的类型名（未加前缀的在根目录，`v2/` 前缀的位于 `v2/` 目录；`undefined` 表示无参数或传 `null`）。标注“实验”的需 `capabilities.experimentalApi = true`。

### 5.1 会话（thread）生命周期

| 方法 | params | 说明 |
|---|---|---|
| `thread/start` | `v2/ThreadStartParams` | 新建会话；返回 `ThreadStartResponse`，并发 `thread/started` 通知 |
| `thread/resume` | `v2/ThreadResumeParams` | 按 threadId 恢复旧会话（或按 history/path）；返回 `ThreadResumeResponse` |
| `thread/fork` | `v2/ThreadForkParams` | 从现有会话分叉（可 `lastTurnId` / `beforeTurnId` 截断）；返回 `ThreadForkResponse` |
| `thread/read` | `v2/ThreadReadParams` | 不加载读取存储线程（可选 `includeTurns`；分页线程不支持 `includeTurns=true`）；返回 `ThreadReadResponse` |
| `thread/list` | `v2/ThreadListParams` | 分页列出存储线程（cursor/limit/sort/filter）；返回 `ThreadListResponse` |
| `thread/loaded/list` | `v2/ThreadLoadedListParams` | 当前内存中已加载线程 id 列表（实验） |
| `thread/turns/list` | `v2/ThreadTurnsListParams` | 分页读回合历史（实验；`itemsView`：notLoaded/summary/full） |
| `thread/items/list` | `v2/ThreadItemsListParams` | 分页读条目历史（实验；可 `turnId` 限定单回合） |
| `thread/search` | `v2/ThreadSearchParams` | 按 `searchTerm` 全库搜索线程 |
| `thread/searchOccurrences` | `v2/ThreadSearchOccurrencesParams` | 线程内大小写不敏感字面匹配定位（实验） |
| `thread/archive` | `v2/ThreadArchiveParams` | 归档会话（含衍生子线程），发 `thread/archived` |
| `thread/unarchive` | `v2/ThreadUnarchiveParams` | 恢复归档会话，返回 thread，发 `thread/unarchived` |
| `thread/delete` | `v2/ThreadDeleteParams` | 硬删除会话及衍生子线程，发 `thread/deleted` |
| `thread/unsubscribe` | `v2/ThreadUnsubscribeParams` | 取消本连接对该线程的订阅；最后一个订阅者无活动 30 分钟后卸载并发 `thread/closed` |
| `thread/name/set` | `v2/ThreadSetNameParams` | 设置用户可见名称，发 `thread/name/updated` |
| `thread/metadata/update` | `v2/ThreadMetadataUpdateParams` | 修补 sqlite 中的 `gitInfo`/`isPinned`，返回刷新后 thread |
| `thread/settings/update` | `v2/ThreadSettingsUpdateParams` | 排队修改已加载线程的“下一回合”设置（实验；变化时发 `thread/settings/updated`） |
| `thread/compact/start` | `v2/ThreadCompactStartParams` | 触发上下文压缩，进度走标准 turn/item 通知 |
| `thread/inject_items` | `v2/ThreadInjectItemsParams` | 向已加载线程追加原始 Responses API 条目（不进模型可见历史前不开新回合） |
| `thread/rollback` | `v2/ThreadRollbackParams` | 丢弃最近 N 回合（**已弃用，即将移除**；分页线程不支持） |
| `thread/shellCommand` | `v2/ThreadShellCommandParams` | 以“!”方式在会话内运行命令（不受线程沙箱约束，全访问） |
| `thread/backgroundTerminals/clean` | `v2/ThreadBackgroundTerminalsCleanParams` | 清理会话全部后台终端（实验） |
| `thread/backgroundTerminals/list` | `v2/ThreadBackgroundTerminalsListParams` | 列出会话运行中的后台终端（实验） |
| `thread/backgroundTerminals/terminate` | `v2/ThreadBackgroundTerminalsTerminateParams` | 按 processId 终止一个后台终端（实验） |
| `thread/approveGuardianDeniedAction` | `v2/ThreadApproveGuardianDeniedActionParams` | 监护人拒绝后的申诉批准 |
| `thread/increment_elicitation` / `thread/decrement_elicitation` | `v2/ThreadIncrementElicitationParams` / `v2/ThreadDecrementElicitationParams` | 服务端待处理交互计数 ±1 |

### 5.2 目标（goal）与记忆

| 方法 | params | 说明 |
|---|---|---|
| `thread/goal/set` | `v2/ThreadGoalSetParams` | 创建/更新线程目标（`objective`、`status`、`tokenBudget`），返回当前 goal 并发 `thread/goal/updated` |
| `thread/goal/get` | `v2/ThreadGoalGetParams` | 读取线程目标；无目标返回 `goal: null` |
| `thread/goal/clear` | `v2/ThreadGoalClearParams` | 清除线程目标，状态变化时发 `thread/goal/cleared` |
| `thread/memoryMode/set` | `v2/ThreadMemoryModeSetParams` | 设置线程记忆资格 `"enabled"` / `"disabled"`（实验） |
| `memory/reset` | `undefined` | 清空 `CODEX_HOME/memories` 并重置 sqlite 记忆阶段数据（实验），保留线程记忆模式 |

### 5.3 回合（turn）

| 方法 | params | 说明 |
|---|---|---|
| `turn/start` | `v2/TurnStartParams` | 向线程追加用户输入并开始生成；立即返回 `TurnStartResponse{turn}`，随后流式下发 `turn/started`、`item/*`、`turn/completed` |
| `turn/steer` | `v2/TurnSteerParams` | 向进行中的回合追加输入（`expectedTurnId` 前置校验）；review/手动压缩回合拒绝 steer |
| `turn/interrupt` | `v2/TurnInterruptParams` | 取消进行中回合；返回 `{}`，回合以 `interrupted` 结束 |

`TurnStartParams` 关键字段：`threadId`、`input: Array<UserInput>`、`clientUserMessageId?`、`responsesapiClientMetadata?`、`additionalContext?`、`environments?`、`cwd?`、`runtimeWorkspaceRoots?`、`approvalPolicy?`、`approvalsReviewer?`、`sandboxPolicy?`（与 `permissions` 互斥）、`permissions?`、`model?`、`serviceTier?`、`effort?`、`summary?`、`personality?`、`outputSchema?`、`collaborationMode?`、`multiAgentMode?`（已弃用，忽略）。

### 5.4 实时会话（realtime，全部实验）

| 方法 | params | 说明 |
|---|---|---|
| `thread/realtime/start` | `v2/ThreadRealtimeStartParams` | 启动线程级实时会话（`outputModality` 为 text 或 audio，`model`/`version`/`transport` 等）；流式 `thread/realtime/*` 通知 |
| `thread/realtime/appendAudio` | `v2/ThreadRealtimeAppendAudioParams` | 追加输入音频块 |
| `thread/realtime/appendText` | `v2/ThreadRealtimeAppendTextParams` | 追加文本输入（`role`: user/developer/assistant，缺省 user） |
| `thread/realtime/appendSpeech` | `v2/ThreadRealtimeAppendSpeechParams` | 追加要模型朗读的文本 |
| `thread/realtime/stop` | `v2/ThreadRealtimeStopParams` | 停止实时会话 |
| `thread/realtime/listVoices` | `v2/ThreadRealtimeListVoicesParams` | 列出可用语音 |

### 5.5 模型 / 能力 / 协作模式

| 方法 | params | 说明 |
|---|---|---|
| `model/list` | `v2/ModelListParams` | 列出可用模型（`includeHidden`；保留 `supportedReasoningEfforts` 顺序） |
| `modelProvider/capabilities/read` | `v2/ModelProviderCapabilitiesReadParams` | 读取当前 provider 能力 |
| `experimentalFeature/list` | `v2/ExperimentalFeatureListParams` | 列出实验特性（stage：beta/underDevelopment/stable 等，cursor 分页） |
| `experimentalFeature/enablement/set` | `v2/ExperimentalFeatureEnablementSetParams` | 修改进程级内存中的特性开关（优先级：cloud requirements > `--enable` > config.toml > 本方法 > 默认） |
| `permissionProfile/list` | `v2/PermissionProfileListParams` | 列出权限配置 id（beta，cursor 分页） |
| `collaborationMode/list` | `v2/CollaborationModeListParams` | 列出协作模式预设（实验；内置预设不选模型，Plan 预设用 medium 推理强度） |
| `mock/experimentalMethod` | `v2/MockExperimentalMethodParams` | 实验门控测试用（实验） |

### 5.6 技能 / 钩子 / 插件 / 市场 / 应用

| 方法 | params | 说明 |
|---|---|---|
| `skills/list` | `v2/SkillsListParams` | 按 cwd 列出技能（可选 `forceReload`） |
| `skills/extraRoots/set` | `v2/SkillsExtraRootsSetParams` | 替换进程级额外独立技能根（不持久化） |
| `skills/config/write` | `v2/SkillsConfigWriteParams` | 按名称或绝对路径写入用户级技能配置 |
| `hooks/list` | `v2/HooksListParams` | 按 cwd 列出发现的钩子 |
| `marketplace/add` | `v2/MarketplaceAddParams` | 添加远端插件市场（HTTP(S)/SSH Git URL 或 `owner/repo`） |
| `marketplace/remove` | `v2/MarketplaceRemoveParams` | 移除已配置市场并删除其安装根 |
| `marketplace/upgrade` | `v2/MarketplaceUpgradeParams` | 升级全部或指定市场 |
| `plugin/list` | `v2/PluginListParams` | 列出市场与插件状态（可用性、安装策略、`mustShowInstallationInterstitial` 等） |
| `plugin/installed` | `v2/PluginInstalledParams` | 已安装插件行（不含完整远端目录） |
| `plugin/read` | `v2/PluginReadParams` | 读取单个插件详情（含 bundled skills/hooks/apps/MCP） |
| `plugin/skill/read` | `v2/PluginSkillReadParams` | 按远端市场/插件/技能名读取未安装插件技能 markdown |
| `plugin/install` | `v2/PluginInstallParams` | 安装插件（含 MCP），返回 auth policy 与待认证 app |
| `plugin/uninstall` | `v2/PluginUninstallParams` | 卸载本地/远端插件 |
| `plugin/share/save` | `v2/PluginShareSaveParams` | 创建/更新插件分享 |
| `plugin/share/updateTargets` | `v2/PluginShareUpdateTargetsParams` | 更新分享目标 |
| `plugin/share/list` | `v2/PluginShareListParams` | 列出分享 |
| `plugin/share/checkout` | `v2/PluginShareCheckoutParams` | 签出分享 |
| `plugin/share/delete` | `v2/PluginShareDeleteParams` | 删除分享 |
| `app/read` | `v2/AppsReadParams` | 读取单个 app/连接器 |
| `app/list` | `v2/AppsListParams` | 列出可用 app |
| `app/installed` | `v2/AppsInstalledParams` | 读取已安装连接器运行时状态（可选先刷新） |

### 5.7 文件系统

| 方法 | params | 说明 |
|---|---|---|
| `fs/readFile` | `v2/FsReadFileParams` | 读绝对路径文件，返回 `{ dataBase64 }` |
| `fs/writeFile` | `v2/FsWriteFileParams` | 以 base64 写文件 |
| `fs/createDirectory` | `v2/FsCreateDirectoryParams` | 建目录（`recursive` 默认 true） |
| `fs/getMetadata` | `v2/FsGetMetadataParams` | 路径元数据（isDirectory/isFile/isSymlink/createdAtMs/modifiedAtMs） |
| `fs/readDirectory` | `v2/FsReadDirectoryParams` | 列目录直接子项（`fileName` 仅子名） |
| `fs/remove` | `v2/FsRemoveParams` | 删除文件/目录树（`recursive`、`force` 默认 true） |
| `fs/copy` | `v2/FsCopyParams` | 复制（目录需 `recursive: true`） |
| `fs/watch` | `v2/FsWatchParams` | 订阅文件变更（自供 `watchId`），返回规范化 path |
| `fs/unwatch` | `v2/FsUnwatchParams` | 停止 watch |

### 5.8 命令 / 进程

| 方法 | params | 说明 |
|---|---|---|
| `command/exec` | `v2/CommandExecParams` | 在服务端沙箱内执行单条命令（不建线程/回合），输出走 `command/exec/outputDelta` |
| `command/exec/write` | `v2/CommandExecWriteParams` | 写 base64 stdin 或关闭 stdin |
| `command/exec/terminate` | `v2/CommandExecTerminateParams` | 按 processId 终止 |
| `command/exec/resize` | `v2/CommandExecResizeParams` | 调整 PTY 尺寸 |
| `process/spawn` | `v2/ProcessSpawnParams` | 脱离 Codex 沙箱在宿主机启动独立进程（实验），输出走 `process/outputDelta`、退出走 `process/exited` |
| `process/writeStdin` | `v2/ProcessWriteStdinParams` | 写 base64 stdin（实验） |
| `process/kill` | `v2/ProcessKillParams` | 按 processHandle 终止（实验） |
| `process/resizePty` | `v2/ProcessResizePtyParams` | 调整 PTY 尺寸（实验） |

### 5.9 环境 / Windows 沙箱

| 方法 | params | 说明 |
|---|---|---|
| `environment/add` | `v2/EnvironmentAddParams` | 新增/替换命名远端环境（实验） |
| `environment/info` | `v2/EnvironmentInfoParams` | 连接环境并返回 shell + 默认 cwd（file: URI，实验） |
| `environment/status` | `v2/EnvironmentStatusParams` | 读取环境状态（ready/pending/disconnected/unknown，实验） |
| `windowsSandbox/setupStart` | `v2/WindowsSandboxSetupStartParams` | 启动 Windows 沙箱配置（elevated/unelevated），完成发 `windowsSandbox/setupCompleted` |
| `windowsSandbox/readiness` | `undefined` | 查询 Windows 沙箱就绪状态 |

### 5.10 MCP

| 方法 | params | 说明 |
|---|---|---|
| `mcpServer/oauth/login` | `v2/McpServerOauthLoginParams` | 启动 MCP 服务器 OAuth 登录，完成发 `mcpServer/oauthLogin/completed` |
| `config/mcpServer/reload` | `undefined` | 从磁盘重载 MCP 配置（编辑 config.toml 后无需重启） |
| `mcpServerStatus/list` | `v2/ListMcpServerStatusParams` | 枚举 MCP 服务器（工具/资源/认证状态，cursor 分页；detail 默认 full） |
| `mcpServer/resource/read` | `v2/McpResourceReadParams` | 读取 MCP 资源（text/blob contents） |
| `mcpServer/tool/call` | `v2/McpServerToolCallParams` | 调用线程 MCP 服务器工具 |

### 5.11 账号 / 认证

| 方法 | params | 说明 |
|---|---|---|
| `account/login/start` | `v2/LoginAccountParams` | 启动登录（API key / ChatGPT 浏览器 / 设备码等模式） |
| `account/login/cancel` | `v2/CancelLoginAccountParams` | 取消进行中的登录 |
| `account/logout` | `undefined` | 登出 |
| `account/read` | `v2/GetAccountParams` | 读取账号信息 |
| `account/rateLimits/read` | `undefined` | 读取限额快照 |
| `account/rateLimitResetCredit/consume` | `v2/ConsumeAccountRateLimitResetCreditParams` | 消费 earned 限额重置 |
| `account/usage/read` | `undefined` | 读取用量 |
| `account/workspaceMessages/read` | `undefined` | 读取工作区消息 |
| `account/sendAddCreditsNudgeEmail` | `v2/SendAddCreditsNudgeEmailParams` | 通知工作区所有者限额 |
| `getAuthStatus` | `GetAuthStatusParams` | 认证状态（兼容旧接口） |

### 5.12 配置 / 外部智能体迁移

| 方法 | params | 说明 |
|---|---|---|
| `config/read` | `v2/ConfigReadParams` | 读取分层解析后的生效配置（含 opaque `desktop` 值） |
| `config/value/write` | `v2/ConfigValueWriteParams` | 写单个配置键（点分路径如 `desktop.someKey`）；与托管 requirement 冲突返回 `configRequirementReadonly` |
| `config/batchWrite` | `v2/ConfigBatchWriteParams` | 原子批量写（可选 `reloadUserConfig: true` 热重载已加载线程） |
| `configRequirements/read` | `undefined` | 读取 requirements.toml/MDM 约束（allow-lists、managed 值、网络约束等） |
| `externalAgentConfig/detect` | `v2/ExternalAgentConfigDetectParams` | 检测可迁移的外部智能体产物 |
| `externalAgentConfig/import` | `v2/ExternalAgentConfigImportParams` | 应用迁移项，返回 `importId`，进度/完成走通知 |
| `externalAgentConfig/import/recordHistory` | `v2/ExternalAgentConfigImportHistoryRecordParams` | 记录导入历史 |
| `externalAgentConfig/import/readHistories` | `undefined` | 读取导入历史与连接器候选 |

### 5.13 其它

| 方法 | params | 说明 |
|---|---|---|
| `review/start` | `v2/ReviewStartParams` | 启动内置代码审查；应答类同 `turn/start`，内联审查下发 `enteredReviewMode`/`exitedReviewMode` 条目 |
| `gitDiffToRemote` | `GitDiffToRemoteParams` | 相对远端分支的 diff |
| `getConversationSummary` | `GetConversationSummaryParams` | 会话摘要 |
| `fuzzyFileSearch` | `FuzzyFileSearchParams` | 一次性模糊文件搜索 |
| `fuzzyFileSearch/sessionStart` | `FuzzyFileSearchSessionStartParams` | 启动模糊搜索会话（实验） |
| `fuzzyFileSearch/sessionUpdate` | `FuzzyFileSearchSessionUpdateParams` | 更新查询（实验） |
| `fuzzyFileSearch/sessionStop` | `FuzzyFileSearchSessionStopParams` | 停止会话（实验） |
| `feedback/upload` | `v2/FeedbackUploadParams` | 提交反馈（返回追踪 thread id） |
| `remoteControl/enable` | `v2/RemoteControlEnableParams`（或 `null`） | 启用远端控制（实验） |
| `remoteControl/disable` | `v2/RemoteControlDisableParams`（或 `null`） | 禁用远端控制（实验） |
| `remoteControl/status/read` | `undefined` | 读取远端控制状态快照（实验） |
| `remoteControl/pairing/start` | `v2/RemoteControlPairingStartParams` | 启动配对（返回 pairingCode 等，实验） |
| `remoteControl/pairing/status` | `v2/RemoteControlPairingStatusParams` | 轮询配对是否被认领（实验） |
| `remoteControl/client/list` | `v2/RemoteControlClientsListParams` | 列出已授权控制器设备（实验） |
| `remoteControl/client/revoke` | `v2/RemoteControlClientsRevokeParams` | 吊销设备授权（实验） |

> 注：codex-ui 置顶固定使用 `threadSection/list`（定位内置 `Pinned` 分区）→ `thread/section/move`（置顶/取消）；`threadSection/move` 与 `thread/metadata/update { isPinned/sectionId }` 已随旧版本支持一并移除，不再探测回退。

## 6. 服务端 → 客户端请求（11 个）

服务端以 JSON-RPC **请求**（带 id）主动向客户端要数据/决策，客户端必须按原 id 回 `{ "id": …, "result": … }`（或 error）。这些请求主要围绕审批与交互。

| 方法 | params | 应答 result |
|---|---|---|
| `item/commandExecution/requestApproval` | `v2/CommandExecutionRequestApprovalParams` | `v2/CommandExecutionApprovalResponse { decision }` |
| `item/fileChange/requestApproval` | `v2/FileChangeRequestApprovalParams` | `v2/FileChangeApprovalResponse { decision }` |
| `item/permissions/requestApproval` | `PermissionsRequestApprovalParams` | `PermissionsRequestApprovalResponse { permissions, scope, strictAutoReview? }` |
| `item/tool/requestUserInput` | `v2/ToolRequestUserInputParams` | `v2/ToolRequestUserInputResponse { answers }`（实验） |
| `mcpServer/elicitation/request` | `v2/McpServerElicitationRequestParams` | `v2/McpServerElicitationRequestResponse { action, content, _meta }` |
| `item/tool/call` | `v2/DynamicToolCallParams` | `v2/DynamicToolCallResponse { contentItems, success }`（实验） |
| `account/chatgptAuthTokens/refresh` | `v2/ChatgptAuthTokensRefreshParams` | `v2/ChatgptAuthTokensRefreshResponse { accessToken, chatgptAccountId, chatgptPlanType }` |
| `attestation/generate` | `v2/AttestationGenerateParams`（空） | `v2/AttestationGenerateResponse { token }`（需 `requestAttestation`） |
| `currentTime/read` | `v2/CurrentTimeReadParams { threadId }` | `v2/CurrentTimeReadResponse { currentTimeAt }`（`current_time_reminder` + 外部时钟时） |
| `applyPatchApproval` | `ApplyPatchApprovalParams` | `{ decision: ReviewDecision }`（旧版，兼容保留） |
| `execCommandApproval` | `ExecCommandApprovalParams` | `{ decision: ReviewDecision }`（旧版，兼容保留） |

### 6.1 命令执行审批（推荐路径）

顺序：

1. `item/started` — 下发 `commandExecution` 条目（含 command、cwd、commandActions），`status: inProgress`。
2. `item/commandExecution/requestApproval`（请求）— 含 `threadId`、`turnId`、`itemId`、`environmentId`（旧事件为 null）、可选 `approvalId`（zsh 子命令回调的独立 UUID）、`reason`、`command`、`cwd`、`commandActions`；实验面还可含 `additionalPermissions`（绝对路径；网络为 `additionalPermissions.network.enabled`）、`proposedExecpolicyAmendment`、`proposedNetworkPolicyAmendments`、`availableDecisions`。
3. 客户端应答 `{ "decision": … }`：
   - `"accept"` / `"acceptForSession"`
   - `{ "acceptWithExecpolicyAmendment": { "execpolicy_amendment": … } }`
   - `{ "applyNetworkPolicyAmendment": { "network_policy_amendment": { "host": …, "action": "allow"|… } } }`
   - `"decline"` / `"cancel"`
4. `serverRequest/resolved` — `{ threadId, requestId }` 确认请求已解决/清除（回合开始/完成/中断时也会为未决请求补发）。
5. `item/completed` — 最终 `commandExecution` 条目，`status: completed | failed | declined` + `aggregatedOutput`/`exitCode`/`durationMs`，这是权威结果。

### 6.2 文件修改审批

1. `item/started` — 下发 `fileChange` 条目（changes diff 摘要，`status: inProgress`）。
2. `item/fileChange/requestApproval`（请求）— `itemId`、`threadId`、`turnId`、可选 `reason`、不稳定字段 `grantRoot`。
3. 应答 `{ "decision": "accept" | "acceptForSession" | "decline" | "cancel" }`。
4. `serverRequest/resolved`。
5. `item/completed` — 最终 `fileChange`，`status: completed | failed | declined`。

### 6.3 权限请求（request_permissions 工具）

`item/permissions/requestApproval` 示例（官方文档）：

```json
{ "method": "item/permissions/requestApproval", "id": 61, "params": {
    "threadId": "thr_123", "turnId": "turn_123", "itemId": "call_123",
    "environmentId": "local", "cwd": "/Users/me/project",
    "reason": "Select a workspace root",
    "permissions": { "fileSystem": { "write": ["/Users/me/project", "/Users/me/shared"] } }
} }
```

应答（只返回被授予的子集，未出现在 `result.permissions` 的请求项视为拒绝；`scope` 为 `"session"` 时跨回合保留，省略或 `"turn"` 为回合级）：

```json
{ "id": 61, "result": {
    "scope": "session",
    "permissions": { "fileSystem": { "write": ["/Users/me/project"] } }
} }
```

同一回合内已授予权限是粘性的（后续 shell 类调用自动复用，不再重复询问）；`Granular` 审批策略下若 `request_permissions: false`，独立请求被自动拒绝且不发请求。

### 6.4 MCP 表单（elicitation）

`mcpServer/elicitation/request` 支持三种 mode：

- `{ "mode": "form", "message", "requestedSchema": McpElicitationSchema }`
- `{ "mode": "openai/form", "message", "requestedSchema": JsonValue }`（需 `mcpServerOpenaiFormElicitation`，schema 为不透明 JSON，客户端负责渲染/校验，无法渲染必须 decline/cancel）
- `{ "mode": "url", "message", "url", "elicitationId" }`

应答：`{ "action": "accept", "content": … }` / `{ "action": "decline", "content": null }` / `{ "action": "cancel", "content": null }`，随后 `serverRequest/resolved`。MCP 工具审批表单的 `_meta` 会带 `codex_approval_kind: "mcp_tool_call"` 与可选 `persist` 提示。

### 6.5 动态工具调用（实验）

`thread/start` 传 `dynamicTools` 注册；被调用时：

1. `item/started` — `dynamicToolCall` 条目（`status: inProgress`，含 tool/arguments）。
2. `item/tool/call` 请求 — `{ threadId, turnId, callId, namespace, tool, arguments }`。
3. 客户端应答 `{ "contentItems": [ { "type": "inputText"|"inputImage"|"inputAudio", … } ], "success": true }`（图片/音频必须 data URL；远端 HTTP(S) 图片与非 data 音频无效）。
4. `item/completed` — 最终状态与 contentItems/success。

工具名约束：`^[a-zA-Z0-9_-]{1,128}$`；命名空间 `^[a-zA-Z0-9_-]{1,64}$`，且不得与保留命名空间（`functions`、`multi_tool_use`、`file_search`、`web`、`browser`、`image_gen`、`computer`、`container`、`terminal`、`python`、`python_user_visible`、`api_tool`、`tool_search`、`submodel_delegator` 等）冲突。

### 6.6 Attestation / 当前时间 / 令牌刷新

- `attestation/generate`：客户端回 `{ "token": "v1.<opaque>" }`；app-server 转发上游时包成 `{ "v": 1, "s": 0, "t": "v1.<opaque>" }`。app-server 自身失败时发同形包络（`s`: 1=timeout、2=request failed、3=request canceled、4=malformed response，无 `t`）。
- `currentTime/read`：`[features.current_time_reminder]` + `clock_source = "external"` 时到期发出；应答 `{ "currentTimeAt": <Unix 秒> }`；失败/取消/超时/畸形会在此回合发送模型请求前终止回合。
- `account/chatgptAuthTokens/refresh`：多账号客户端按 `previousAccountId` 提示刷新正确工作区令牌。

## 7. 服务端 → 客户端通知（72 个）

> 通知包络（`ServerNotificationEnvelope`）：`{ method, params }`，可选附加 `emittedAtMs`。可按精确方法名在 initialize 时用 `optOutNotificationMethods` 抑制。下表 params 类型同生成绑定名。

### 7.1 线程生命周期 / 状态 / 目标

| 方法 | params | 说明 |
|---|---|---|
| `thread/started` | `v2/ThreadStartedNotification { thread }` | start/resume/fork 后下发 |
| `thread/archived` | `v2/ThreadArchivedNotification` | 归档 |
| `thread/unarchived` | `v2/ThreadUnarchivedNotification` | 恢复归档 |
| `thread/deleted` | `v2/ThreadDeletedNotification` | 删除 |
| `thread/closed` | `v2/ThreadClosedNotification` | 线程卸载（无订阅 30 分钟后） |
| `thread/status/changed` | `v2/ThreadStatusChangedNotification` | 状态变化（notLoaded/idle/systemError/active+activeFlags） |
| `thread/name/updated` | `v2/ThreadNameUpdatedNotification` | 名称更新（`threadName?`） |
| `thread/goal/updated` | `v2/ThreadGoalUpdatedNotification` | 目标更新（含完整 goal 与 `turnId`） |
| `thread/goal/cleared` | `v2/ThreadGoalClearedNotification` | 目标清除 |
| `thread/settings/updated` | `v2/ThreadSettingsUpdatedNotification` | 生效的下一回合设置变化（实验） |
| `thread/tokenUsage/updated` | `v2/ThreadTokenUsageUpdatedNotification` | token 用量增量 |
| `thread/environment/connected` | `v2/EnvironmentConnectionNotification` | 环境 exec-server 连接建立（实验；不回放当前状态） |
| `thread/environment/disconnected` | `v2/EnvironmentConnectionNotification` | 环境 exec-server 连接断开（实验；不回放当前状态） |
| `thread/compacted` | `v2/ContextCompactedNotification` | 已压缩（**已弃用**，改用 `contextCompaction` 条目） |

### 7.2 回合 / 钩子 / 模型

| 方法 | params | 说明 |
|---|---|---|
| `turn/started` | `v2/TurnStartedNotification { threadId, turn }` | 回合开始（turn 无 items，status: inProgress） |
| `turn/completed` | `v2/TurnCompletedNotification { threadId, turn }` | 回合结束（completed/interrupted/failed；最终 agentMessage 摘要兜底） |
| `turn/diff/updated` | `v2/TurnDiffUpdatedNotification` | 回合级聚合 unified diff 快照 |
| `turn/plan/updated` | `v2/TurnPlanUpdatedNotification` | 计划更新（`explanation?` + `plan: [{step,status}]`） |
| `turn/moderationMetadata` | `v2/TurnModerationMetadataNotification` | 回合级审核元数据（实验） |
| `hook/started` / `hook/completed` | `v2/HookStartedNotification` / `v2/HookCompletedNotification` | 生命周期钩子起止 |
| `model/rerouted` | `v2/ModelReroutedNotification` | 后端改路由到其它模型 |
| `model/verification` | `v2/ModelVerificationNotification` | 额外账号验证（如 trustedAccessForCyber） |
| `model/safetyBuffering/updated` | `v2/ModelSafetyBufferingUpdatedNotification` | 进入安全缓冲（瞬态，不入库） |
| `rawResponse/completed` | `v2/RawResponseCompletedNotification` | 上游 Responses API 完成（仅 `experimentalRawEvents` 内部使用） |
| `rawResponseItem/completed` | `v2/RawResponseItemCompletedNotification` | 原始条目完成（内部） |

### 7.3 条目生命周期与增量

| 方法 | params | 说明 |
|---|---|---|
| `item/started` | `v2/ItemStartedNotification` | 条目开始（完整 item，含 `startedAtMs`）；`item.id` 与后续 delta 的 `itemId` 一致 |
| `item/completed` | `v2/ItemCompletedNotification` | 条目完成（权威结果，含 `completedAtMs`） |
| `item/autoApprovalReview/started` | `v2/ItemGuardianApprovalReviewStartedNotification` | [UNSTABLE] 自动审批评审开始 |
| `item/autoApprovalReview/completed` | `v2/ItemGuardianApprovalReviewCompletedNotification` | [UNSTABLE] 自动审批评审结束 |
| `item/agentMessage/delta` | `v2/AgentMessageDeltaNotification` | 智能体文本增量；同 `itemId` 按序拼接 delta 还原全文 |
| `item/plan/delta` | `v2/PlanDeltaNotification` | 计划内容增量（实验，对应 `<proposed_plan>`） |
| `item/reasoning/summaryTextDelta` | `v2/ReasoningSummaryTextDeltaNotification` | 可读推理摘要增量（`summaryIndex` 分组） |
| `item/reasoning/summaryPartAdded` | `v2/ReasoningSummaryPartAddedNotification` | 推理摘要分区边界 |
| `item/reasoning/textDelta` | `v2/ReasoningTextDeltaNotification` | 原始推理文本增量（开源模型等，`contentIndex` 分组） |
| `item/commandExecution/outputDelta` | `v2/CommandExecutionOutputDeltaNotification` | 命令 stdout/stderr 明文增量（按序拼接还原；仅 `command/exec/outputDelta` 与 `process/outputDelta` 才是 base64） |
| `item/commandExecution/terminalInteraction` | `v2/TerminalInteractionNotification` | 终端交互事件 |
| `item/fileChange/outputDelta` | `v2/FileChangeOutputDeltaNotification` | **已弃用**的 apply_patch 文本输出增量（新服务端不再发） |
| `item/fileChange/patchUpdated` | `v2/FileChangePatchUpdatedNotification` | 结构化文件修改快照（`apply_patch_streaming_events` 特性） |
| `item/mcpToolCall/progress` | `v2/McpToolCallProgressNotification` | MCP 工具调用进度 |
| `serverRequest/resolved` | `v2/ServerRequestResolvedNotification` | 服务端反向请求已解决/清除 |

### 7.4 命令 / 进程 / 文件系统

| 方法 | params | 说明 |
|---|---|---|
| `command/exec/outputDelta` | `v2/CommandExecOutputDeltaNotification` | `command/exec` 会话 stdout/stderr（base64） |
| `process/outputDelta` | `v2/ProcessOutputDeltaNotification` | `process/spawn` 会话输出（实验） |
| `process/exited` | `v2/ProcessExitedNotification` | `process/spawn` 会话退出（实验） |
| `fs/changed` | `v2/FsChangedNotification` | 被 watch 路径变化（watchId + changedPaths） |

### 7.5 错误 / 警告 / 状态

| 方法 | params | 说明 |
|---|---|---|
| `error` | `v2/ErrorNotification` | 回合中错误（可先于 turn/completed=failed 到达） |
| `warning` | `v2/WarningNotification` | 通用非致命警告（`threadId?` + `message`） |
| `guardianWarning` | `v2/GuardianWarningNotification` | 监护人安全警告 |
| `deprecationNotice` | `v2/DeprecationNoticeNotification` | 弃用提示 |
| `configWarning` | `v2/ConfigWarningNotification` | 配置解析/初始化警告（`{ summary, details?, path?, range? }`） |
| `skills/changed` | `v2/SkillsChangedNotification` | 被 watch 的技能文件变化 |
| `mcpServer/startupStatus/updated` | `v2/McpServerStatusUpdatedNotification` | MCP 服务器启动状态（starting/ready/failed/cancelled；`failureReason: reauthenticationRequired` 提示重连） |
| `mcpServer/oauthLogin/completed` | `v2/McpServerOauthLoginCompletedNotification` | OAuth 登录完成 |
| `account/updated` | `v2/AccountUpdatedNotification` | 账号更新 |
| `account/rateLimits/updated` | `v2/AccountRateLimitsUpdatedNotification` | 限额更新 |
| `account/login/completed` | `v2/AccountLoginCompletedNotification` | 登录完成 |
| `app/list/updated` | `v2/AppListUpdatedNotification` | app 列表更新 |
| `remoteControl/status/changed` | `v2/RemoteControlStatusChangedNotification` | 远端控制状态变化（新初始化客户端会先收到当前快照） |
| `externalAgentConfig/import/progress` | `v2/ExternalAgentConfigImportProgressNotification` | 迁移进度 |
| `externalAgentConfig/import/completed` | `v2/ExternalAgentConfigImportCompletedNotification` | 迁移完成（含 itemTypeResults 成败明细） |

### 7.6 模糊搜索 / 实时 / Windows

| 方法 | params | 说明 |
|---|---|---|
| `fuzzyFileSearch/sessionUpdated` | `FuzzyFileSearchSessionUpdatedNotification` | 当前查询匹配文件更新（实验） |
| `fuzzyFileSearch/sessionCompleted` | `FuzzyFileSearchSessionCompletedNotification` | 查询完成（实验） |
| `thread/realtime/started` | `v2/ThreadRealtimeStartedNotification` | 实时会话启动（实验） |
| `thread/realtime/itemAdded` | `v2/ThreadRealtimeItemAddedNotification` | 无专属类型的原始实时条目（如 handoff_request，实验） |
| `thread/realtime/transcript/delta` | `v2/ThreadRealtimeTranscriptDeltaNotification` | 实时转录增量（实验） |
| `thread/realtime/transcript/done` | `v2/ThreadRealtimeTranscriptDoneNotification` | 实时转录整段完成（实验） |
| `thread/realtime/outputAudio/delta` | `v2/ThreadRealtimeOutputAudioDeltaNotification` | 输出音频块（camelCase 字段；可独立抑制，实验） |
| `thread/realtime/sdp` | `v2/ThreadRealtimeSdpNotification` | WebRTC 远端 SDP 应答（实验） |
| `thread/realtime/error` | `v2/ThreadRealtimeErrorNotification` | 实时传输/后端错误（实验） |
| `thread/realtime/closed` | `v2/ThreadRealtimeClosedNotification` | 实时传输关闭（实验） |
| `windows/worldWritableWarning` | `v2/WindowsWorldWritableWarningNotification` | Windows 目录可写风险警告 |
| `windowsSandbox/setupCompleted` | `v2/WindowsSandboxSetupCompletedNotification` | Windows 沙箱配置完成 |

### 7.7 条目类型与生命周期规则

`ThreadItem` 是回合应答与 `item/*` 通知中的标签联合，当前支持：

| type | 关键字段 |
|---|---|
| `userMessage` | `id, clientId?, content: Array<UserInput>` |
| `agentMessage` | `id, text, phase?, memoryCitation?` |
| `plan` | `id, text` |
| `reasoning` | `id, summary: string[], content: string[]` |
| `commandExecution` | `id, pluginId?, scriptPath?, command, cwd, processId?, source, status, commandActions, aggregatedOutput?, exitCode?, durationMs?` |
| `fileChange` | `id, changes: Array<FileUpdateChange>, status` |
| `mcpToolCall` | `id, server, tool, status, arguments, appContext?, pluginId?, result?, error?, durationMs?` |
| `dynamicToolCall` | `id, namespace?, tool, arguments, status, contentItems?, success?, durationMs?` |
| `collabAgentToolCall` | `id, tool, status, senderThreadId, receiverThreadIds, prompt?, model?, reasoningEffort?, agentsStates?` |
| `subAgentActivity` | `id, kind, agentThreadId, agentPath` |
| `webSearch` | `id, query, action?, results?` |
| `imageView` | `id, path` |
| `sleep` | `id, durationMs` |
| `imageGeneration` | `id, status, revisedPrompt?, result` |
| `enteredReviewMode` / `exitedReviewMode` | `id, review` |
| `contextCompaction` | `id`（替代已弃用的 `thread/compacted` 通知） |

每个条目统一生命周期：`item/started` → 零或多个条目专属 delta → `item/completed`。

## 8. 核心数据类型

### 8.1 Thread

```ts
type Thread = {
  id: string;                    // UUIDv7
  extra: ThreadExtra | null;
  sessionId: string;             // 同一会话树的共享 session id
  forkedFromId: string | null;
  parentThreadId: string | null; // 子智能体线程才有
  preview: string;
  ephemeral: boolean;            // true 时 thread.path 为 null
  isPinned: boolean;
  historyMode: ThreadHistoryMode;
  modelProvider: string;
  createdAt: number;             // Unix 秒
  updatedAt: number;
  recencyAt: number | null;
  status: ThreadStatus;          // notLoaded | idle | systemError | active{activeFlags}
  path: string | null;           // [UNSTABLE]
  cwd: string;
  cliVersion: string;
  source: SessionSource;         // cli | vscode | exec | mcp | custom | internal | subagent | unknown
  canAcceptDirectInput: boolean | null; // 已加载线程才能非 null
  threadSource: ThreadSource | null;
  agentNickname: string | null;
  agentRole: string | null;
  gitInfo: GitInfo | null;
  name: string | null;
  turns: Array<Turn>;            // 仅 resume/rollback/fork/read(includeTurns) 等应答填充
};
```

`ThreadStatus.activeFlags`：`"waitingOnApproval" | "waitingOnUserInput"`。

### 8.2 Turn

```ts
type TurnStatus = "completed" | "interrupted" | "failed" | "inProgress";

type Turn = {
  id: string;                    // UUIDv7
  items: Array<ThreadItem>;
  itemsView: TurnItemsView;      // "notLoaded" | "summary" | "full"
  status: TurnStatus;
  error: TurnError | null;       // 仅 failed
  startedAt: number | null;      // Unix 秒
  completedAt: number | null;
  durationMs: number | null;
};

type TurnError = { message: string; codexErrorInfo: CodexErrorInfo | null; additionalDetails: string | null };
```

### 8.3 目标 / 设置 / 用量

```ts
type ThreadGoalStatus = "active" | "paused" | "blocked" | "usageLimited" | "budgetLimited" | "complete";

type ThreadGoal = {
  threadId: string; objective: string; status: ThreadGoalStatus;
  tokenBudget: number | null; tokensUsed: number; timeUsedSeconds: number;
  createdAt: number; updatedAt: number;
};

type ThreadSettings = {
  cwd: string; approvalPolicy: AskForApproval; approvalsReviewer: ApprovalsReviewer;
  sandboxPolicy: SandboxPolicy; activePermissionProfile: ActivePermissionProfile | null;
  model: string; modelProvider: string; serviceTier: string | null;
  effort: ReasoningEffort | null; summary: ReasoningSummary | null;
  collaborationMode: CollaborationMode; multiAgentMode: MultiAgentMode; personality: Personality | null;
};

type ThreadTokenUsage = {
  total: TokenUsageBreakdown; last: TokenUsageBreakdown; modelContextWindow: number | null;
};
// TokenUsageBreakdown: totalTokens/inputTokens/cachedInputTokens/cacheWriteInputTokens/outputTokens/reasoningOutputTokens
```

### 8.4 输入与内容

```ts
type UserInput =
  | { "type": "text", text: string, text_elements: Array<TextElement> }
  | { "type": "image", detail?: ImageDetail, url: string }
  | { "type": "localImage", detail?: ImageDetail, path: string }
  | { "type": "audio", url: string }
  | { "type": "localAudio", path: string }
  | { "type": "skill", name: string, path: string }
  | { "type": "mention", name: string, path: string };

type ContentItem =
  | { "type": "input_text", text: string }
  | { "type": "input_image", image_url: string, detail?: ImageDetail }
  | { "type": "input_audio", audio_url: string }
  | { "type": "output_text", text: string };
```

### 8.5 常用枚举

```ts
type AskForApproval = "untrusted" | "on-request"
  | { "granular": { sandbox_approval: boolean; rules: boolean; skill_approval: boolean; request_permissions: boolean; mcp_elicitations: boolean } }
  | "never";
type SandboxMode = "read-only" | "workspace-write" | "danger-full-access";
type Personality = "none" | "friendly" | "pragmatic";
type MessagePhase = "commentary" | "final_answer";
type ReasoningEffort = string;                       // 文档见 platform.openai.com reasoning 指南
type CollaborationMode = { mode: ModeKind; settings: Settings };
type MultiAgentMode = { "custom": string } | "explicitRequestOnly" | "proactive"; // 已弃用，Ultra effort 替代
type ThreadMemoryMode = "enabled" | "disabled";
type TurnItemsView = "notLoaded" | "summary" | "full";
type SortDirection = "asc" | "desc";
type ThreadSortKey = "created_at" | "updated_at" | "recency_at";
type ThreadSourceKind = "cli" | "vscode" | "exec" | "appServer" | "subAgent"
  | "subAgentReview" | "subAgentCompact" | "subAgentThreadSpawn" | "subAgentOther" | "unknown";
```

## 9. 错误码与错误处理

### 9.1 JSON-RPC 错误码

标准 JSON-RPC 2.0 错误码（规范定义）：

| code | 含义 |
|---|---|
| `-32700` | 解析错误（Parse error） |
| `-32600` | 非法请求（Invalid Request） |
| `-32601` | 方法不存在（Method not found） |
| `-32602` | 参数非法（Invalid params） |
| `-32603` | 内部错误（Internal error） |

### 9.2 app-server 特定错误

| 现象 | 说明 |
|---|---|
| `-32001` `"Server overloaded; retry later."` | 请求入口饱和，可重试（指数退避 + 抖动） |
| `-32600` | 分页线程独占写冲突：另一进程持有线程时，`thread/resume`/`thread/archive`/`thread/delete` 失败；旧版本以「unknown variant」形式报方法未知（两种形态均出现） |
| `-32601` | 方法不存在；或分页线程存储不支持某能力（如 item 分页、paginated 历史下 thread/rollback） |
| `"Not initialized"` | 连接未完成 initialize 就调用其它方法 |
| `"Already initialized"` | 同连接重复 initialize |
| `<descriptor> requires experimentalApi capability` | 未开启实验 API 却调用实验方法/字段/枚举变体（descriptor 形如 `mock/experimentalMethod`、`thread/start.mockExperimentalField`、`askForApproval.granular`） |
| `configRequirementReadonly` | `config/value/write` 与托管 requirement 重叠被拒 |

### 9.3 回合错误（error 通知 / turn.status=failed）

`error` 通知携带 `{ error: { message, codexErrorInfo?, additionalDetails? }, willRetry, threadId, turnId }`；`codexErrorInfo`（`CodexErrorInfo`）常见值：

- `contextWindowExceeded` / `sessionBudgetExceeded` / `usageLimitExceeded`
- `serverOverloaded` / `internalServerError` / `badRequest` / `unauthorized` / `sandboxError`
- `httpConnectionFailed { httpStatusCode? }` / `responseStreamConnectionFailed { httpStatusCode? }` / `responseStreamDisconnected { httpStatusCode? }` / `responseTooManyFailedAttempts { httpStatusCode? }`
- `activeTurnNotSteerable { turnKind }`（回合为 `/review`、手动 `/compact` 等不可 steer 类型时，`turn/start`/`turn/steer` 被拒）
- `threadRollbackFailed` / `cyberPolicy` / `other`

## 10. 实验 API 门控

- 生成 schema 时：`generate-ts`/`generate-json-schema` 默认只含稳定面，`--experimental` 才含实验方法/字段。
- 运行时：`initialize` 里 `capabilities.experimentalApi: true` 一次性协商；缺省视为 `false`。
- 未开启时调用实验方法/字段，返回 `"<descriptor> requires experimentalApi capability"`。
- 门控粒度覆盖方法、字段、枚举变体三类。

## 11. 完整 Schema 再生成

```powershell
# TypeScript 绑定（本机 0.149.0）
codex app-server generate-ts --experimental --out <DIR>

# JSON Schema 全量包（含全部类型定义与字段描述，最权威机器可读源）
codex app-server generate-json-schema --experimental --out <DIR>
```

生成物组织：根目录为基础类型 + `ClientRequest`/`ServerNotification`/`ServerRequest`/`ClientNotification` 四张总表；`v1/` 为旧版 `InitializeParams`/`InitializeResponse`（仅要求 `clientInfo`）；`v2/` 为当前版全部方法参数/应答/通知类型；`codex_app_server_protocol.schemas.json` 与 `codex_app_server_protocol.v2.schemas.json` 为全量 JSON Schema。

升级 codex 后建议：重新生成 → diff 客户端类型 → 跑 `cargo test`（`src-tauri/tests/app_server_integration.rs`，设置 `CODEX_BIN` 指向新二进制）验证握手/回合/目标/记忆/设置/搜索全流程。

## 12. 附录：真实报文示例

### 12.1 握手

```json
→ { "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "clientInfo": { "name": "codex-ui", "title": "Codex UI", "version": "0.1.2" },
    "capabilities": { "experimentalApi": true, "requestAttestation": false } } }
← { "id": 1, "result": { "userAgent": "…", "codexHome": "C:\\Users\\Admin\\.codex",
    "platformFamily": "windows", "platformOs": "windows" } }
→ { "jsonrpc": "2.0", "method": "initialized" }
```

### 12.2 新建会话（thread/start）

```json
→ { "jsonrpc": "2.0", "id": 10, "method": "thread/start", "params": {
    "cwd": "D:\\codex\\codex-ui", "approvalPolicy": "untrusted", "sandbox": "read-only" } }
← { "id": 10, "result": { "thread": { "id": "thr_…", "preview": "",
    "modelProvider": "openai", "createdAt": 1730910000, … }, "model": "…",
    "cwd": "D:\\codex\\codex-ui", "approvalPolicy": "untrusted", "sandbox": "…", … } }
← { "method": "thread/started", "params": { "thread": { … } } }
```

### 12.3 发送回合（turn/start）

```json
→ { "jsonrpc": "2.0", "id": 11, "method": "turn/start", "params": {
    "threadId": "thr_…",
    "input": [ { "type": "text", "text": "只回复 OK 两个字母", "text_elements": [] } ] } }
← { "id": 11, "result": { "turn": { "id": "turn_…", "items": [], "itemsView": "full",
    "status": "inProgress", "error": null, "startedAt": … } } }
← { "method": "turn/started", "params": { "threadId": "thr_…", "turn": { … } } }
← { "method": "item/started", "params": { "threadId": "thr_…", "turnId": "turn_…",
    "item": { "type": "agentMessage", "id": "item_…", "text": "", "phase": "commentary" },
    "startedAtMs": … } }
← { "method": "item/agentMessage/delta", "params": { "threadId": "thr_…", "turnId": "turn_…",
    "itemId": "item_…", "delta": "OK" } }
← { "method": "item/completed", "params": { "threadId": "thr_…", "turnId": "turn_…",
    "item": { "type": "agentMessage", "id": "item_…", "text": "OK", "phase": "final_answer" },
    "completedAtMs": … } }
← { "method": "turn/completed", "params": { "threadId": "thr_…",
    "turn": { "id": "turn_…", "status": "completed", … } } }
```

### 12.4 目标挂载 / 更新 / 清除

```json
→ { "jsonrpc": "2.0", "id": 20, "method": "thread/goal/set", "params": {
    "threadId": "thr_…", "objective": "修复 CI 失败" } }
← { "id": 20, "result": { "goal": { "threadId": "thr_…", "objective": "修复 CI 失败",
    "status": "active", "tokenBudget": null, "tokensUsed": 0, … } } }
← { "method": "thread/goal/updated", "params": { "threadId": "thr_…", "turnId": "turn_…",
    "goal": { … "status": "complete" … } } }
→ { "jsonrpc": "2.0", "id": 21, "method": "thread/goal/clear", "params": { "threadId": "thr_…" } }
← { "id": 21, "result": { "cleared": true } }
← { "method": "thread/goal/cleared", "params": { "threadId": "thr_…" } }
```

### 12.5 审批流（命令执行）

```json
← { "method": "item/started", "params": { "threadId": "thr_…", "turnId": "turn_…",
    "item": { "type": "commandExecution", "id": "item_…", "command": "git pull",
      "cwd": "D:\\codex\\codex-ui", "status": "inProgress", "commandActions": [ … ] },
    "startedAtMs": … } }
← { "method": "item/commandExecution/requestApproval", "id": 30, "params": {
    "threadId": "thr_…", "turnId": "turn_…", "itemId": "item_…",
    "environmentId": "local", "command": "git pull", "cwd": "D:\\codex\\codex-ui",
    "commandActions": [ … ], "availableDecisions": [ "accept", "decline", "cancel" ] } }
→ { "jsonrpc": "2.0", "id": 30, "result": { "decision": "accept" } }
← { "method": "serverRequest/resolved", "params": { "threadId": "thr_…", "requestId": 30 } }
← { "method": "item/completed", "params": { "threadId": "thr_…", "turnId": "turn_…",
    "item": { "type": "commandExecution", "id": "item_…", "command": "git pull",
      "status": "completed", "exitCode": 0, "aggregatedOutput": "…", "durationMs": 812 },
    "completedAtMs": … } }
```

---

## 参考

- 官方 README：`openai/codex` → `codex-rs/app-server/README.md`（rust-v0.149.0）
- 生成绑定：`codex app-server generate-ts --experimental` / `generate-json-schema --experimental`（codex-cli 0.149.0）
- 本地实测：`src-tauri/tests/app_server_integration.rs`、`docs/协议盘点.md`
