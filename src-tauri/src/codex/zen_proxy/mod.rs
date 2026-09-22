//! Zen 本地代理：把 codex 的 OpenAI Responses 请求翻译为 OpenCode Zen 的
//! Chat Completions 请求并转发；响应反向翻译回 Responses 格式（含流式 SSE）。
//!
//! - Zen base URL 与 User-Agent 硬编码（模块私有，不暴露到前端）。
//! - API Key 不固定：读取入站请求的 `Authorization` 头原样转发。
//! - 仅绑定回环地址，专供 codex-ui 自身使用，无额外鉴权。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
#[cfg(test)]
use axum::Json;
use axum::Router;
use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};
use tokio::task::AbortHandle;

use crate::codex::session_log::SessionLog;
use crate::codex::zen_trace::{TraceCall, TraceSink};

/// Zen 本地代理默认端口（可在设置页修改）。
pub(crate) const DEFAULT_ZEN_PROXY_PORT: u16 = 18080;
/// 模拟 opencode 桌面客户端的固定识别头（随请求发往 Zen）。
const OPENCODE_CLIENT: &str = "desktop";
const OPENCODE_PROJECT: &str = "global";
/// OpenCode Zen 默认上游 base URL（可在设置页修改，空/非法时回退此值）。
pub(crate) const DEFAULT_ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";
/// OpenCode 免费层的**请求体门禁**要求请求里出现这些工具名（opencode 客户端的内置工具）：
/// 只在 `tools` 里补声明、不提供任何实现，模型真去调用时由 codex 侧以 `unsupported call`
/// 裁决（代理不做特例过滤）。门禁只校验名字，不看描述与参数。
const ZEN_REQUIRED_TOOL_NAMES: [&str; 6] = ["bash", "edit", "glob", "grep", "read", "write"];
/// 上述假工具的描述：门禁只看名字，这段文案负责劝模型别真调用。
const ZEN_FAKE_TOOL_DESCRIPTION: &str = "这是弃用的工具，请勿调用";
/// 门禁要求的 `max_tokens`（与 `messages` 同级）：入站请求没有自己的输出预算时补上这个值。
/// **只对 Zen 上游生效**（与 [`ZEN_REQUIRED_TOOL_NAMES`] 同受 `zen_body_patch` 约束，
/// 判定见 [`is_zen_upstream`]）。
const ZEN_MAX_TOKENS: u64 = 32_000;
/// 请求上游时固定的 User-Agent（与 opencode 官方客户端一致）。
const ZEN_USER_AGENT: &str = "opencode/1.18.29 ai-sdk/provider-utils/4.0.23 runtime/node.js/24";
/// 反向代理时不应转发的逐跳请求/响应头。
const HOP_BY_HOP_HEADERS: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// 畸形工具调用被拦截时写入 `response.failed` 的错误码。
const CODE_MALFORMED_TOOL_CALL: &str = "malformed_tool_call_arguments";
/// 上游随流下发 `error`、或读取中断时写入 `response.failed` 的错误码。
const CODE_UPSTREAM_STREAM_ERROR: &str = "upstream_stream_error";
/// 补齐悬空工具调用时代入的工具结果文本。
const MISSING_TOOL_OUTPUT_TEXT: &str = "该工具调用未执行（参数非法或调用被中止），请重新发起。";
/// `finish_reason` 之后等待尾部分片（usage 等）的宽限；超时按现状收尾，避免挂住回合。
const USAGE_GRACE: Duration = Duration::from_secs(3);
/// `zen_proxy.stream_summary` 里记录的 delta 键上限（去重后按字典序取前若干个）。
const DELTA_KEY_LIMIT: usize = 16;
/// 翻译分支的请求体上限：长会话实测已近 1MB，axum 默认 2MB 会撞 413。
const RESPONSES_BODY_LIMIT: usize = 64 * 1024 * 1024;
/// 端口刚被上一实例释放时的绑定重试次数与间隔（同端口换上游会立即重启代理）。
const BIND_RETRY: u32 = 20;
const BIND_RETRY_INTERVAL: Duration = Duration::from_millis(50);
/// 一次转发最多发出的上游请求数（可选字段降级 + reasoning_content 开关各一次修复）。
const FORWARD_MAX_ATTEMPTS: usize = 3;

/// 口嗨自动续跑：**单次入站请求**内最多注入几次续跑提醒（等价于单请求最多
/// `NUDGE_MAX_INJECTIONS + 1` 次上游调用：首轮 + 每次催办一轮）。
///
/// 计数只活在一次请求的循环里，不跨请求、也没有时间窗口：每个「纯文本 + 零工具 + 无收尾
/// 标签」的终局都会被催（此前是「同一会话连续 2 次后 10 分钟内不再催」，会把真正的口嗨一起
/// 静默掉）。循环里每次迭代要么收尾、要么注入一次，因此原先的 `NUDGE_MAX_PASSES` 与本上限
/// 完全等价，已删除——只留这一个旋钮。
const NUDGE_MAX_INJECTIONS: usize = 4;
/// 注入给上游的续跑提醒。只出现在发给上游的历史里，不会进入 codex 自己的记录，
/// 因此应用聊天里看不到这条消息（模型按提醒回出的 [`TASK_COMPLETED_MARKER`] 标签会被
/// [`TagStripper`] 整段剥离，见该类型文档）。
///
/// **两条路径强收尾**：默认模式没有独立判定请求可用了（Zen 免费层会拒那条后台
/// `chat/completions`），所以判定搬进原对话——被催办时模型只有两条路：
/// ① 仍有可执行的剩余工作 → 必须实际调用工具完成，**不得用文字描述、计划或承诺代替工具调用**，
/// 也不得直接甩出完成标签逃避执行；
/// ② 任务已结束、无需继续或无法继续 → 不复述上一条回复正文、不重复此前已执行过的工具操作，
/// 用一行 [`TASK_COMPLETED_MARKER`] 成对标签宣告结束，代理据此（纯代码前缀判据，
/// 见 [`is_task_completed`]）直接收尾。
///
/// 第②条显式排除「重复执行已完成的操作」——操作型请求（如「请 git 提交并推送」）若在更早
/// 回合已经做完，被催办也不能让模型再提交/再推送一次；并且要求**本轮只回这一行标签**
/// （「标签外不得有任何其他字符」）——上一轮正文已经展示过，收尾轮再补正文只会是噪音。
/// 末句给出标签的**结构不变量**（成对闭合、独占一行、不换行、本轮最多一个、不进代码块、
/// 不改写标签），未来的代理层过滤/分桶只依赖这些结构与标签前缀，不依赖载荷词汇
/// （载荷保持固定短词，只服务日志阅读）。
const NUDGE_TEXT: &str = "【自动续跑】\n你上一条回复未调用任何工具就结束了回合。现在必须立即从以下两条路径中选择并执行。\n\n1. 仍有可执行的剩余工作：\n- 必须实际调用工具完成剩余工作；不得用文字描述、计划或承诺代替工具调用。\n- 只要还有可执行工作，就不得直接输出完成标签来逃避执行。\n- 完成剩余工作后，按第 2 条标签格式收尾。\n\n2. 任务已结束、无需继续或无法继续：\n- 不复述上一条回复正文。\n- 不重复此前已执行过的工具操作（例如已 git commit/push 的，不得再次提交/推送）。\n- 仅输出一行闭合标签：\n<zen_task_completed>已完成</zen_task_completed>\n其中“已完成”必须替换为以下四者之一：已完成、无需改动、已放弃、做不下去。\n- 标签必须成对闭合、独占一行、不换行；本轮最多一个；不得放入代码块；不得改写标签；标签外不得有任何其他字符。";
/// 计划模式专用的续跑提醒：计划模式的交付物是「计划」而不是「动手改代码」，
/// 所以不能沿用 [`NUDGE_TEXT`] 的执行口径（否则会把模型逼去在计划模式里改代码）。
/// **排除式三选一**：被催办时先判两种「不需要给方案」的情况——用户已放弃
/// （[`PLAN_CANCEL_MARKER`]）与「问题本身无法或无需产出实现计划」
/// （[`PLAN_UNACHIEVABLE_MARKER`]，如事实问题/纯查询/闲聊），两者都不成立才必须把完整方案
/// 落进 `<proposed_plan>` 骨架。骨架必须带完整形态：弱模型经常只把计划写成普通 Markdown，
/// 而 codex 只在终局文本里出现 `<proposed_plan>` 包裹时才生成计划条目（否则应用里没有
/// 「计划已就绪」），所以提醒里直接给出骨架，并要求标签原样保留、成对闭合、各自独占一行、
/// 不换行、本轮最多只写一个、不要放进代码块、不要改写标签。
/// 「计划已被认可、无需改动、保持现状」不属于「无法/无需计划」：这同样是一个评估结论，
/// 应把该结论或重申的原计划写进 `<proposed_plan>` 收尾，而不是逃到非方案标签。
const PLAN_NUDGE_TEXT: &str = "【自动续跑】你上一条回复没有交付计划就结束了回合。请先判断下面两种不需要给方案的情况是否成立，都不成立时才必须给出完整方案。\n\n标签必须原样保留、成对闭合、各自独占一行、不换行、本轮最多只写一个，不要放进代码块，不要改写标签，也不要只写正文：\n- 用户已经放弃这个计划（例如让你不要再处理、先不做了）：不要重新给方案，也不要只写正文，直接用一行 <zen_plan_cancelled>已放弃</zen_plan_cancelled> 收尾（标签里只写「已放弃」这三个字，不要写别的说明）。\n- 问题本身无法或无需产出实现计划（例如「1+1=？」这类事实问题、纯查询或闲聊，本就不产出计划这种交付物）：不要先回答正文，只用一行 <zen_plan_unachievable>无法计划</zen_plan_unachievable> 收尾（标签里只写「无法计划」这四个字，不要写别的说明）。\n\n以上两种情况都不成立时，必须想办法把完整方案写进下面这个结构里：\n\n<proposed_plan>\n# 计划标题\n- 步骤 1\n- 步骤 2\n</proposed_plan>\n\n即便你的结论是无需改动、保持现状或原有计划已经可以，也要把该结论（或重申原计划）写进这个结构里收尾。";
/// 计划模式开发者消息的开头标签：codex 把当前协作模式拼进 developer 条目（形如
/// `…<collaboration_mode># Plan Mode (Conversational)\r\n…</collaboration_mode>`）。
const COLLABORATION_MODE_TAG: &str = "<collaboration_mode>";
/// 计划模式的标题行判据（大小写不敏感，只匹配块内**第一个非空行**）：codex 0.154 实测
/// 标题为 `# Plan Mode (Conversational)`（旧版本是 `# Collaboration Mode: Plan`，见下一条）。
/// 计划模式下的口嗨判据完全由代码给出：终局文本含计划标签（见 [`PLAN_OUTPUT_MARKER`]）
/// 即「计划已交付」，否则注入 [`PLAN_NUDGE_TEXT`] 催它给计划——**不调用 AI 判定**。
const PLAN_MODE_HEADING_MARKER: &str = "plan mode";
/// 旧版 codex 的计划模式标题（`# Collaboration Mode: Plan`），保留兼容分支。
const PLAN_MODE_HEADING_MARKER_LEGACY: &str = "collaboration mode: plan";
/// 计划产物的包裹标签（协议 `item/plan/delta` 与之对应）：**计划模式**下出现即视为
/// 计划已交付（全程纯代码判据）；默认模式不据此短路，也没有别的判定可用——默认模式的收尾
/// 靠模型自己回出 [`TASK_COMPLETED_MARKER`]。
const PLAN_OUTPUT_MARKER: &str = "<proposed_plan";
/// 计划模式下「用户已取消这个计划」的约定标记（由 [`PLAN_MODE_CONTRACT_TEXT`] 与
/// [`PLAN_NUDGE_TEXT`] 教给模型，教学形态是成对标签 + 固定短词
/// `<zen_plan_cancelled>已放弃</zen_plan_cancelled>`，与计划产物标签 [`PLAN_OUTPUT_MARKER`]
/// 同形：`<proposed_plan>` = 计划已交付、本标记 = 计划已取消）。标签由 [`TagStripper`] 整段剥离，
/// 不进聊天、不进 codex 会话（载荷只在日志 `nudge_skipped 标签内容=` 里回看）。
/// **计划模式**下终局文本出现即视为计划话题已终结、直接收尾：不再续跑，也**不生成计划条目**、
/// 不弹「计划已就绪」。
/// 判定与 [`PLAN_OUTPUT_MARKER`] 同口径（大小写不敏感 + 前缀匹配）：兼容大写、缺闭合标签
/// 等写法；计划标签判据优先于本判据。**不兼容旧的无前缀写法**（`<cancelled_plan>`）。
const PLAN_CANCEL_MARKER: &str = "<zen_plan_cancelled";
/// 计划模式下「问题本身无法或无需产出实现计划」的约定标记（由 [`PLAN_MODE_CONTRACT_TEXT`] 与
/// [`PLAN_NUDGE_TEXT`] 教给模型，教学形态是成对标签 + 固定短词
/// `<zen_plan_unachievable>无法计划</zen_plan_unachievable>`，与 [`PLAN_OUTPUT_MARKER`] /
/// [`PLAN_CANCEL_MARKER`] 同形）：`<proposed_plan>` = 计划已交付、`<zen_plan_cancelled>` =
/// 计划已取消、本标记 = 无法/无需计划（如「1+1=？」这类事实问题、纯查询或闲聊，本就不产出计划
/// 这种交付物）。「计划已被认可、无需改动、保持现状」**不属于**本标记：那是评估结论，
/// 应写进 `<proposed_plan>` 收尾，不逃到非方案标签。标签同样由 [`TagStripper`] 剥离。
/// **计划模式**下终局文本出现即视为计划话题已终结、直接收尾：不再续跑，也**不生成计划条目**。
/// 判定与 [`PLAN_OUTPUT_MARKER`] 同口径（大小写不敏感 + 前缀匹配）：兼容大写、缺闭合标签
/// 等写法；计划标签判据优先于本判据。**不兼容旧的无前缀写法**（`<unachievable_plan>`）。
const PLAN_UNACHIEVABLE_MARKER: &str = "<zen_plan_unachievable";
/// **默认模式**下「任务已经结束」的约定标记（由首轮教学的 [`DEFAULT_MODE_CONTRACT_TEXT`] 与
/// [`NUDGE_TEXT`] 教给模型，教学形态是成对标签 + 固定短词
/// `<zen_task_completed>已完成</zen_task_completed>`）：默认模式的判定搬进原对话——
/// 代理不再发独立的判定请求（Zen 免费层会以 `FreeTierError: OpenCode's free tier can only be
/// used from within OpenCode` 拒绝那条后台 `chat/completions`），只看模型自己在终局文本里回了
/// 什么。
///
/// 命名：小写 + 下划线 + **`zen_` 前缀**（自研标签统一前缀：本标记用 `zen_task_`，计划模式的
/// 两个出口用 `zen_plan_`）；`<proposed_plan>` 是唯一例外——它是 codex 侧约定（协议
/// `item/plan/delta` 与之对应），改名后 codex 就不再生成计划条目。
///
/// 判据（见 [`is_task_completed`]）：大小写不敏感 + 前缀匹配，缺闭合标签同样命中；
/// **不解析载荷**（载荷是自由文本，只给人和未来的软映射分桶用）；**不兼容旧的无前缀写法**
/// （`<task_completed>`）。
///
/// 结构不变量（写进 [`NUDGE_TEXT`]，未来代理层过滤/分桶只依赖这些）：成对闭合、独占一行、
/// 不换行、一整轮最多一个、不放进代码块、不改写标签。催办轮（上一轮正文已经展示过）额外要求
/// 「标签外不得有任何其他字符」——收尾轮只回这一行标签；首轮教学契约不写这一条，因为它会把
/// 模型的结论正文一起禁掉（契约只要求「标签独占一行、该行不再有别的字符、正文写在标签之前」）。
/// 有了这些结构，过滤 = 删掉整段闭合标签（缺闭合时删到该行行尾），与载荷内容无关；分桶 =
/// 对载荷首词做软映射（已完成/无需改动/已放弃/无法继续 → 枚举），映射不到落 `unknown`。
/// 若将来真需要机器可读的强枚举，优先加**第二个标签名**（沿用 plan 系「词表在标签名里、
/// 载荷自由」的风格），而不是把枚举塞进载荷。
///
/// 标签**不会进应用**：模型回出的标签由 [`TagStripper`] 整段剥离（载荷只在日志
/// `nudge_skipped 标签内容=` 里回看），标签之前的正文原样保留。默认模式下终局文本出现即视为
/// 任务已终结、直接收尾；计划模式不使用本判据。
const TASK_COMPLETED_MARKER: &str = "<zen_task_completed";
/// 三个自研标签的**闭标签**：剥离时按「开标签 → 对应闭标签」的结构剪掉整段，
/// 载荷写什么、是不是固定短词都不参与匹配（见 [`TagStripper`]）。
const TASK_COMPLETED_CLOSE: &str = "</zen_task_completed>";
const PLAN_CANCEL_CLOSE: &str = "</zen_plan_cancelled>";
const PLAN_UNACHIEVABLE_CLOSE: &str = "</zen_plan_unachievable>";
/// 结构化剥离用的「开标签前缀 + 闭标签」对照表。
const ZEN_TAG_PAIRS: [(&str, &str); 3] = [
    (TASK_COMPLETED_MARKER, TASK_COMPLETED_CLOSE),
    (PLAN_CANCEL_MARKER, PLAN_CANCEL_CLOSE),
    (PLAN_UNACHIEVABLE_MARKER, PLAN_UNACHIEVABLE_CLOSE),
];
/// 日志里记标签载荷时的截断长度与条数上限（`nudge_skipped 标签内容=`）。
const TAG_PAYLOAD_CHARS: usize = 60;
const TAG_PAYLOAD_ENTRIES: usize = 4;
/// 协议登记表里承认的协作模式取值（与协议 `ModeKind` 一致）：其余取值一律不登记。
const KNOWN_MODES: [&str; 2] = ["plan", "default"];
/// 协议登记表的条目上限：超过即整体清空（模式每轮 `turn/start` 都会重新登记，
/// 清空只会让极少数线程临时退回关键词兜底，不会让表无界增长）。
const THREAD_MODE_LIMIT: usize = 1024;
/// `zen_proxy.request` 的 `mode_src` 取值：模式来自协议登记表（权威）/ 关键词兜底判据。
const MODE_SRC_REGISTRY: &str = "registry";
const MODE_SRC_HEURISTIC: &str = "heuristic";
/// 会话标题生成请求（应用发起的后台临时线程）提示词的固定前缀，用于识别并整轮放行。
/// 与 `src/composables/useCodex/threads.ts` 的 `autoTitleThread` 提示词一致，改那句话
/// 时必须同步这里，否则标题线程会重新被口嗨检测拦住（标题必然「纯文本 + 零工具调用」，
/// 每次都会白花一轮续跑提醒）。
const TITLE_TASK_PREFIX: &str = "给下面用户消息生成一个不超过 30 字的中文会话标题";

/// **首轮教学**（默认模式）：把收尾契约追加到发给上游的 `instructions` 尾部，让模型第一轮就按
/// 约定收尾——守约定的模型首轮即带回 [`TASK_COMPLETED_MARKER`]，那条「纯文本 + 零工具」终局
/// 就不必再多打一轮上游换标签（催办提醒仍保留为第二道，见 [`NUDGE_TEXT`]）。
/// 与提醒的关系：措辞可以不同，但标签字面量与结构不变量必须一致（成对闭合、独占一行、不换行、
/// 本轮最多一个、不进代码块、不改写标签）——单测对两者都做断言，防半改。**只有催办提醒**才写
/// 「标签外不得有任何其他字符」：契约必须让模型照常输出结论正文，只要求收尾标签自成一行、
/// 该行不再有别的字符（正文写在标签之前）。
/// 准入见 [`contract_injection_eligible`]：只给「流式 + 声明了工具 + 非会话标题线程」的请求
/// 注入；非流式没有催办路径可消费，标题线程只产出标题，压缩/摘要类后台请求也通常不带工具。
const DEFAULT_MODE_CONTRACT_TEXT: &str = "【回合收尾约定】当你结束回合、且本轮没有调用任何工具时：任务已全部完成就把结论写在正文里，然后用一行 <zen_task_completed>已完成</zen_task_completed> 收尾——标签里只写这四个词之一：已完成／无需改动／已放弃／做不下去，不要写别的说明；标签必须成对闭合、独占一行、不换行；本轮最多一个；不得放入代码块；不得改写标签；标签那一行不得再有任何其他字符（结论正文写在标签之前）。还有没做完的工作必须实际调用工具继续做，不要只写「接下来我会…」这类承诺；不要重复执行更早回合已经用工具做过的操作。";
/// **首轮教学**（计划模式）：只前置两个「不需要给方案」的出口标签，不前置「必须给完整方案」——
/// 计划模式的正常形态是 chat your way，强制口径留在 [`PLAN_NUDGE_TEXT`] 里，避免把中间闲聊
/// 回合逼出假方案。边界（「无需改动、保持现状、原计划已认可」属于评估结论、要写进
/// `<proposed_plan>`、不算 unachievable）必须写在这里，否则会重现 2026-09-18 那次误逃。
const PLAN_MODE_CONTRACT_TEXT: &str = "【计划模式收尾约定】若用户已放弃这个计划（让你不要再处理、先不做了），用一行 <zen_plan_cancelled>已放弃</zen_plan_cancelled> 收尾（标签里只写「已放弃」，不要写别的说明）；若问题本身无法或无需产出实现计划（事实问题、纯查询、闲聊），用一行 <zen_plan_unachievable>无法计划</zen_plan_unachievable> 收尾（标签里只写「无法计划」，不要写别的说明）；其余情况照常把完整方案写进 <proposed_plan>（「无需改动、保持现状、原计划已认可」属于评估结论，要写进 <proposed_plan>，不算 unachievable）。这两个出口标签必须成对闭合、独占一行、不换行；本轮最多一个；不得放入代码块；不得改写标签；标签那一行不得再有任何其他字符。";

/// 「线程 id → 协作模式」协议登记表（内存态，进程内共享）：由 app-server 侧的真实来源
/// （`turn/start` / `thread/settings/update` 的参数、`thread/settings/updated` 通知）更新，
/// 供 Zen 代理按入站请求头 `session-id`（= codex 线程 id）直接取用。
///
/// 存在的理由：codex 会把**历次**协作模式块都留在请求历史里，切回默认模式后历史里仍有旧的
/// `<collaboration_mode># Plan Mode…</collaboration_mode>`，只按关键词判定就会把默认模式误判成
/// 计划模式（实测某线程 codex 侧 `collaboration_mode_kind=default` 的回合，代理仍按 plan 分流）。
/// 登记表记的是「应用实际请求 / 服务端实际生效」的模式，不需要猜。
#[derive(Debug, Default)]
pub(crate) struct ThreadModeRegistry {
    modes: Mutex<HashMap<String, String>>,
}

impl ThreadModeRegistry {
    /// 值是否是协议承认的协作模式（`plan` / `default`）。
    fn is_known_mode(mode: &str) -> bool {
        KNOWN_MODES.contains(&mode)
    }

    /// 登记一个线程的协作模式；空白线程 id 或非 `plan`/`default` 的取值一律忽略。
    /// 返回是否发生了实际变化（便于调用方按需记日志）；表达到上限时整体清空后再插入。
    pub(crate) fn record(&self, thread_id: &str, mode: &str) -> bool {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() || !Self::is_known_mode(mode) {
            return false;
        }
        let Ok(mut map) = self.modes.lock() else {
            return false;
        };
        if map.len() >= THREAD_MODE_LIMIT && !map.contains_key(thread_id) {
            map.clear();
        }
        let previous = map.insert(thread_id.to_string(), mode.to_string());
        previous.as_deref() != Some(mode)
    }

    /// 查一个线程登记的模式；未登记（或登记表不可用）返回 None，调用方退回关键词兜底判据。
    pub(crate) fn mode(&self, thread_id: &str) -> Option<String> {
        let map = self.modes.lock().ok()?;
        map.get(thread_id).cloned()
    }
}

/// 从 app-server 的请求/通知参数里取出「线程 id + 协作模式」，供 [`ThreadModeRegistry`] 登记。
///
/// 覆盖两种形状：`turn/start` 与 `thread/settings/update` 的 `{ threadId, collaborationMode }`、
/// `thread/settings/updated` 通知的 `{ threadId, threadSettings.collaborationMode }`。
/// 缺线程 id、模式缺失（如 `collaborationMode: null`）或不是 `plan` / `default` 时返回 None，
/// 调用方据此保持已有登记不变（协作模式在协议里是粘滞的，缺失表示沿用上一回合）。
pub(crate) fn thread_mode_from_params(params: &Value) -> Option<(String, String)> {
    let thread_id = params.get("threadId")?.as_str()?.trim().to_string();
    if thread_id.is_empty() {
        return None;
    }
    let mode = params
        .pointer("/collaborationMode/mode")
        .or_else(|| params.pointer("/threadSettings/collaborationMode/mode"))
        .and_then(Value::as_str)?;
    if !ThreadModeRegistry::is_known_mode(mode) {
        return None;
    }
    Some((thread_id, mode.to_string()))
}

/// 单段文本里**最后一个**协作模式块是否声明「计划模式」：从该块标签 [`COLLABORATION_MODE_TAG`]
/// 之后取**第一个非空行**，只有它是 Markdown 标题（`# …`）且含 `plan mode`（codex 0.154：
/// `# Plan Mode (Conversational)`）或 `collaboration mode: plan`（旧文案）才算计划模式；
/// `None` 表示这段文本里根本没有模式块。
///
/// **只认标题行**：默认模式文案的标题是 `# Collaboration Mode: Default`，正文第一句却写着
/// 「… for other modes (e.g. Plan mode) are no longer active.」——按整段（甚至按正文行）子串
/// 匹配都会把默认模式误判成计划模式，那样默认模式的正常编码回合会被注入「请给出计划」。
/// 块不闭合（文本被截断）时同样按标题行判定。
fn last_mode_block_declares_plan(text: &str) -> Option<bool> {
    let lower = text.to_lowercase();
    let mut rest = lower.as_str();
    let mut verdict = None;
    while let Some(index) = rest.find(COLLABORATION_MODE_TAG) {
        rest = &rest[index + COLLABORATION_MODE_TAG.len()..];
        // 标题行 = 标签之后的第一个非空行；codex 实测标签后紧跟 "# Plan Mode …"
        let plan = rest
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .is_some_and(|heading| {
                heading.starts_with('#')
                    && (heading.contains(PLAN_MODE_HEADING_MARKER)
                        || heading.contains(PLAN_MODE_HEADING_MARKER_LEGACY))
            });
        verdict = Some(plan);
    }
    verdict
}

/// 请求里可能携带协作模式块的文本，按 codex 的组装顺序：`instructions` → `input`
/// （`input` 为字符串简写时先取它，为数组时逐条目取文本）。
fn request_mode_texts(req: &Value) -> Vec<String> {
    let mut texts: Vec<String> = Vec::new();
    if let Some(instructions) = req.get("instructions").and_then(Value::as_str) {
        texts.push(instructions.to_string());
    }
    if let Some(Value::String(text)) = req.get("input") {
        texts.push(text.clone());
    }
    if let Some(items) = req.get("input").and_then(Value::as_array) {
        for item in items {
            let text = item_content_text(item);
            if !text.is_empty() {
                texts.push(text);
            }
        }
    }
    texts
}

/// 请求是否处于计划模式——**关键词兜底判据**，只在协议登记表查不到该线程时使用：
/// 取请求里**最后一个**协作模式块的标题行（大小写不敏感）。codex 会把历次模式块都留在历史里
/// （切回默认模式后旧的 `# Plan Mode …` 块仍在），按「任一命中即计划模式」必然把默认模式误判
/// 成计划模式，所以这里以最后一块为准。
fn request_is_plan_mode(req: &Value) -> bool {
    let mut verdict = None;
    for text in request_mode_texts(req) {
        if let Some(block) = last_mode_block_declares_plan(&text) {
            verdict = Some(block);
        }
    }
    verdict.unwrap_or(false)
}

/// 入站请求对应的 codex 线程 id：取请求头 `session-id`（codex 用它上报线程 id），
/// 归一化见 [`normalize_client_session`]（与 [`SessionMap`] 用同一口径，登记表按原始线程 id 存）。
/// 缺失、纯空白或非 ASCII 时返回 None（调用方退回关键词兜底判据）。
fn request_header_thread_id(headers: &HeaderMap) -> Option<String> {
    normalize_client_session(headers.get("session-id")?.to_str().ok()?)
}

/// 本次入站请求的协作模式与来源：先按请求头里的线程 id 查协议登记表（权威，命中即不做任何
/// 关键词扫描），查不到才退回关键词判据（只看**最后一个**协作模式块）。
/// 返回 `(是否计划模式, mode_src)`，`mode_src` 进 `zen_proxy.request` 日志便于排查。
fn resolve_nudge_mode(
    state: &ProxyState,
    headers: &HeaderMap,
    req: &Value,
) -> (bool, &'static str) {
    if let Some(mode) = request_header_thread_id(headers).and_then(|id| state.modes.mode(&id)) {
        return (mode == "plan", MODE_SRC_REGISTRY);
    }
    (request_is_plan_mode(req), MODE_SRC_HEURISTIC)
}

/// 终局文本是否为计划产物（`<proposed_plan>` 包裹）：**计划模式**下命中即视为
/// 「计划已交付」并直接收尾（代码判据，不调用 AI 判定）；默认模式不使用本判据。
fn is_plan_deliverable(text: &str) -> bool {
    text.to_lowercase().contains(PLAN_OUTPUT_MARKER)
}

/// 终局文本是否带「用户已取消这个计划」标记（见 [`PLAN_CANCEL_MARKER`]）：**计划模式**下
/// 命中即视为计划话题已终结并直接收尾——不再注入续跑提醒，也**不生成计划条目**
/// （用户放弃后看到的就是模型那句普通结论，标签本身被 [`TagStripper`] 剥离）；
/// 默认模式不使用本判据。
fn is_plan_cancelled(text: &str) -> bool {
    text.to_lowercase().contains(PLAN_CANCEL_MARKER)
}

/// 终局文本是否带「问题无法或无需产出实现计划」标记（见 [`PLAN_UNACHIEVABLE_MARKER`]）：
/// **计划模式**下命中即视为计划话题已终结并直接收尾——不再注入续跑提醒，也**不生成计划条目**
/// （例如「1+1=？」这类事实问题：正文已在上一轮给出，催办轮按约定只回标签，标签由
/// [`TagStripper`] 剥离）；默认模式不使用本判据。
fn is_plan_unachievable(text: &str) -> bool {
    text.to_lowercase().contains(PLAN_UNACHIEVABLE_MARKER)
}

/// 终局文本是否带「任务已经结束」标记（见 [`TASK_COMPLETED_MARKER`]）：**默认模式**下命中即视为
/// 模型自己宣告任务终结、直接收尾——不再注入续跑提醒、不触发后续 pass。判据与别的标签同口径
/// （大小写不敏感 + 前缀匹配，缺闭合标签、空标签同样命中），且**不解析载荷**；计划模式不使用
/// 本判据。
fn is_task_completed(text: &str) -> bool {
    text.to_lowercase().contains(TASK_COMPLETED_MARKER)
}

/// 请求是否声明了可调用的工具（`tools` 数组非空）：催办与首轮教学共用同一判据，
/// 避免两处口径漂移（没有工具可调时既不催办、也不注入契约）。
fn request_has_tools(req: &Value) -> bool {
    req.get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty())
}

/// 首轮教学是否该注入：只给「**流式** + **声明了工具** + **非会话标题线程**」的请求注入。
/// 纯函数，便于单测真值表。三条理由：非流式没有催办路径可消费这份契约；没有工具声明的请求
/// 在催办判据里也被放过（压缩/摘要类后台请求通常不带工具）；标题线程只产出标题。
fn contract_injection_eligible(want_stream: bool, has_tools: bool, title_task: bool) -> bool {
    want_stream && has_tools && !title_task
}

/// 本次该注入哪段契约（计划模式用 [`PLAN_MODE_CONTRACT_TEXT`]、默认模式用
/// [`DEFAULT_MODE_CONTRACT_TEXT`]）。
fn contract_text(plan_mode: bool) -> &'static str {
    if plan_mode {
        PLAN_MODE_CONTRACT_TEXT
    } else {
        DEFAULT_MODE_CONTRACT_TEXT
    }
}

/// 把契约追加到请求的 `instructions` 尾部（原内容逐字保留，只补一个空行分隔）；`instructions`
/// 缺失或为空时以契约本身作为它。**只改 `instructions`、不动 `input`**——协作模式块的关键词
/// 扫描与 `resolve_nudge_mode` 的结论都只看 `instructions` 里的模式块标签，所以调用方必须先
/// 解析模式、再注入；契约文本里也不含 `<collaboration_mode>` 块。幂等：`instructions` 已经以
/// 该契约结尾时直接返回（同一请求重复调用不会叠加两遍）。
fn inject_contract(req: &mut Value, text: &str) {
    let merged = match req.get("instructions").and_then(Value::as_str) {
        None | Some("") => text.to_string(),
        Some(existing) if existing.ends_with(text) => return,
        Some(existing) => format!("{existing}\n\n{text}"),
    };
    req["instructions"] = Value::String(merged);
}

/// 请求是否为应用发起的「会话标题生成」任务（后台临时线程）：末条 user 文本以提示词
/// 固定前缀开头即命中。标题线程只把首条消息压成短标题、必然以纯文本结束，让口嗨检测
/// 参与只会白花一轮上游（注入续跑提醒后还要再等一次生成），因此整轮放行。
fn request_is_title_task(req: &Value) -> bool {
    last_user_text(req).trim().starts_with(TITLE_TASK_PREFIX)
}

/// 取入站 Responses 请求里最后一条 user 消息的纯文本（会话标题线程的识别用）。
fn last_user_text(req: &Value) -> String {
    match req.get("input") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .rev()
            .find(|item| item.get("role").and_then(Value::as_str) == Some("user"))
            .map(item_content_text)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// 构造续跑请求体：克隆原请求，把本轮助手文本与注入提醒（默认模式用 [`NUDGE_TEXT`]、
/// 计划模式用 [`PLAN_NUDGE_TEXT`]）按序追加到 `input` 尾部。原请求不被修改；
/// `input` 为字符串（简写形态）时先转成数组再追加。
fn nudge_continuation_body(original: &Value, assistant_text: &str, nudge_text: &str) -> Value {
    let mut body = original.clone();
    let mut input = match body.get("input") {
        Some(Value::String(text)) => vec![json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": text }]
        })],
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    input.push(json!({
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": assistant_text }]
    }));
    input.push(json!({
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": nudge_text }]
    }));
    body["input"] = Value::Array(input);
    body
}

/// 运行中的代理句柄；`stop()` 终止监听任务。
pub struct ZenProxyHandle {
    pub port: u16,
    /// 生效中的启动配置（已归一化，用于判断是否需要重启）。
    config: ZenProxyConfig,
    abort: AbortHandle,
}

/// 代理启动配置：端口、上游地址与两个行为开关。任一项变化都要重启代理，
/// 因此整体作为 [`ZenProxyHandle`] 的比对基准。
#[derive(Debug, Clone)]
pub(crate) struct ZenProxyConfig {
    pub port: u16,
    /// 上游 base_url（已归一化：去空白与末尾 `/`）。
    pub base_url: String,
    /// 「OpenCode 客户端身份」开关：是否按 opencode 客户端形状发送识别头/UA，
    /// 并对 host 含 `opencode` 的上游补齐免费层请求体门禁字段。
    pub opencode_identity: bool,
    /// 「回合收尾强制约束」开关：是否启用口嗨检测 + 自动续跑（含首轮教学与标签剥离）。
    pub nudge_enabled: bool,
}

impl ZenProxyConfig {
    /// 归一化构造：base_url 去空白与末尾 `/`，空值回退默认上游。
    pub(crate) fn new(
        port: u16,
        base_url: &str,
        opencode_identity: bool,
        nudge_enabled: bool,
    ) -> Self {
        Self {
            port,
            base_url: normalize_base_url(base_url),
            opencode_identity,
            nudge_enabled,
        }
    }

    /// 是否按 Zen 免费层门禁补请求体形状：只有「身份开启」且上游 host 含 `opencode` 时才补。
    fn zen_body_patch(&self) -> bool {
        self.opencode_identity && is_zen_upstream(&self.base_url)
    }

    /// 是否与运行中的实例等价（用于「要不要重启代理」的判定）：端口、归一化后的上游地址
    /// 与两个行为开关全部相同才算未变。
    fn matches(&self, other: &Self) -> bool {
        self.port == other.port
            && self.base_url == other.base_url
            && self.opencode_identity == other.opencode_identity
            && self.nudge_enabled == other.nudge_enabled
    }
}

/// Zen 本地代理的诊断日志句柄（复用应用会话日志；None 时不落盘）。
pub(crate) type ZenLog = Option<Arc<SessionLog>>;

impl ZenProxyHandle {
    pub fn stop(&self) {
        self.abort.abort();
    }
}

/// 代理状态（供设置页与 `zen_proxy_status` 命令展示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ZenProxyStatus {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
}

impl ZenProxyStatus {
    pub fn stopped() -> Self {
        Self {
            running: false,
            port: DEFAULT_ZEN_PROXY_PORT,
            error: None,
        }
    }
}

/// 归一化上游 base_url：去首尾空白、空值回退默认地址、去掉末尾所有 `/`。
/// `https://a/` 与 `https://a`、`https://a/zen/v2/` 与 `https://a/zen/v2` 因此等价，
/// 既影响翻译路由的派生，也影响「上游地址是否变化」的重启判定。
fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_ZEN_BASE_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

/// 翻译入口路径 = 上游 base_url 的路径 + `/responses`（base_url 无路径时为 `/responses`）。
/// codex 对 provider 的 base_url 追加 `/responses`，因此本地 provider 的路径需与提供方一致。
fn responses_path(base_url: &str) -> String {
    let base = normalize_base_url(base_url);
    let path = reqwest::Url::parse(&base)
        .map(|url| url.path().trim_end_matches('/').to_string())
        .unwrap_or_default();
    format!("{path}/responses")
}

/// 上游是否为 OpenCode Zen（host 小写化后含有 `opencode`，大小写不敏感）。
/// 只有 Zen 免费层才有 [`ZEN_REQUIRED_TOOL_NAMES`] / [`ZEN_MAX_TOKENS`] 这套请求体门禁，
/// 因此补形状只对 Zen 生效；指向自建或第三方兼容端点（DeepSeek 等）时请求体保持原样。
fn is_zen_upstream(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    host.contains("opencode")
}

/// 在当前 tokio runtime 上启动本地代理；端口被占用时返回 Err。
pub(crate) async fn start(
    config: ZenProxyConfig,
    log: ZenLog,
    trace: TraceSink,
    modes: Arc<ThreadModeRegistry>,
) -> Result<ZenProxyHandle, String> {
    let port = config.port;
    let base_url = config.base_url.clone();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut attempt = 0;
    let listener = loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => break listener,
            // 同端口换上游会先停旧实例再立即绑定，旧监听可能尚未释放，短暂重试。
            Err(e) => {
                if attempt >= BIND_RETRY {
                    return Err(format!("代理端口 {port} 启动失败（可能被占用）：{e}"));
                }
                attempt += 1;
                tokio::time::sleep(BIND_RETRY_INTERVAL).await;
            }
        }
    };
    let app = Router::new().fallback(handle_any).with_state(ProxyState {
        session: opencode_id("ses", true),
        session_map: Arc::new(SessionMap::default()),
        base_url: base_url.clone(),
        log,
        trace,
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        opencode_identity: config.opencode_identity,
        zen_body_patch: config.zen_body_patch(),
        nudge_enabled: config.nudge_enabled,
        modes,
    });
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(ZenProxyHandle {
        port,
        config,
        abort: task.abort_handle(),
    })
}

/// 由 `CodexServer` 调用：按设置启停代理并返回当前状态。
pub(crate) async fn apply(
    handle: &mut Option<ZenProxyHandle>,
    enabled: bool,
    config: ZenProxyConfig,
    log: ZenLog,
    trace: TraceSink,
    modes: Arc<ThreadModeRegistry>,
) -> ZenProxyStatus {
    let port = config.port;
    // 端口、上游地址与两个行为开关都没变（含 `https://a/` 与 `https://a` 这类等价写法）
    // 才复用现有实例。
    let unchanged = handle.as_ref().is_some_and(|h| h.config.matches(&config));
    let need_start = enabled && !unchanged;
    if !enabled || !unchanged {
        if let Some(h) = handle.take() {
            h.stop();
        }
    }
    if need_start {
        match start(config, log, trace, modes).await {
            Ok(h) => {
                *handle = Some(h);
                ZenProxyStatus {
                    running: true,
                    port,
                    error: None,
                }
            }
            Err(e) => ZenProxyStatus {
                running: false,
                port,
                error: Some(e),
            },
        }
    } else if enabled {
        ZenProxyStatus {
            running: true,
            port,
            error: None,
        }
    } else {
        ZenProxyStatus::stopped()
    }
}

// ---------------------------------------------------------------------------
// HTTP 入口
// ---------------------------------------------------------------------------

/// 代理运行期共享状态；`session` 与真实 opencode 客户端一致，在代理生命周期内保持稳定，
/// 作为客户端未提供 `session-id` 请求头时的 `x-opencode-session` 回落值。
#[derive(Clone)]
struct ProxyState {
    /// 代理级稳定会话：客户端没给 `session-id` 时用它作 `x-opencode-session`（同样是
    /// opencode 形状，代理启动时生成一次）。
    session: String,
    /// 「codex 会话 id ↔ 上游 `ses_*`」映射表，见 [`SessionMap`]。
    session_map: Arc<SessionMap>,
    base_url: String,
    log: ZenLog,
    /// 内容诊断日志（常开）：每次上游尝试的请求体/响应原文/收尾事件。
    trace: TraceSink,
    /// 上游是否要求历史里带 `tool_calls` 的 assistant 消息回传 `reasoning_content`
    /// （DeepSeek 思考模式）。默认关闭，收到明确报错后学习并粘滞到本代理实例结束。
    requires_reasoning_rc: Arc<AtomicBool>,
    /// 「OpenCode 客户端身份」开关：是否发 opencode 形状的识别头与 UA，并对 host 含
    /// `opencode` 的上游补请求体门禁字段（见 [`patch_zen_request_body`]）。
    opencode_identity: bool,
    /// 是否按 Zen 免费层的请求体门禁补形状（`max_tokens` + 内置工具名，见
    /// [`patch_zen_request_body`]）：由启动配置派生 = 「OpenCode 客户端身份」开启
    /// **且** 上游 host 含 `opencode`（见 [`ZenProxyConfig::zen_body_patch`]）；
    /// 其它上游、以及身份开关关闭时一律保持请求体原样。
    zen_body_patch: bool,
    /// 「回合收尾强制约束」开关：是否启用口嗨检测 + 自动续跑（含首轮教学与标签剥离）。
    nudge_enabled: bool,
    /// 协作模式登记表（key = codex 线程 id）：由 app-server 侧登记，代理解析模式时优先查它。
    modes: Arc<ThreadModeRegistry>,
}

/// 写一条 zen_proxy 诊断日志；句柄为 None 或写盘失败时静默忽略。
fn log_at(log: &Option<Arc<SessionLog>>, level: &str, event: &str, kv: &[(&str, String)]) {
    let Some(log) = log else { return };
    let kv: Vec<(String, String)> = kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    log.write(level, None, event, &kv);
}

/// opencode 客户端标识的字符集（与 `sst/opencode` 的 `Identifier.create` 一致）。
const OPENCODE_ID_CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// opencode 客户端识别头：由「OpenCode 客户端身份」开关统一控制（关闭时一个都不发，
/// 入站请求带来的同名声也要剔除，避免误当成 opencode 客户端）。
const OPENCODE_IDENTITY_HEADERS: [&str; 4] = [
    "x-opencode-client",
    "x-opencode-project",
    "x-opencode-request",
    "x-opencode-session",
];
/// opencode 客户端标识的后段长度：`ses_` / `msg_` 之后固定 26 位。
const OPENCODE_ID_LEN: usize = 26;
/// [`opencode_id`] 的进程内计数状态：`(毫秒时间戳, 同毫秒内已用序号)`。
static OPENCODE_ID_STATE: Mutex<(u64, u64)> = Mutex::new((0, 0));
/// 会话映射表上限：超出后整体清空再插入（与 [`ThreadModeRegistry`] 同一取舍）。
const SESSION_MAP_LIMIT: usize = 1024;
/// 铸号撞上已有会话时的最大重铸次数。
const SESSION_MAP_MINT_RETRY: usize = 7;

/// 生成 opencode 客户端标识：`prefix_` + 26 位（前 12 位时间十六进制 + 后 14 位随机字母数字）。
///
/// 复刻 `sst/opencode` 的 `Identifier.create`：`current = Date.now() * 0x1000 + 计数器`
/// （计数器同毫秒内自增、跨毫秒清零），`descending` 取 `!current`（会话 id 用降序、
/// 消息 id 用升序）；前 12 位是 `current` 低 6 字节的小写十六进制，后 14 位是随机字节按
/// `% 62` 映射到 [`OPENCODE_ID_CHARS`]。Zen 服务端要求 `ses_` / `msg_` 之后恰为 26 位
/// `[0-9A-Za-z]`（真实 opencode 客户端就是这种形状），格式不符会被判成「非 opencode 客户端」。
fn opencode_id(prefix: &str, descending: bool) -> String {
    let (timestamp, counter) = {
        let mut state = OPENCODE_ID_STATE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = unix_now_ms();
        if now != state.0 {
            state.0 = now;
            state.1 = 0;
        }
        state.1 += 1;
        (state.0, state.1)
    };
    let value = {
        let current = timestamp.wrapping_mul(0x1000).wrapping_add(counter);
        if descending {
            !current
        } else {
            current
        }
    };
    let mut out = String::with_capacity(prefix.len() + 1 + OPENCODE_ID_LEN);
    out.push_str(prefix);
    out.push('_');
    for index in 0..6 {
        let byte = ((value >> (40 - 8 * index)) & 0xff) as u8;
        out.push_str(&format!("{byte:02x}"));
    }
    for byte in random_bytes::<14>() {
        out.push(OPENCODE_ID_CHARS[(byte % 62) as usize] as char);
    }
    out
}

/// 取 `N` 字节随机数：优先 Windows CNG（`BCryptGenRandom` + 系统首选随机源），
/// 失败时回落到「时间纳秒 + 进程内计数」播种的 SplitMix64（保证不 panic、仍有基本离散度）。
fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    if !fill_random_from_cng(&mut buf) {
        fill_random_fallback(&mut buf);
    }
    buf
}

/// 用 Windows CNG 填充随机字节；返回是否成功。
#[cfg(windows)]
fn fill_random_from_cng(buf: &mut [u8]) -> bool {
    use windows::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    // SAFETY: 传给 CNG 的是可写切片（长度由 windows crate 换算），`None` 表示使用系统首选
    // 随机源——不涉及句柄，也就不需要释放。返回 NTSTATUS，0 即 STATUS_SUCCESS。
    unsafe { BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG).0 == 0 }
}

#[cfg(not(windows))]
fn fill_random_from_cng(_buf: &mut [u8]) -> bool {
    false
}

/// 随机源不可用时的回落：SplitMix64 由时间纳秒与进程内计数混合播种。
fn fill_random_fallback(buf: &mut [u8]) {
    /// SplitMix64 的常量增量。
    const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let mut x = unix_now_nanos() ^ SEQ.fetch_add(1, Ordering::Relaxed).wrapping_mul(GAMMA);
    for slot in buf.iter_mut() {
        x = x.wrapping_add(GAMMA);
        *slot = (mix64(x) >> 32) as u8;
    }
}

/// SplitMix64 的最终混合函数。
fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// 「codex 会话 id ↔ 上游 `x-opencode-session`」双向映射表（内存态，进程内共享）。
///
/// 存在的理由：Zen 服务端要求会话标识是 opencode 形状（`ses_` + 26 位 `[0-9A-Za-z]`），
/// 而 codex 上报的线程 id 是 UUID（带连字符，形状不符）。这里按真实 opencode 的生成规则
/// （[`opencode_id`] + 降序）为每个 codex 线程铸一个会话 id 并缓存：同一线程的所有请求发
/// 同一个值（上游按会话分组/限流的语义不变），不同线程互不相同；反向索引让「看到 `ses_*`
/// 想找回 codex 线程 id」变成一次查表（日志与排查用）。表只存内存：代理重启后同一线程会
/// 拿到新的 `ses_*`，与 [`ThreadModeRegistry`] 的取舍一致。
#[derive(Default)]
struct SessionMap {
    inner: Mutex<SessionMapInner>,
}

#[derive(Default)]
struct SessionMapInner {
    /// codex 会话 id（归一化后）→ 上游 `ses_*`。
    forward: HashMap<String, String>,
    /// 上游 `ses_*` → codex 会话 id（归一化后）。
    reverse: HashMap<String, String>,
}

impl SessionMap {
    /// 取入站会话 id 对应的上游 `ses_*`：首次见到时铸一个并双向登记，之后恒定返回同一个。
    /// 入站值为空白（或客户端没给）时返回 None，调用方回落到代理级稳定会话。
    fn resolve(&self, client_session_id: &str) -> Option<String> {
        self.resolve_with(client_session_id, || opencode_id("ses", true))
    }

    /// [`SessionMap::resolve`] 的可注入版本：`mint` 负责铸号，便于单测构造撞号场景。
    fn resolve_with(
        &self,
        client_session_id: &str,
        mut mint: impl FnMut() -> String,
    ) -> Option<String> {
        let key = normalize_client_session(client_session_id)?;
        let Ok(mut inner) = self.inner.lock() else {
            // 表不可用（锁中毒）时仍要给出合法形状的值：现铸一个，只是失去会话稳定性。
            return Some(mint());
        };
        if let Some(existing) = inner.forward.get(&key) {
            return Some(existing.clone());
        }
        if inner.forward.len() >= SESSION_MAP_LIMIT {
            inner.forward.clear();
            inner.reverse.clear();
        }
        let mut session = mint();
        // 撞号（该值已属于另一个 codex 会话）就重铸；26 位标识撞号概率可忽略，
        // 重试用尽后沿用最后一次取值（反向索引只保留最新一条）。
        for _ in 0..SESSION_MAP_MINT_RETRY {
            if !inner.reverse.contains_key(&session) {
                break;
            }
            session = mint();
        }
        inner.forward.insert(key.clone(), session.clone());
        inner.reverse.insert(session.clone(), key);
        Some(session)
    }

    /// 反查：由上游 `ses_*` 找回 codex 会话 id；未登记（如代理级回落会话）返回 None。
    fn codex_of(&self, session: &str) -> Option<String> {
        let inner = self.inner.lock().ok()?;
        inner.reverse.get(session).cloned()
    }
}

/// 归一化入站会话 id：trim 后剥掉可能存在的 `ses_` 前缀（codex 上报的是裸线程 id），
/// 结果为空时返回 None（调用方视为「客户端没给会话 id」）。
fn normalize_client_session(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let bare = trimmed.strip_prefix("ses_").unwrap_or(trimmed).trim();
    (!bare.is_empty()).then(|| bare.to_string())
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    // 不在客户端上固定 User-Agent：识别头与 UA 都由每个请求按「OpenCode 客户端身份」
    // 开关决定（见 [`apply_opencode_identity`]），关闭时才能透传入站 UA。
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// `zen_proxy.request` 的诊断列（纯函数，便于单测）：体积（提示词 / 历史字符数）
/// 用于判断是否逼近上游窗口；`reasoning_effort` 记录 codex 本次实际请求的推理强度
/// （`-` 表示没请求推理），排查「思考过程为空」时先看这一格；
/// `mode` 是口嗨检测识别出的协作模式（`plan`/`default`）——排查「默认模式被注入计划提醒」
/// 这类问题时先看这一格；`mode_src` 说明它的来源（`registry` = 协议登记表，
/// `heuristic` = 关键词兜底判据，见 [`resolve_nudge_mode`]）；
/// `patch_failures` 是本次历史里已失败的补丁调用条数（>0 说明模型正在补丁上打转，
/// 代理已对这些工具结果追加格式纠错提示）；`call_id` 与内容日志
/// （`logs/zen/<时间>-<call_id>-a<n>.*`）一一对应，便于两者对照。
fn request_log_fields(
    req: &Value,
    want_stream: bool,
    call_id: &str,
    mode: &str,
    mode_src: &str,
    contract: &str,
) -> Vec<(&'static str, String)> {
    let model = req
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let input_msg_count = req
        .get("input")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    let tool_count = req
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| tools.len())
        .unwrap_or(0);
    let instructions_chars = req
        .get("instructions")
        .and_then(Value::as_str)
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let input_chars = req
        .get("input")
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let reasoning_effort = req
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_string();
    let patch_failures = patch_failure_count(req);
    vec![
        ("call_id", call_id.to_string()),
        ("model", model),
        ("stream", want_stream.to_string()),
        ("模式", mode.to_string()),
        ("模式来源", mode_src.to_string()),
        ("首轮教学", contract.to_string()),
        ("input_msg_count", input_msg_count.to_string()),
        ("input_chars", input_chars.to_string()),
        ("instructions_chars", instructions_chars.to_string()),
        ("reasoning_effort", reasoning_effort),
        ("tool_count", tool_count.to_string()),
        ("patch_failures", patch_failures.to_string()),
    ]
}

/// 统一入口：`POST {上游 base_url 路径}/responses` 走 Responses→Chat 翻译，其余路径/方法透传。
async fn handle_any(
    State(state): State<ProxyState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let expected = responses_path(&state.base_url);
    if method == Method::POST && uri.path() == expected {
        return handle_responses(&state, &headers, body).await;
    }
    // 客户端把 `/responses` 打到了别的路径：本地 provider 的 base_url 路径与上游不一致。
    if uri.path().ends_with("/responses") {
        log_at(
            &state.log,
            "warn",
            "zen_proxy.path_unmatched",
            &[
                ("method", method.to_string()),
                ("path", uri.path().to_string()),
                ("expected", expected),
            ],
        );
    }
    forward_passthrough(&state, method, &uri, &headers, body).await
}

/// 把请求体读成 JSON 后交给翻译路径；读取或解析失败时返回明确错误。
async fn handle_responses(state: &ProxyState, headers: &HeaderMap, body: Body) -> Response {
    let bytes = match axum::body::to_bytes(body, RESPONSES_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(e) => {
            return error_json(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("请求体过大或读取失败：{e}"),
            );
        }
    };
    let mut req: Value = match serde_json::from_slice(&bytes) {
        Ok(req) => req,
        Err(e) => {
            return error_json(StatusCode::BAD_REQUEST, format!("请求体不是合法 JSON：{e}"));
        }
    };
    let want_stream = req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    // 本次入站请求的稳定标识：贯穿内容日志文件名与 session 日志行。
    let call_id = opencode_id("req", false);
    // 协作模式先按协议登记表解析（查不到才退回关键词），日志里连同来源一起记下
    let (plan_mode, mode_src) = resolve_nudge_mode(state, headers, &req);
    let mode = if plan_mode { "plan" } else { "default" };
    // 首轮教学：把收尾契约追加到 instructions 尾部（必须在模式解析之后——契约文本里不含
    // `<collaboration_mode>` 块，模式判据与 mode_src 因此完全不受影响）
    // 「回合收尾强制约束」关闭时整条链路（首轮教学、终局判定、续跑、标签剥离）都不参与
    let nudge_enabled = state.nudge_enabled;
    let contract = if nudge_enabled
        && contract_injection_eligible(
            want_stream,
            request_has_tools(&req),
            request_is_title_task(&req),
        ) {
        inject_contract(&mut req, contract_text(plan_mode));
        mode
    } else {
        "off"
    };
    // 日志按**注入后**的实际请求体记（`instructions_chars` 含契约长度，便于对照体积变化）
    let fields = request_log_fields(&req, want_stream, &call_id, mode, mode_src, contract);
    let kv: Vec<(&str, String)> = fields.iter().map(|(k, v)| (*k, v.clone())).collect();
    log_at(&state.log, "info", "zen_proxy.request", &kv);

    let started = Instant::now();
    let base_url = state.base_url.clone();
    match forward(&req, headers, want_stream, state, &call_id).await {
        Ok(forwarded) => {
            let status = forwarded.status;
            log_at(
                &state.log,
                "info",
                "zen_proxy.forward",
                &[
                    ("url", base_url.clone()),
                    ("status", status.as_u16().to_string()),
                    ("elapsed_ms", started.elapsed().as_millis().to_string()),
                    (
                        "dropped_fields",
                        if forwarded.dropped_fields.is_empty() {
                            "-".to_string()
                        } else {
                            forwarded.dropped_fields.join(",")
                        },
                    ),
                    (
                        "reasoning_rc",
                        if forwarded.reasoning_rc { "on" } else { "off" }.to_string(),
                    ),
                    (
                        "zen_body",
                        if state.zen_body_patch && state.opencode_identity {
                            "on"
                        } else {
                            "off"
                        }
                        .to_string(),
                    ),
                    (
                        "身份伪装",
                        if state.opencode_identity { "on" } else { "off" }.to_string(),
                    ),
                ],
            );
            if let Some(change) = forwarded.reasoning_rc_change.as_ref() {
                log_at(
                    &state.log,
                    "warn",
                    if change.enabled {
                        "zen_proxy.reasoning_content_enabled"
                    } else {
                        "zen_proxy.reasoning_content_disabled"
                    },
                    &[(
                        "detail",
                        if change.enabled {
                            format!(
                                "上游要求历史回传 reasoning_content，已改为回传：{}",
                                change.detail
                            )
                        } else {
                            format!(
                                "上游不接受 reasoning_content，已停止回传：{}",
                                change.detail
                            )
                        },
                    )],
                );
            }
            if forwarded.repairs.invalid_arguments > 0 || forwarded.repairs.missing_tool_outputs > 0
            {
                log_at(
                    &state.log,
                    "warn",
                    "zen_proxy.history_repaired",
                    &[
                        (
                            "invalid_arguments",
                            forwarded.repairs.invalid_arguments.to_string(),
                        ),
                        (
                            "missing_tool_outputs",
                            forwarded.repairs.missing_tool_outputs.to_string(),
                        ),
                    ],
                );
            }
            if !forwarded.dropped_fields.is_empty() {
                log_at(
                    &state.log,
                    "warn",
                    "zen_proxy.optional_fields_dropped",
                    &[
                        ("fields", forwarded.dropped_fields.join(",")),
                        (
                            "detail",
                            "上游指名拒绝这些可选字段，已摘除后重试一次".to_string(),
                        ),
                    ],
                );
            }
            match forwarded.payload {
                ForwardPayload::Text(text) => {
                    proxy_error_text(status, &text, &state.log, forwarded.trace)
                }
                ForwardPayload::Live(resp) => {
                    if !status.is_success() {
                        return proxy_error_response(status, resp, &state.log, forwarded.trace)
                            .await;
                    }
                    if want_stream {
                        proxy_stream_response(
                            state.clone(),
                            headers.clone(),
                            call_id.clone(),
                            req,
                            resp,
                            forwarded.trace,
                        )
                        .await
                    } else {
                        proxy_json_response(
                            req,
                            resp,
                            &state.log,
                            state.nudge_enabled,
                            forwarded.trace,
                        )
                        .await
                    }
                }
            }
        }
        Err((status, msg)) => {
            log_at(
                &state.log,
                "warn",
                "zen_proxy.forward_error",
                &[("url", base_url), ("error", msg.clone())],
            );
            error_json(status, msg)
        }
    }
}

/// 透传核心（便于单测）：构造上游 URL、转发请求并流式回传响应。
/// 只替换 host，路径与 query 原样发出——本地 provider 的 base_url 路径需与提供方一致。
async fn forward_passthrough(
    state: &ProxyState,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let url = match passthrough_url(&state.base_url, uri) {
        Ok(url) => url,
        Err(e) => {
            log_passthrough(state, &method, uri.path(), "error");
            return error_json(StatusCode::BAD_GATEWAY, e);
        }
    };
    let mut rq = http_client()
        .request(to_reqwest_method(&method), url)
        .headers(forwarded_request_headers(headers, state));
    if method_has_body(&method) {
        rq = rq.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    match rq.send().await {
        Ok(resp) => {
            let status = resp.status();
            log_passthrough(state, &method, uri.path(), &status.as_u16().to_string());
            passthrough_response(resp).await
        }
        Err(e) => {
            log_passthrough(state, &method, uri.path(), "error");
            error_json(StatusCode::BAD_GATEWAY, format!("上游请求失败：{e}"))
        }
    }
}

/// 透传日志：2xx 记 info、其余记 warn，配合 `zen_proxy.path_unmatched` 判断请求走了哪条分支。
fn log_passthrough(state: &ProxyState, method: &Method, path: &str, status: &str) {
    let level = if status.starts_with('2') {
        "info"
    } else {
        "warn"
    };
    log_at(
        &state.log,
        level,
        "zen_proxy.passthrough",
        &[
            ("method", method.to_string()),
            ("path", path.to_string()),
            ("status", status.to_string()),
        ],
    );
}

/// 透传目标 URL：scheme/host/port 取自 base_url，入站 path 与 query 原样发出（零路径转换）。
fn passthrough_url(base_url: &str, uri: &Uri) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(&normalize_base_url(base_url))
        .map_err(|e| format!("Zen 代理转发地址无效: {e}"))?;
    url.set_path(uri.path());
    url.set_query(uri.query());
    url.set_fragment(None);
    Ok(url)
}

/// 上游 `x-opencode-session` 取值：按入站请求头 `session-id`（OpenAI 协议会话标识，codex
/// 每次请求都会带上）查 [`SessionMap`]，首次见到就按 opencode 规则铸一个 `ses_*` 并登记
/// （同一 codex 线程恒定、不同线程不同）；请求头缺失、空白或非可见 ASCII 时回落到代理
/// 生命周期内稳定的 `state.session`（同样是合法形状）。
fn opencode_session(state: &ProxyState, headers: &HeaderMap) -> String {
    headers
        .get("session-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| state.session_map.resolve(value))
        .unwrap_or_else(|| state.session.clone())
}

/// 按「OpenCode 客户端身份」开关落到请求头上的全部效果（翻译路径与透传路径共用）：
/// - 开启：附加固定识别头（`x-opencode-client` / `-project` / `-request` / `-session`）与
///   opencode 客户端 UA，入站 UA 一律丢弃（与既有行为一致）；
/// - 关闭：不附加任何识别头，并把入站自带的同名头剔除（避免仍然“自称 opencode”），
///   UA 改为透传入站值；入站没有 UA 时不带 UA（客户端不再设默认 UA）。
///
/// `session` 与 `request_id` 由调用方提供，保证内容日志里记的值与实发头逐字一致；
/// 关闭时两者不参与发送。
fn apply_opencode_identity(
    out: &mut HeaderMap,
    headers: &HeaderMap,
    state: &ProxyState,
    session: &str,
    request_id: &str,
) {
    for name in OPENCODE_IDENTITY_HEADERS {
        out.remove(name);
    }
    out.remove(header::USER_AGENT);
    if !state.opencode_identity {
        if let Some(ua) = headers.get(header::USER_AGENT) {
            out.insert(header::USER_AGENT, ua.clone());
        }
        return;
    }
    out.insert(
        "x-opencode-client",
        HeaderValue::from_static(OPENCODE_CLIENT),
    );
    out.insert(
        "x-opencode-project",
        HeaderValue::from_static(OPENCODE_PROJECT),
    );
    if let Ok(value) = HeaderValue::from_str(request_id) {
        out.insert("x-opencode-request", value);
    }
    if let Ok(value) = HeaderValue::from_str(session) {
        out.insert("x-opencode-session", value);
    }
    if let Ok(value) = HeaderValue::from_str(ZEN_USER_AGENT) {
        out.insert(header::USER_AGENT, value);
    }
}

/// 构造转发给上游的请求头：保留入站头，剔除逐跳头，再按身份开关补识别头/UA。
fn forwarded_request_headers(headers: &HeaderMap, state: &ProxyState) -> HeaderMap {
    let mut out = headers.clone();
    for name in HOP_BY_HOP_HEADERS {
        out.remove(name);
    }
    out.remove(header::HOST);
    out.remove(header::CONTENT_LENGTH);
    apply_opencode_identity(
        &mut out,
        headers,
        state,
        &opencode_session(state, headers),
        &opencode_id("msg", false),
    );
    out
}

/// 构造回传给客户端的响应头：剔除逐跳头与长度头，保留内容类型/缓存等。
fn forwarded_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut out = headers.clone();
    for name in HOP_BY_HOP_HEADERS {
        out.remove(name);
    }
    out.remove(header::CONTENT_LENGTH);
    out
}

fn method_has_body(method: &Method) -> bool {
    method != Method::GET && method != Method::HEAD && method != Method::OPTIONS
}

fn to_reqwest_method(method: &Method) -> reqwest::Method {
    reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET)
}

async fn passthrough_response(resp: reqwest::Response) -> Response {
    let status = resp.status();
    let headers = forwarded_response_headers(resp.headers());
    let stream = resp
        .bytes_stream()
        .map(|chunk| chunk.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
    let mut builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        builder = builder.header(name.clone(), value.clone());
    }
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

/// 上游响应载体：2xx 保留可流式读取的响应；4xx 为判定 `stream_options` 兼容性
/// 读出错误体时直接携带文本，避免二次读取。
enum ForwardPayload {
    Live(reqwest::Response),
    Text(String),
}

/// `reasoning_content` 回传开关的状态变化（仅用于日志）。
struct ReasoningRcChange {
    /// true = 本次打开了回传（上游要求历史回传思维链），false = 本次关闭（上游不接受该字段）。
    enabled: bool,
    /// 触发变化的上游错误原文（已截断）。
    detail: String,
}

/// chat 消息形态统计（诊断用）：只看 assistant 消息里带 `tool_calls` / 带 `content`
/// 的条数，以及各自带回传字段 `reasoning_content` 的条数。
#[derive(Debug, Default, PartialEq, Eq)]
struct MessageShape {
    total: usize,
    with_tools: usize,
    with_tools_rc: usize,
    with_content: usize,
    with_content_rc: usize,
}

fn message_shape(body: &Value) -> MessageShape {
    let mut shape = MessageShape::default();
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return shape;
    };
    shape.total = messages.len();
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let has_rc = message.get("reasoning_content").is_some();
        if message.get("tool_calls").is_some() {
            shape.with_tools += 1;
            if has_rc {
                shape.with_tools_rc += 1;
            }
        }
        // content 为 null 的（聚合出的 tool_calls 消息）不算"文本消息"
        if message.get("content").is_some_and(|c| !c.is_null()) {
            shape.with_content += 1;
            if has_rc {
                shape.with_content_rc += 1;
            }
        }
    }
    shape
}

/// 一次转发的完整结果：状态、响应载体、历史修复统计、被摘掉的可选字段。
struct Forwarded {
    status: StatusCode,
    payload: ForwardPayload,
    repairs: RepairReport,
    dropped_fields: Vec<&'static str>,
    /// 本次请求是否回传了 `reasoning_content`。
    reasoning_rc: bool,
    /// 本次请求是否改变了回传开关（改变时记一条 warn 日志）。
    reasoning_rc_change: Option<ReasoningRcChange>,
    /// 本次尝试的内容日志句柄（请求体已落盘，响应侧继续写）。
    trace: TraceCall,
}

/// 可选请求字段：取到值才加入请求体；上游指名拒绝时按需摘除后重试一次。
struct OptionalField {
    /// 写入 chat 请求体的字段名。
    key: &'static str,
    /// 上游错误体里指认该字段的关键词（小写比较）。
    needles: &'static [&'static str],
    value: Value,
}

/// 本次请求要带上的可选字段：流式用量 + 推理强度等透传字段。
fn optional_fields(req: &Value, want_stream: bool) -> Vec<OptionalField> {
    let mut out: Vec<OptionalField> = Vec::new();
    if want_stream {
        out.push(OptionalField {
            key: "stream_options",
            needles: &["stream_options", "include_usage"],
            value: json!({ "include_usage": true }),
        });
    }
    if let Some(effort) = req
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
    {
        out.push(OptionalField {
            key: "reasoning_effort",
            needles: &["reasoning_effort", "reasoning effort"],
            value: json!(effort),
        });
    }
    if let Some(v) = req.get("parallel_tool_calls").filter(|v| v.is_boolean()) {
        out.push(OptionalField {
            key: "parallel_tool_calls",
            needles: &["parallel_tool_calls"],
            value: v.clone(),
        });
    }
    if let Some(v) = req
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        out.push(OptionalField {
            key: "prompt_cache_key",
            needles: &["prompt_cache_key"],
            value: json!(v),
        });
    }
    if let Some(v) = req
        .get("service_tier")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        out.push(OptionalField {
            key: "service_tier",
            needles: &["service_tier"],
            value: json!(v),
        });
    }
    if let Some(choice) = tool_choice_to_chat(req.get("tool_choice")) {
        out.push(OptionalField {
            key: "tool_choice",
            needles: &["tool_choice"],
            value: choice,
        });
    }
    out
}

/// Zen 免费层的 `max_tokens` 补法：只在入站请求**没有**自己的输出预算（`max_output_tokens`
/// 缺失或为 null）时才补 [`ZEN_MAX_TOKENS`]，客户端显式给的预算一律尊重。
///
/// 走 [`OptionalField`] 而不是直接写进请求体，是为了复用既有的降级路径：上游若在 4xx
/// 错误体里指名 `max_tokens`（某些模型中上游拒绝超出自身上限的值），会被摘掉后重试一次。
fn zen_max_tokens_field(req: &Value) -> Option<OptionalField> {
    match req.get("max_output_tokens") {
        Some(v) if !v.is_null() => None,
        _ => Some(OptionalField {
            key: "max_tokens",
            needles: &["max_tokens"],
            value: json!(ZEN_MAX_TOKENS),
        }),
    }
}

/// 请求体形状补丁（只对 Zen 上游调用，见 [`is_zen_upstream`]）：把门禁要求的
/// [`ZEN_REQUIRED_TOOL_NAMES`] 以假工具追加到 `tools` **末尾**。
///
/// 只补缺失的名字：客户端已声明的真实工具（含命名空间扁平名与自由格式工具）一律保留、
/// 顺序不变，同名时以客户端声明为准（不覆盖）；入站请求原本没有 `tools` 时，数组只含这
/// 几个假工具。幂等：重复调用不会叠加。
///
/// 补完工具后另把「弃用工具声明」教学写进 `messages`（见 [`inject_fake_tools_instruction`]）：
/// 上游是 chat/completions，没有 `instructions` 字段，教学只能走消息列表。
fn patch_zen_request_body(body: &mut Value) {
    let existing = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // chat 侧工具名就是 `tools[i].function.name`（命名空间已展开成扁平名，自由格式工具
    // 也是单参数函数），因此按这个路径取名字即可与既有去重口径一致。
    let mut seen: HashSet<String> = existing
        .iter()
        .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    let mut tools = existing;
    let mut added: Vec<&str> = Vec::new();
    for name in ZEN_REQUIRED_TOOL_NAMES {
        if !seen.insert(name.to_string()) {
            continue;
        }
        added.push(name);
        tools.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": ZEN_FAKE_TOOL_DESCRIPTION,
                "parameters": { "type": "object", "properties": {} }
            }
        }));
    }
    body["tools"] = Value::Array(tools);
    // 动态教学：只列出真正被追加的工具名，与 description 形成双重约束
    if let Some(text) = fake_tools_instruction(&added) {
        inject_fake_tools_instruction(body, &text);
    }
}

/// 根据实际追加的工具名列表动态生成弃用教学文本；列表为空时返回 None。
fn fake_tools_instruction(added: &[&str]) -> Option<String> {
    if added.is_empty() {
        return None;
    }
    Some(format!(
        "【弃用工具声明】以下工具已弃用，禁止调用：{}。这些工具名仅用于满足客户端门禁校验，实际没有任何实现。如果你需要执行类似操作，请使用可用的工具完成任务。",
        added.join("、")
    ))
}

/// 把弃用工具教学文本并入发往上游的 `messages`（**不是**顶层 `instructions`）。
///
/// 上游是 chat/completions，协议里没有 `instructions` 这个字段：入站的 Responses
/// `instructions` 早在 [`responses_to_chat`] 阶段就被消费成了 `messages[0]` 的 system 消息，
/// 翻译后的请求体顶层根本没有该键——写在那里只会给上游多发一个未知字段，模型永远看不到。
/// 因此教学只能落进消息列表：
/// - 首条是 system 消息且 `content` 是字符串 → 追加到它末尾（原内容逐字保留、仍是前缀）；
/// - 否则（没有 system 消息，或首条 system 的 `content` 不是字符串）→ 在 `messages` **最前**
///   插一条独立 system 消息，已有消息顺序不变。
///
/// 幂等：目标 system 消息已包含该文本时直接返回（首次调用后首条必为带教学的 system 消息，
/// 重复调用天然命中）。`messages` 缺失或不是数组时按空数组新建（纯函数，不 panic）。
fn inject_fake_tools_instruction(body: &mut Value, text: &str) {
    if !body.get("messages").is_some_and(Value::is_array) {
        body["messages"] = json!([]);
    }
    let leading_system_text = body["messages"]
        .as_array()
        .and_then(|messages| messages.first())
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("system"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    match leading_system_text {
        Some(existing) if existing.contains(text) => {}
        Some(existing) => {
            body["messages"][0]["content"] = Value::String(format!("{existing}\n\n{text}"));
        }
        None => {
            if let Some(messages) = body["messages"].as_array_mut() {
                messages.insert(0, json!({ "role": "system", "content": text }));
            }
        }
    }
}

/// Responses 的 `tool_choice` → chat 形态：字符串直接透传，
/// `{type:"function",name}` 转 `{type:"function",function:{name}}`，
/// 其余（`allowed_tools` 等无 chat 等价物）返回 None。
fn tool_choice_to_chat(choice: Option<&Value>) -> Option<Value> {
    let choice = choice.filter(|v| !v.is_null())?;
    match choice {
        Value::String(s) if !s.trim().is_empty() => Some(json!(s)),
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("function") => {
            let name = object.get("name").and_then(Value::as_str)?;
            Some(json!({ "type": "function", "function": { "name": name } }))
        }
        _ => None,
    }
}

/// 错误体是否指认了该可选字段。
fn mentions_field(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

/// 命名空间工具在 Chat Completions 侧的扁平名：`{namespace}_{tool}`。
/// Chat Completions 没有命名空间概念（函数名只允许 `[A-Za-z0-9_-]`，codex 也不认裸命名空间名），
/// 因此命名空间必须展开成扁平函数名发给上游，回译时再还原成 `{name, namespace}`。
fn flat_namespace_tool_name(namespace: &str, name: &str) -> String {
    format!("{namespace}_{name}")
}

/// 一次请求的工具形态：命名空间需要展开/还原，自由格式工具（`type:"custom"`，如 apply_patch）
/// 需要走 `custom_tool_call` 回译，两者都按工具名查表。
#[derive(Default, Clone)]
struct ToolShape {
    /// 命名空间工具的「扁平名 → （命名空间, 子工具名）」。
    namespaces: HashMap<String, (String, String)>,
    /// 自由格式工具的（chat 侧）工具名集合。
    custom: HashSet<String>,
}

impl ToolShape {
    /// chat 侧工具名 → Responses 的 `(name, namespace)`：命中命名空间映射时回带命名空间。
    fn resolve(&self, name: &str) -> (String, Option<String>) {
        match self.namespaces.get(name) {
            Some((namespace, tool)) => (tool.clone(), Some(namespace.clone())),
            None => (name.to_string(), None),
        }
    }

    /// 是否为自由格式工具（回译成 `custom_tool_call`、参数允许非 JSON 文本）。
    fn is_custom(&self, name: &str) -> bool {
        self.custom.contains(name)
    }
}

/// 从 Responses 工具声明收集本次请求的工具形态（纯函数，便于单测）。
fn tool_shape(req: &Value) -> ToolShape {
    let mut shape = ToolShape::default();
    let Some(tools) = req.get("tools").and_then(Value::as_array) else {
        return shape;
    };
    for tool in tools {
        match tool.get("type").and_then(Value::as_str) {
            Some("namespace") => {
                let Some(namespace) = tool.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let Some(children) = tool.get("tools").and_then(Value::as_array) else {
                    continue;
                };
                for child in children {
                    let Some(name) = child.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    shape
                        .namespaces
                        .entry(flat_namespace_tool_name(namespace, name))
                        .or_insert_with(|| (namespace.to_string(), name.to_string()));
                }
            }
            Some("custom") => {
                if let Some(name) = tool.get("name").and_then(Value::as_str) {
                    shape.custom.insert(name.to_string());
                }
            }
            _ => {}
        }
    }
    shape
}

/// 是否自由格式的 `apply_patch` 工具（按名匹配，大小写不敏感）。
fn is_apply_patch_tool(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("apply_patch")
}

/// `apply_patch` 的补丁语法规范（**英文原文**，从 codex 声明的 lark grammar 提炼）。
///
/// codex 的补丁语法只存在于自由格式工具声明的 `format.definition`（lark grammar）里，
/// description 本身只有「可以编辑文件 + 别包 JSON」两句；而 Chat Completions 没有语法约束
/// 能力，代理也不下发 grammar，于是上游模型（真机实测 mimo 系）完全不知道 `@@` 的语义，
/// 自创 `@@ 中文描述 @@` 的 hunk 头 → codex 判 `apply_patch verification failed: Failed to find
/// context …` → 反复重试到回合结束。这段文字就是补上那份缺失的语法说明。
/// 模型可见文案统一英文（补丁关键字、参数名与 codex 原生描述都是英文）。
fn patch_syntax_spec() -> &'static str {
    r"Patch syntax (follow it exactly):
*** Begin Patch
*** Update File: <path>
@@ <optional: one line that exists verbatim in the file, e.g. a function signature or struct field>
 <context line: copied verbatim from the file, keep the leading space and the indentation>
-old line
+new line
*** End Patch

Rules:
- Enclose the whole patch between `*** Begin Patch` and `*** End Patch`. One patch may carry several file blocks and several hunks.
- File blocks: `*** Update File: <path>`, `*** Add File: <path>` (every following line starts with `+`), `*** Delete File: <path>`, and `*** Move to: <path>` (only directly after an Update File block, to rename it).
- A hunk header is either `@@` alone or `@@ <a line that exists verbatim in the file>`. Never write `@@ some description @@` or any prose text there: everything after `@@` is searched as a file line, so a description always fails the whole patch.
- Line prefixes: one space marks an unchanged context line, `-` a removed line, `+` an added line.
- Context lines must match the file byte-for-byte, indentation included. Read the file first; never rebuild context from memory.
- Append `*** End of File` to anchor a hunk at the end of the file."
}

/// 自由格式工具（apply_patch 等）在上游的调用约定：Chat Completions 只有函数调用，
/// 因此把整段文本放进 JSON 的 `input` 字段；codex 原描述里的 FREEFORM 提示会误导模型，
/// 命中时整段替换为中性说明，其余情况保留原文再追加约定。
/// `apply_patch` 另补一份补丁语法规范（见 `patch_syntax_spec`），其它自由格式工具不加。
fn custom_tool_description(raw: &str, name: &str) -> String {
    const HINT: &str =
        "This channel is a function call: put the full patch text in the JSON \"input\" field.";
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let misleading =
        trimmed.is_empty() || lower.contains("freeform") || lower.contains("do not wrap");
    let base = if misleading {
        format!("Edit files ({name} patch language).")
    } else {
        trimmed.to_string()
    };
    let mut out = format!("{base}\n\n{HINT}");
    if is_apply_patch_tool(name) {
        out.push_str("\n\n");
        out.push_str(patch_syntax_spec());
    }
    out
}

/// 自由格式工具在 chat 侧的参数 schema：单个 `input` 字符串承载全部文本。
fn custom_tool_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "input": {
                "type": "string",
                "description": "The full patch text, from \"*** Begin Patch\" to \"*** End Patch\"."
            }
        },
        "required": ["input"]
    })
}

/// 自由格式工具调用要交给 codex 的 `input`：优先取 JSON 对象的 `input` 字段，
/// 否则（模型直接给补丁原文、或 JSON 里没有 input）原样使用参数文本。
fn custom_tool_input(arguments: &str) -> String {
    match serde_json::from_str::<Value>(arguments.trim()) {
        Ok(Value::Object(obj)) => obj
            .get("input")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| arguments.to_string()),
        _ => arguments.to_string(),
    }
}

/// codex 补丁校验失败的原文标记（小写比较，避免大小写差异漏判）。
const PATCH_FAILURE_MARKERS: [&str; 4] = [
    "apply_patch verification failed",
    "failed to find context",
    "invalid patch",
    "invalid context",
];

/// 工具输出是否为补丁校验失败。
fn is_patch_failure(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    PATCH_FAILURE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// 补丁失败后追加给上游的纠错提示（**英文原文**）。
///
/// codex 自己的失败原文只说「找不到某段上下文」，弱模型据此只会换一种错法反复试
/// （真机实测 mimo 系会连续发出 `@@ 描述 @@` 形态的 hunk）。这段提示点明 `@@` 的真实语义
/// 并给出最小正确形状，附在**发往上游**的那条工具结果后面；codex 侧的记录与 rollout 不变。
fn patch_failure_hint() -> &'static str {
    r"[apply_patch format correction] The patch above was rejected because a hunk header was wrong.
- A hunk header must be `@@` alone or `@@ <a line that exists verbatim in the file>`.
- Never write `@@ some description @@`: whatever follows `@@` is looked up as a file line, so a description or any prose text can only fail.
- Context lines start with exactly one space and must be copied verbatim from the file, indentation included.
- Read the file first, then retry in the correct shape:
*** Begin Patch
*** Update File: <path>
@@ <verbatim anchor line, optional>
 <verbatim context line>
-old line
+new line
*** End Patch"
}

/// 本次请求历史里「确实失败了」的自由格式工具调用（`call_id` 集合）。
///
/// 判据同时要求：`call_id` 来自本段历史里的 `custom_tool_call`（自由格式工具的历史回放形态，
/// 含 apply_patch），且对应的 `custom_tool_call_output` 文本命中补丁失败标记——只认这两条，
/// 避免把恰好打印了同样字样的普通命令输出也算成补丁失败（纠错提示与诊断统计共用同一判据）。
fn patch_failure_call_ids(items: &[Value]) -> HashSet<String> {
    let custom: HashSet<&str> = items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("custom_tool_call"))
        .filter_map(|item| item.get("call_id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .collect();
    if custom.is_empty() {
        return HashSet::new();
    }
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("custom_tool_call_output"))
        .filter_map(|item| {
            let id = item.get("call_id").and_then(Value::as_str)?;
            if !custom.contains(id) {
                return None;
            }
            let text = value_to_text(item.get("output").unwrap_or(&Value::Null));
            is_patch_failure(&text).then(|| id.to_string())
        })
        .collect()
}

/// 本次请求历史里已失败的补丁调用条数（诊断用，0 表示历史干净）。
fn patch_failure_count(req: &Value) -> usize {
    req.get("input")
        .and_then(Value::as_array)
        .map(|items| patch_failure_call_ids(items).len())
        .unwrap_or(0)
}

/// 内容日志的请求侧元信息：`Authorization` 只记 presence，任何情况下不落盘 API Key。
#[allow(clippy::too_many_arguments)]
fn note_request_meta(
    call: &mut TraceCall,
    url: &str,
    request_id: &str,
    headers: &HeaderMap,
    session: &str,
    session_codex: Option<&str>,
    identity: bool,
    body: &Value,
    dropped: &[&'static str],
    reasoning_rc: bool,
) {
    call.note("upstream_url", url.to_string());
    call.note(
        "authorization",
        if headers.contains_key(header::AUTHORIZATION) {
            "present"
        } else {
            "absent"
        },
    );
    // 身份开关关闭时不发识别头，日志同样记 `-`（否则会误导成「已发过这个会话 id」）
    call.note("identity", if identity { "on" } else { "off" });
    call.note(
        "x_opencode_session",
        if identity {
            session.to_string()
        } else {
            "-".to_string()
        },
    );
    call.note(
        "session_codex",
        if identity {
            session_codex.unwrap_or("-").to_string()
        } else {
            "-".to_string()
        },
    );
    call.note(
        "x_opencode_request",
        if identity {
            request_id.to_string()
        } else {
            "-".to_string()
        },
    );
    call.note(
        "model",
        body.get("model")
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_string(),
    );
    call.note(
        "stream",
        body.get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .to_string(),
    );
    call.note(
        "message_count",
        body.get("messages")
            .and_then(Value::as_array)
            .map(|m| m.len())
            .unwrap_or(0)
            .to_string(),
    );
    call.note(
        "tool_count",
        body.get("tools")
            .and_then(Value::as_array)
            .map(|t| t.len())
            .unwrap_or(0)
            .to_string(),
    );
    call.note(
        "dropped_fields",
        if dropped.is_empty() {
            "-".to_string()
        } else {
            dropped.join(",")
        },
    );
    call.note("reasoning_rc", if reasoning_rc { "on" } else { "off" });
}

/// 翻译请求并转发到 Zen（含历史净化与可选字段降级重试）。
/// `state` 提供上游地址（测试时可指向本地 mock）、会话映射与代理级状态。
/// `call_id` 用于把本次入站请求的多次尝试（内容日志）关联到同一条 session 日志。
async fn forward(
    req: &Value,
    headers: &HeaderMap,
    want_stream: bool,
    state: &ProxyState,
    call_id: &str,
) -> Result<Forwarded, (StatusCode, String)> {
    let url = format!("{}/chat/completions", state.base_url.trim_end_matches('/'));
    let client = http_client();
    // 本会话发往上游的 x-opencode-session（同一 codex 线程在代理生命周期内恒定）
    let session = opencode_session(state, headers);
    // 反查：把实发的 ses_* 映射回 codex 会话 id，落进内容日志便于人工对照
    let session_codex = state.session_map.codex_of(&session);
    let mut optional = optional_fields(req, want_stream);
    // Zen 免费层的请求体门禁之一：补 `max_tokens`（与 messages 同级）。它与下面的工具名
    // 补丁**同受 `zen_body_patch` 约束**——只有上游 host 含有 `opencode` 时才加，
    // 换成 DeepSeek 等自建/第三方端点时两项都不加（无法只开其中一项）。放进可选字段列表
    // 是为了上游指名拒绝它能走既有「摘掉后重试一次」的降级。
    if state.zen_body_patch && state.opencode_identity {
        if let Some(field) = zen_max_tokens_field(req) {
            optional.push(field);
        }
    }
    let mut dropped: Vec<&'static str> = Vec::new();
    let mut reasoning_rc = state.requires_reasoning_rc.load(Ordering::Relaxed);
    let mut reasoning_rc_change: Option<ReasoningRcChange> = None;
    // 一次请求内最多切换一次 reasoning_content 回传方向，避免"开了又关"地来回重试。
    let mut reasoning_toggled = false;
    let mut attempts = 0usize;
    // 上游尝试序号（从 1 开始，含重试），用于内容日志文件名与摘要。
    let mut attempt_no = 0usize;
    loop {
        attempt_no += 1;
        let (mut body, repairs) = responses_to_chat(req, want_stream, reasoning_rc)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("请求翻译失败：{e}")))?;
        // 同一道门禁的另一项：工具名每次尝试都补一遍（`dropped` 只影响可选字段，不影响它）。
        // 与上面的 `max_tokens` 完全同源，同样只在上游 host 含有 `opencode` 时为真。
        if state.zen_body_patch && state.opencode_identity {
            patch_zen_request_body(&mut body);
        }
        for field in optional.iter() {
            if dropped.contains(&field.key) {
                continue;
            }
            body[field.key] = field.value.clone();
        }
        // 内容诊断日志：先落盘本次尝试实际发出的请求体与关键元信息（不含 API Key）
        let mut call = state.trace.begin(call_id, attempt_no);
        let request_id = opencode_id("msg", false);
        note_request_meta(
            &mut call,
            &url,
            &request_id,
            headers,
            &session,
            session_codex.as_deref(),
            state.opencode_identity,
            &body,
            &dropped,
            reasoning_rc,
        );
        call.write_request_json(&body);
        let resp = build_chat_request(client, &url, state, headers, &session, &request_id, &body)
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("上游请求失败：{e}")))?;
        let status = resp.status();
        call.note("upstream_status", status.as_u16().to_string());
        // 4xx 且还有重试额度时按需修复后重发（最多 FORWARD_MAX_ATTEMPTS 次请求）：
        // ① 可选字段被指名拒绝 → 摘掉重试；② 思考模式要求回传 reasoning_content（或反向）→ 切换后重试；
        //  两者都没命中（如纯参数校验失败）则直接返回上游错误，避免反复发请求。
        if attempts + 1 < FORWARD_MAX_ATTEMPTS && status.is_client_error() {
            attempts += 1;
            let text = resp.text().await.unwrap_or_default();
            let shape = message_shape(&body);
            log_at(
                &state.log,
                "warn",
                "zen_proxy.forward_attempt",
                &[
                    ("attempt", attempts.to_string()),
                    ("status", status.as_u16().to_string()),
                    ("detail", truncate_chars(&text, 200)),
                    ("messages", shape.total.to_string()),
                    ("assistant_with_tools", shape.with_tools.to_string()),
                    ("assistant_with_tools_rc", shape.with_tools_rc.to_string()),
                    ("assistant_with_content", shape.with_content.to_string()),
                    (
                        "assistant_with_content_rc",
                        shape.with_content_rc.to_string(),
                    ),
                ],
            );
            let hit: Vec<&'static str> = optional
                .iter()
                .filter(|field| {
                    !dropped.contains(&field.key) && mentions_field(&text, field.needles)
                })
                .map(|field| field.key)
                .collect();
            let toggle_reasoning = hit.is_empty()
                && !reasoning_toggled
                && mentions_field(&text, &["reasoning_content"]);
            if !hit.is_empty() || toggle_reasoning {
                // 内容诊断日志：只对**确实会重试**的尝试落错误原文与重试标记
                // （不可重试时错误体由 proxy_error_text 落盘，避免同一份写两次）
                call.note("retried", "true");
                call.write_response_text("response.txt", &text);
                let flags = suspicious_flags(&SuspiciousFacts {
                    upstream_http_error: true,
                    ..SuspiciousFacts::default()
                });
                call.note("suspicious", flags_field(&flags));
                call.finish();
            }
            if !hit.is_empty() {
                dropped.extend(hit);
                continue;
            }
            // DeepSeek 思考模式：本轮 assistant 消息（带 tool_calls 的与带 content 的）都要回传
            // reasoning_content。上游既然明确要求，就打开开关（反向：本已回传却被拒绝，则关掉）
            // 并重试一次——一次请求最多翻转一次；400 不产生副作用，重试安全。
            if toggle_reasoning {
                reasoning_toggled = true;
                let enabled = !reasoning_rc;
                reasoning_rc = enabled;
                state
                    .requires_reasoning_rc
                    .store(enabled, Ordering::Relaxed);
                reasoning_rc_change = Some(ReasoningRcChange {
                    enabled,
                    detail: truncate_chars(&text, 200),
                });
                continue;
            }
            return Ok(Forwarded {
                status,
                payload: ForwardPayload::Text(text),
                repairs,
                dropped_fields: Vec::new(),
                reasoning_rc,
                reasoning_rc_change,
                trace: call,
            });
        }
        return Ok(Forwarded {
            status,
            payload: ForwardPayload::Live(resp),
            repairs,
            dropped_fields: dropped,
            reasoning_rc,
            reasoning_rc_change,
            trace: call,
        });
    }
}

/// 构造发往 Zen 的 chat/completions 请求：识别头/UA 按「OpenCode 客户端身份」开关决定
/// （见 [`apply_opencode_identity`]），Authorization 始终原样透传。
fn build_chat_request(
    client: &reqwest::Client,
    url: &str,
    state: &ProxyState,
    headers: &HeaderMap,
    session: &str,
    request_id: &str,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut rq = client.post(url).json(body);
    let mut identity = HeaderMap::new();
    apply_opencode_identity(&mut identity, headers, state, session, request_id);
    for (name, value) in identity.iter() {
        rq = rq.header(name, value.clone());
    }
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if let Ok(v) = auth.to_str() {
            rq = rq.header(header::AUTHORIZATION, v.to_string());
        }
    }
    rq
}

/// 非流式：把 Zen 的 chat.completion JSON 翻译为 responses 对象。
/// 可疑结束判定的输入事实（纯数据结构，便于单测）。
#[derive(Default)]
struct SuspiciousFacts<'a> {
    finish_reason: Option<&'a str>,
    text_chars: usize,
    reasoning_chars: usize,
    call_count: usize,
    failed: bool,
    usage_present: bool,
    usage_timeout: bool,
    upstream_http_error: bool,
    /// 本次请求历史里已有失败的补丁调用（模型在补丁格式上打转的信号）。
    patch_retry: bool,
}

/// 启发式「可疑结束」标记（纯函数）：命中多项时按固定顺序返回，未命中返回空。
/// 目的只是让「回合提前结束（任务未完成）」这类偶发问题能一眼筛出来，
/// 判定本身不改变任何行为。
fn suspicious_flags(facts: &SuspiciousFacts<'_>) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    if facts.upstream_http_error {
        out.push("upstream_http_error");
    }
    if facts.failed {
        out.push("failed");
    }
    if facts.finish_reason == Some("length") {
        out.push("truncated");
    }
    if facts.patch_retry {
        out.push("patch_retry");
    }
    if facts.finish_reason.is_none_or(|reason| reason.is_empty()) {
        out.push("finish_reason_missing");
    }
    if facts.finish_reason == Some("stop") && facts.call_count == 0 {
        if facts.text_chars == 0 && facts.reasoning_chars == 0 {
            out.push("stop_without_output");
        } else {
            out.push("stop_without_tool_call");
        }
    }
    if !facts.usage_present {
        out.push("usage_missing");
    }
    if facts.usage_timeout {
        out.push("usage_timeout");
    }
    out
}

/// 标记列表 → 日志字段值（无标记记 `-`）。
fn flags_field(flags: &[&'static str]) -> String {
    if flags.is_empty() {
        "-".to_string()
    } else {
        flags.join(",")
    }
}

/// 汇总非流式 chat 响应的关键事实，并写入可疑结束标记。
fn note_chat_result(call: &mut TraceCall, chat: &Value) {
    let choice = chat
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first());
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = choice.and_then(|c| c.get("message"));
    let text_chars = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .map(|t| t.chars().count())
        .unwrap_or(0);
    let reasoning_chars = ["reasoning_content", "reasoning"]
        .iter()
        .find_map(|key| {
            message
                .and_then(|m| m.get(*key))
                .and_then(Value::as_str)
                .map(|t| t.chars().count())
        })
        .unwrap_or(0);
    let call_count = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array)
        .map(|calls| calls.len())
        .unwrap_or(0);
    let usage = chat.get("usage").filter(|v| !v.is_null());
    call.note("finish_reason", finish_reason.clone().unwrap_or_default());
    call.note("text_chars", text_chars.to_string());
    call.note("reasoning_chars", reasoning_chars.to_string());
    call.note("call_count", call_count.to_string());
    call.note("usage", if usage.is_some() { "present" } else { "none" });
    if let Some(usage) = usage {
        call.note("usage_raw", truncate_chars(&usage.to_string(), 300));
    }
    let flags = suspicious_flags(&SuspiciousFacts {
        finish_reason: finish_reason.as_deref(),
        text_chars,
        reasoning_chars,
        call_count,
        failed: false,
        usage_present: usage.is_some(),
        usage_timeout: false,
        upstream_http_error: false,
        // 非流式路径拿不到入站请求历史，补丁重试标记只在流式收尾统计。
        patch_retry: false,
    });
    call.note("suspicious", flags_field(&flags));
}

/// 非流式：把 Zen 的 chat.completion JSON 翻译为 responses 对象。
async fn proxy_json_response(
    req: Value,
    resp: reqwest::Response,
    log: &Option<Arc<SessionLog>>,
    strip_tags: bool,
    mut call: TraceCall,
) -> Response {
    let status = resp.status();
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            call.note("read_error", e.to_string());
            call.note("suspicious", "upstream_read_error");
            call.finish();
            return error_json(StatusCode::BAD_GATEWAY, format!("读取上游响应失败：{e}"));
        }
    };
    // 内容诊断日志：上游整份响应原文
    call.write_response_text("response.json", &text);
    let Ok(chat) = serde_json::from_str::<Value>(&text) else {
        log_at(
            log,
            "warn",
            "zen_proxy.response_parse_error",
            &[(
                "detail",
                "上游响应不是合法 JSON：".to_string() + &text.chars().take(200).collect::<String>(),
            )],
        );
        call.note("suspicious", "response_parse_error");
        call.finish();
        return error_json(
            StatusCode::BAD_GATEWAY,
            "上游响应不是合法 JSON：".to_string() + &text.chars().take(200).collect::<String>(),
        );
    };
    let model = req
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let out = match chat_to_responses(&chat, &model, &tool_shape(&req), strip_tags) {
        Ok(out) => out,
        Err(e) => {
            log_at(
                log,
                "warn",
                "zen_proxy.malformed_tool_call",
                &[("reason", e.clone())],
            );
            call.note("suspicious", "malformed_tool_call");
            call.note("failure_detail", e.clone());
            call.finish();
            return error_json(StatusCode::BAD_GATEWAY, e);
        }
    };
    if let Some(usage) = chat.get("usage").filter(|v| !v.is_null()) {
        log_at(
            log,
            "info",
            "zen_proxy.usage",
            &[("detail", truncate_chars(&usage.to_string(), 300))],
        );
    }
    let body = serde_json::to_string(&out).unwrap_or_else(|_| "{}".into());
    log_at(
        log,
        "info",
        "zen_proxy.response",
        &[
            ("status", status.as_u16().to_string()),
            ("event_bytes", body.len().to_string()),
        ],
    );
    note_chat_result(&mut call, &chat);
    call.note("translated_response", body.clone());
    call.finish();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

/// 读取一轮上游 SSE：把 data 行翻译成 responses 事件并送入通道。
///
/// 只负责「读一轮」，**不**调用 `finish_stream`——收尾由调用方在所有 pass 结束后统一发一次，
/// 这样「口嗨检测 + 续跑」才能把第二轮的事件续在同一个响应流里。
/// `trace` 为这一轮的内容日志句柄（第一轮是入站 call，续跑轮各有自己的）。
async fn pump_stream(
    resp: reqwest::Response,
    st: &mut StreamState,
    log: &ZenLog,
    tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
    trace: &mut TraceCall,
) {
    let mut bytes = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut finish_deadline: Option<Instant> = None;
    loop {
        // finish_reason 之后还要再等尾部分片（OpenAI 的 `include_usage` 把 usage 放在
        // 最后一个独立分片里）；最多等 USAGE_GRACE。
        let next = if st.finish_reason.is_some() {
            let deadline = *finish_deadline.get_or_insert_with(|| Instant::now() + USAGE_GRACE);
            match tokio::time::timeout(
                deadline.saturating_duration_since(Instant::now()),
                bytes.next(),
            )
            .await
            {
                Ok(item) => item,
                Err(_) => {
                    log_at(
                        log,
                        "warn",
                        "zen_proxy.usage_timeout",
                        &[(
                            "detail",
                            "finish_reason 后未再收到分片，按现状收尾".to_string(),
                        )],
                    );
                    st.usage_timeout = true;
                    return;
                }
            }
        } else {
            bytes.next().await
        };
        match next {
            Some(Ok(chunk)) => {
                buf.extend_from_slice(&chunk);
                let mut out: Vec<String> = Vec::new();
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line);
                    let line = line.trim_end();
                    // 内容诊断日志：上游分行原样落盘（含非 data 行）
                    trace.write_response_line(line);
                    if line.is_empty() || !line.starts_with("data:") {
                        continue;
                    }
                    let payload = line["data:".len()..].trim();
                    if payload == "[DONE]" {
                        log_at(log, "info", "zen_proxy.stream_done", &[]);
                        if !out.is_empty() {
                            let _ = tx.send(out.concat().into_bytes()).await;
                        }
                        return;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(payload) {
                        out.extend(process_chunk(&v, st));
                        // 上游随流下发 error：立即停止读取（失败收尾由调用方统一发）
                        if st.failure.is_some() {
                            if !out.is_empty() {
                                let _ = tx.send(out.concat().into_bytes()).await;
                            }
                            return;
                        }
                    }
                }
                if !out.is_empty() && tx.send(out.concat().into_bytes()).await.is_err() {
                    return;
                }
            }
            Some(Err(_)) => {
                // 上游读取出错：按失败收尾（原先按完成收尾会让回合静默结束）
                if st.failure.is_none() {
                    st.failure = Some(StreamFailure {
                        code: CODE_UPSTREAM_STREAM_ERROR.to_string(),
                        message: "上游连接中断，本轮未正常结束；请重试".to_string(),
                        detail: "上游读取出错（连接中断或超时）".to_string(),
                    });
                }
                return;
            }
            None => {
                log_at(
                    log,
                    "info",
                    "zen_proxy.stream_end",
                    &[("detail", "上游未发送 [DONE]，补发完成事件".to_string())],
                );
                // 上游正常结束但未收到 [DONE]（兼容实现差异）：按现状收尾
                return;
            }
        }
    }
}

/// 流式：把 Zen 的 SSE data 行翻译为 responses 事件序列（text/event-stream）。
///
/// 实际处理在 `run_stream_task`：读第一轮上游流，终局若命中「纯文本 + 无工具调用」就
/// 走口嗨决策链——标题生成请求整轮放行；两个模式都用**纯代码判据**决定收尾还是续跑：
/// 计划模式看计划系标签（[`PLAN_OUTPUT_MARKER`] / [`PLAN_CANCEL_MARKER`] /
/// [`PLAN_UNACHIEVABLE_MARKER`]，命中即收尾，都不命中才注入 [`PLAN_NUDGE_TEXT`] 催计划）；
/// 默认模式看模型自己回的 [`TASK_COMPLETED_MARKER`]（命中即收尾清零计数），不命中就注入
/// [`NUDGE_TEXT`] 再打一轮上游、把第二轮事件续在同一个响应流里（工具调用照常下发给 codex，
/// 对 codex 透明），最后统一收尾。**没有任何独立的判定请求**——判定就发生在这一段注入的
/// 对话轮里。
async fn proxy_stream_response(
    state: ProxyState,
    headers: HeaderMap,
    call_id: String,
    req: Value,
    resp: reqwest::Response,
    call: TraceCall,
) -> Response {
    let model = req
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let response_id = gen_id("resp");
    let created = sse_event(
        "response.created",
        &json!({
            "type": "response.created",
            "response": {
                "id": response_id,
                "object": "response",
                "created_at": unix_now(),
                "status": "in_progress",
                "model": model,
                "output": [],
                "error": null,
            }
        }),
    );
    // 通道 + 独立任务：事件一边产出一边下发，续跑都发生在同一个响应流里
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    tokio::spawn(async move {
        run_stream_task(
            state,
            headers,
            call_id,
            req,
            resp,
            call,
            response_id,
            model,
            created,
            tx,
        )
        .await;
    });
    let body = stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|item| (Ok::<_, std::convert::Infallible>(item), rx))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header("Cache-Control", "no-cache")
        .body(Body::from_stream(body))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

/// 一个入站流式请求的完整处理：原始 pass（+ 必要时续跑 pass）→ 统一收尾。
#[allow(clippy::too_many_arguments)]
async fn run_stream_task(
    state: ProxyState,
    headers: HeaderMap,
    call_id: String,
    req: Value,
    first_resp: reqwest::Response,
    call: TraceCall,
    response_id: String,
    model: String,
    created: String,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    let log: ZenLog = state.log.clone();
    let mut st = StreamState::new(response_id, model.clone());
    // 标签剥离与「回合收尾强制约束」同开关：关闭时文本直通（不剪标签、不催办）
    st.strip_tags = state.nudge_enabled;
    // 内容诊断日志句柄随流状态走：收尾与中断（Drop）时都能落盘
    st.trace = call;
    // 工具形态：回译模型调用时还原 codex 期望的 name/namespace 或 custom_tool_call
    st.tool_shape = tool_shape(&req);
    // 历史里的补丁失败条数：收尾时进摘要与可疑标记（模型是否在补丁格式上打转）
    st.patch_failures = patch_failure_count(&req);
    if tx.send(created.into_bytes()).await.is_err() {
        return;
    }

    let has_tools = request_has_tools(&req);
    // 口嗨判据完全由代码给：计划模式看计划系标签，默认模式看 [`TASK_COMPLETED_MARKER`]。
    // 模式本身优先取协议登记表（`run_stream_task` 与 `handle_responses` 用的是同一个纯函数，
    // 两处结论必然一致），查不到才退回关键词兜底。
    let (plan_mode, _) = resolve_nudge_mode(&state, &headers, &req);
    // 会话标题生成任务（应用的后台临时线程）：整轮放行，不催办也不注入
    let title_task = request_is_title_task(&req);
    let session = opencode_session(&state, &headers);

    let mut body = req.clone();
    let mut current = first_resp;
    let mut pass = 0usize;
    // 本次入站请求已注入的续跑提醒次数（只活在这个循环里，见 NUDGE_MAX_INJECTIONS）
    let mut injections = 0usize;
    let mut next_trace: Option<TraceCall> = None;
    loop {
        pass += 1;
        let text_len_before = st.text.as_ref().map(|t| t.text_buf.len()).unwrap_or(0);
        if pass == 1 {
            // 第一轮沿用入站 call：收尾摘要仍写在它上面（与改造前一致）
            let mut trace = std::mem::replace(&mut st.trace, TraceCall::disabled());
            pump_stream(current, &mut st, &log, &tx, &mut trace).await;
            st.trace = trace;
        } else {
            // 上一轮的 finish_reason 必须清掉：它会让 `pump_stream` 从本轮第一个分片起
            // 就套用 3 秒尾包宽限，把慢上游的第二轮截断（工具调用会被丢掉）。
            st.finish_reason = None;
            let mut trace = next_trace.take().unwrap_or_else(TraceCall::disabled);
            pump_stream(current, &mut st, &log, &tx, &mut trace).await;
            trace.note("nudge_pass", pass.to_string());
            trace.finish();
        }

        // 失败：交给收尾统一发 response.failed
        if st.failure.is_some() {
            break;
        }
        // 模型真的调用了工具：本轮口嗨已被纠正（后续回合走新的入站请求）
        if !st.calls.is_empty() {
            break;
        }
        // 「回合收尾强制约束」关闭：终局判定与续跑整段跳过（本轮怎么收尾完全由上游决定），
        // 每个入站请求只记一条诊断；后续分支（标题线程放行、催办、催办上限）都不会走到。
        // 放在标题线程判定之前：关闭开关后标题线程也无需再单独记一条 `title_task`
        if !state.nudge_enabled {
            log_at(
                &log,
                "info",
                "zen_proxy.nudge_skipped",
                &[
                    ("原因", "disabled".to_string()),
                    ("轮次", pass.to_string()),
                    ("说明", NUDGE_SKIP_NOTE_DISABLED.to_string()),
                ],
            );
            break;
        }
        // 会话标题生成任务：直接放行（标题线程必然「纯文本 + 零工具调用」结束，
        // 注入只会白花一轮上游），不注入
        if title_task {
            log_at(
                &log,
                "info",
                "zen_proxy.nudge_skipped",
                &[
                    ("原因", "title_task".to_string()),
                    ("轮次", pass.to_string()),
                    ("说明", NUDGE_SKIP_NOTE_TITLE_TASK.to_string()),
                ],
            );
            break;
        }
        let pass_text = st
            .text
            .as_ref()
            .map(|t| t.text_buf[text_len_before..].to_string())
            .unwrap_or_default();
        // 只处理「纯文本 + 正常结束」的终局：无文本、被截断、无可调工具都不催办
        if !has_tools || st.finish_reason.as_deref() != Some("stop") || pass_text.trim().is_empty()
        {
            break;
        }
        // 计划模式：只用代码判据决定本轮怎么收尾——终局带计划标签即已交付、直接收尾；
        // 带「已取消计划」标记则视为用户已放弃、带「无法或无需计划」标记则视为本就不产出
        // 计划，三者都直接收尾；三者都没有才按「未交付」催它给出计划（计划专用提醒）
        let nudge_mode = if plan_mode { "plan" } else { "default" };
        if plan_mode && is_plan_deliverable(&pass_text) {
            log_at(
                &log,
                "info",
                "zen_proxy.nudge_skipped",
                &[
                    ("原因", "plan_output".to_string()),
                    ("轮次", pass.to_string()),
                    ("模式", nudge_mode.to_string()),
                    ("说明", "计划已交付，直接收尾".to_string()),
                ],
            );
            break;
        }
        // 用户已取消这个计划（提醒里约定的标签）：计划话题已终结，直接收尾——
        // 既不再催它给一份已被放弃的方案，也不触发后续 pass
        if plan_mode && is_plan_cancelled(&pass_text) {
            log_at(
                &log,
                "info",
                "zen_proxy.nudge_skipped",
                &[
                    ("原因", "plan_cancelled".to_string()),
                    ("轮次", pass.to_string()),
                    ("模式", nudge_mode.to_string()),
                    ("说明", "用户已放弃计划，直接收尾".to_string()),
                    ("标签内容", tag_payload(&st)),
                ],
            );
            break;
        }
        // 问题本身无法或无需产出实现计划（提醒里约定的标签）：同样直接收尾——
        // 不逼它在计划模式里强行给方案，也不触发后续 pass
        if plan_mode && is_plan_unachievable(&pass_text) {
            log_at(
                &log,
                "info",
                "zen_proxy.nudge_skipped",
                &[
                    ("原因", "plan_unachievable".to_string()),
                    ("轮次", pass.to_string()),
                    ("模式", nudge_mode.to_string()),
                    ("说明", "问题无需实现计划，直接收尾".to_string()),
                    ("标签内容", tag_payload(&st)),
                ],
            );
            break;
        }
        // 默认模式：终局带「任务已经结束」标签（首轮教学与提醒里约定的 `<zen_task_completed>`），
        // 即模型自己宣告任务已终结——直接收尾，不再注入、不触发后续 pass。
        if !plan_mode && is_task_completed(&pass_text) {
            log_at(
                &log,
                "info",
                "zen_proxy.nudge_skipped",
                &[
                    ("原因", "task_completed".to_string()),
                    ("轮次", pass.to_string()),
                    ("模式", nudge_mode.to_string()),
                    ("说明", "模型已自行宣告任务结束，直接收尾".to_string()),
                    ("标签内容", tag_payload(&st)),
                ],
            );
            break;
        }
        // 单请求内的注入上限：达到后不再注入（计数只在本次请求的循环里累加，不跨请求）
        if injections >= NUDGE_MAX_INJECTIONS {
            log_at(
                &log,
                "warn",
                "zen_proxy.nudge_limited",
                &[
                    ("原因", "max_injections".to_string()),
                    ("轮次", pass.to_string()),
                    ("模式", nudge_mode.to_string()),
                    (
                        "说明",
                        format!("本请求已催办 {NUDGE_MAX_INJECTIONS} 次仍未收尾，停止催办"),
                    ),
                ],
            );
            break;
        }
        // 两个模式各用各的提醒：默认模式那条负责把 `<zen_task_completed>` 教给模型
        // （判定搬进原对话，不再有独立的判定请求）
        let nudge_text = if plan_mode {
            PLAN_NUDGE_TEXT
        } else {
            NUDGE_TEXT
        };
        // 注入提醒并续跑：第二轮的事件继续喂同一个 StreamState
        body = nudge_continuation_body(&body, &pass_text, nudge_text);
        log_at(
            &log,
            "info",
            "zen_proxy.nudge_injected",
            &[
                ("会话", session.clone()),
                ("轮次", pass.to_string()),
                (
                    "本请求催办次数",
                    format!("{}/{NUDGE_MAX_INJECTIONS}", injections + 1),
                ),
                ("模式", nudge_mode.to_string()),
                ("助手字数", pass_text.chars().count().to_string()),
                ("说明", nudge_injected_note(plan_mode).to_string()),
            ],
        );
        injections += 1;
        let nudge_call_id = format!("{call_id}-nudge{pass}");
        match forward(&body, &headers, true, &state, &nudge_call_id).await {
            Ok(forwarded) if forwarded.status.is_success() => match forwarded.payload {
                ForwardPayload::Live(resp) => {
                    next_trace = Some(forwarded.trace);
                    current = resp;
                }
                ForwardPayload::Text(text) => {
                    log_at(
                        &log,
                        "warn",
                        "zen_proxy.nudge_skipped",
                        &[
                            ("原因", "nudge_upstream_error".to_string()),
                            ("说明", "续跑轮上游返回错误，已放弃本次催办".to_string()),
                            ("详情", truncate_chars(&text, 200)),
                        ],
                    );
                    break;
                }
            },
            Ok(forwarded) => {
                log_at(
                    &log,
                    "warn",
                    "zen_proxy.nudge_skipped",
                    &[
                        ("原因", "nudge_upstream_status".to_string()),
                        ("说明", "续跑轮上游非 2xx，已放弃本次催办".to_string()),
                        ("状态", forwarded.status.as_u16().to_string()),
                    ],
                );
                break;
            }
            Err((_, message)) => {
                log_at(
                    &log,
                    "warn",
                    "zen_proxy.nudge_skipped",
                    &[
                        ("原因", "nudge_forward_error".to_string()),
                        ("说明", "续跑轮转发失败，已放弃本次催办".to_string()),
                        ("详情", truncate_chars(&message, 200)),
                    ],
                );
                break;
            }
        }
    }

    let terminal = finish_stream(&mut st, &log).concat();
    let _ = tx.send(terminal.into_bytes()).await;
}

/// `zen_proxy.nudge_injected` 的中文 `说明`：一句话讲清「为什么催、要求什么」。
fn nudge_injected_note(plan_mode: bool) -> &'static str {
    if plan_mode {
        "模型以纯文本收尾且未交付计划，已注入计划续跑提醒（要求给出 <proposed_plan> 或两个出口标签）"
    } else {
        "模型以纯文本收尾且未宣告完成，已注入续跑提醒（要求继续调用工具或回 <zen_task_completed>）"
    }
}

/// `zen_proxy.nudge_skipped` 里会话标题线程的中文 `说明`（其余原因的说明就地写在日志调用处）。
const NUDGE_SKIP_NOTE_TITLE_TASK: &str = "会话标题线程，整轮放行";
/// `zen_proxy.nudge_skipped` 里「回合收尾强制约束」开关关闭时的中文 `说明`。
const NUDGE_SKIP_NOTE_DISABLED: &str = "回合收尾强制约束已关闭，不注入也不催办";

/// 日志用：本次流里被结构剥离掉的标签载荷（标签不再下发，这里是唯一回看入口）。
fn tag_payload(st: &StreamState) -> String {
    st.text
        .as_ref()
        .map(|t| t.stripper.stripped_note())
        .unwrap_or_default()
}

/// **结构化剥离器**：把发给 codex 的文本里三个自研标签整段剪掉（标签只服务代理判定，
/// 不该污染会话内容）。匹配只看结构——「开标签前缀 → 闭标签」，**载荷写什么、写多长、
/// 是不是教学里要求的固定短词，一概不参与匹配**。
///
/// 规则：
/// 1. 命中任一开标签前缀即进入标签态，此后内容压住不下发，直到闭标签；
/// 2. 闭标签取三个 zen 闭标签里**最早出现**的那个（同名优先命中，弱模型写错闭标签也不会
///    把整条消息吞掉），整段丢弃并记进 `stripped`；
/// 3. 支持跨分片：开/闭标签被切成多段（甚至逐字符）到达也成立；
/// 4. 流结束时仍在标签态（未闭合）→ 丢弃标签开头到文本结尾（与「缺闭合标签也命中」的判据一致）；
/// 5. 一行里多个标签逐个处理；不做嵌套解析（进入标签后遇到第一个闭标签即结束）；
/// 6. 标签**独占一行**时连行首缩进与行尾换行一起去掉，避免聊天里留空行；同一行还有别的正文时
///    只删标签本身，同行其余文本保留。
#[derive(Debug, Default)]
struct TagStripper {
    /// 是否启用剥离（= 「回合收尾强制约束」开关）：关闭时**直通**——文本原样下发、
    /// 不记录载荷，也不存在跨分片 holdback，行为与没有这段逻辑完全一致。
    enabled: bool,
    /// 还没判定完、暂时不能下发的尾巴（可能是开标签的开头几个字符）。
    pending: String,
    /// 已进入标签态：正在等闭标签（`pending` 此时就是载荷 + 可能的半个闭标签）。
    inside: bool,
    /// 进入的标签是否独占一行（决定删完后要不要连行尾换行一起吞掉）。
    inside_line_tag: bool,
    /// 当前已下发文本的「本行至今只有空白」状态：为真且标签前只有空白时，该标签独占一行。
    line_start_ws: bool,
    /// 刚删掉一个整行标签：正在吞掉紧随其后的空白与一个换行。
    trim_after_tag: bool,
    /// 本次流里被删掉的标签载荷（截断、条数封顶），供日志回看。
    stripped: Vec<String>,
}

impl TagStripper {
    /// 构造剥离器：`enabled=false` 时为直通模式（见字段说明）。
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            line_start_ws: true,
            ..Self::default()
        }
    }

    /// 喂一段上游文本，返回可以下发给 codex 的可见文本（可能为空）。
    /// 直通模式下逐字返回原文。
    fn feed(&mut self, text: &str) -> String {
        if !self.enabled {
            return text.to_string();
        }
        self.pending.push_str(text);
        let mut visible = String::new();
        loop {
            if self.trim_after_tag {
                let cut = self.take_line_tail();
                if !cut {
                    break;
                }
                continue;
            }
            if self.inside {
                match find_close_tag(&self.pending) {
                    Some((at, len)) => {
                        let payload = self.pending[..at].to_string();
                        self.record_payload(&payload);
                        self.pending.drain(..at + len);
                        self.inside = false;
                        // 只有「标签独占一行」时，才继续吞掉行尾空白与换行
                        self.trim_after_tag = self.inside_line_tag;
                        self.inside_line_tag = false;
                        continue;
                    }
                    // 整段都还可能是载荷（或半个闭标签）：继续压住
                    None => break,
                }
            }
            match find_open_tag(&self.pending) {
                Some((at, len)) => {
                    let prefix = self.pending[..at].to_string();
                    // 标签是否独占一行：标签之前（本行内）只有空白与缩进
                    let line_ws_at_tag = line_state_after(self.line_start_ws, &prefix);
                    let keep = if line_ws_at_tag {
                        // 丢掉这一行的缩进，但保留前面各行的正文与换行
                        prefix[..prefix.rfind('\n').map(|i| i + 1).unwrap_or(0)].to_string()
                    } else {
                        prefix
                    };
                    self.note_visible(&keep);
                    visible.push_str(&keep);
                    self.pending.drain(..at + len);
                    self.inside = true;
                    self.inside_line_tag = line_ws_at_tag;
                    continue;
                }
                None => {
                    // 没有开标签：把「可能是开标签开头」的尾巴压住，其余原样下发
                    let hold = held_prefix_len(&self.pending);
                    let cut = self.pending.len() - hold;
                    if cut > 0 {
                        let head: String = self.pending.drain(..cut).collect();
                        self.note_visible(&head);
                        visible.push_str(&head);
                    }
                    break;
                }
            }
        }
        visible
    }

    /// 流结束：未闭合的标签整段丢弃；否则把压住的尾巴原样交出。
    fn finish(&mut self) -> String {
        if self.inside {
            let payload = std::mem::take(&mut self.pending);
            self.record_payload(&payload);
            self.inside = false;
            self.trim_after_tag = false;
            return String::new();
        }
        self.trim_after_tag = false;
        let tail = std::mem::take(&mut self.pending);
        if !tail.is_empty() {
            self.note_visible(&tail);
        }
        tail
    }

    /// 供日志：本次流里被删掉的标签载荷（含尚未闭合的那一段），截断后拼接。
    fn stripped_note(&self) -> String {
        let mut all = self.stripped.clone();
        if self.inside {
            all.push(truncate_chars(
                &clean_payload(&self.pending),
                TAG_PAYLOAD_CHARS,
            ));
        }
        all.retain(|item| !item.is_empty());
        truncate_chars(&all.join(" / "), TAG_PAYLOAD_CHARS * 2)
    }

    /// 一次性剥离（非流式路径用）：返回 (可见文本, 被删载荷)。
    /// `enabled=false` 时原样返回文本、载荷为空。
    fn strip_once(text: &str, enabled: bool) -> (String, String) {
        let mut stripper = Self::new(enabled);
        let mut visible = stripper.feed(text);
        visible.push_str(&stripper.finish());
        (visible, stripper.stripped_note())
    }

    /// 吞掉整行标签后面的空白与一个换行；返回是否还需要继续处理 `pending`。
    fn take_line_tail(&mut self) -> bool {
        let mut consumed = 0;
        for (idx, ch) in self.pending.char_indices() {
            if ch == '\n' {
                self.pending.drain(..idx + ch.len_utf8());
                self.trim_after_tag = false;
                self.line_start_ws = true;
                return true;
            }
            if !is_line_blank(ch) {
                // 同一行后面还有正文：停止吞空白，正文照常下发
                if consumed > 0 {
                    self.pending.drain(..consumed);
                }
                self.trim_after_tag = false;
                self.line_start_ws = false;
                return true;
            }
            consumed = idx + ch.len_utf8();
        }
        // 目前只有空白：先吃掉，等后续分片
        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        false
    }

    /// 记录一条被删载荷（截断；空载荷也记，便于区分「有标签」与「没标签」）。
    fn record_payload(&mut self, payload: &str) {
        if self.stripped.len() >= TAG_PAYLOAD_ENTRIES {
            return;
        }
        self.stripped
            .push(truncate_chars(&clean_payload(payload), TAG_PAYLOAD_CHARS));
    }

    /// 维护「本行至今只有空白」状态。
    fn note_visible(&mut self, text: &str) {
        self.line_start_ws = line_state_after(self.line_start_ws, text);
    }
}

/// 行首缩进允许的空白（不含换行本身）。
fn is_line_blank(ch: char) -> bool {
    ch == ' ' || ch == '\t' || ch == '\r'
}

/// 走过一段已下发文本后，「本行至今只有空白」的状态（换行重置为真，非空白字符置为假）。
fn line_state_after(mut state: bool, text: &str) -> bool {
    for ch in text.chars() {
        if ch == '\n' {
            state = true;
        } else if !is_line_blank(ch) {
            state = false;
        }
    }
    state
}

/// 载荷清理：去掉开标签残留的 `>`、行首缩进与首尾空白（只影响日志里记的内容）。
fn clean_payload(raw: &str) -> String {
    raw.trim_start_matches(|ch: char| is_line_blank(ch) || ch == '>')
        .trim()
        .to_string()
}

/// 三个 zen 闭标签里最早出现的那个，返回 (起点, 长度)。
fn find_close_tag(hay: &str) -> Option<(usize, usize)> {
    let lower = hay.to_lowercase();
    ZEN_TAG_PAIRS
        .iter()
        .filter_map(|(_, close)| lower.find(close).map(|at| (at, close.len())))
        .min_by_key(|(at, _)| *at)
}

/// 三个 zen 开标签里最早出现的那个，返回 (起点, 开标签长度)。
fn find_open_tag(hay: &str) -> Option<(usize, usize)> {
    let lower = hay.to_lowercase();
    ZEN_TAG_PAIRS
        .iter()
        .filter_map(|(open, _)| lower.find(open).map(|at| (at, open.len())))
        .min_by_key(|(at, _)| *at)
}

/// `pending` 末尾有多少字符可能是某个开标签的开头（必须压住不下发）。
fn held_prefix_len(hay: &str) -> usize {
    let lower = hay.to_lowercase();
    let mut hold = 0;
    for (open, _) in ZEN_TAG_PAIRS {
        let max = open.len().min(lower.len());
        for len in 1..=max {
            if lower.ends_with(&open[..len]) {
                hold = hold.max(len);
            }
        }
    }
    hold
}

/// 上游非 2xx：透传状态码与错误体。
async fn proxy_error_response(
    status: StatusCode,
    resp: reqwest::Response,
    log: &Option<Arc<SessionLog>>,
    call: TraceCall,
) -> Response {
    let text = resp.text().await.unwrap_or_default();
    proxy_error_text(status, &text, log, call)
}

/// 上游非 2xx 且错误体已读出：透传状态码与错误体。
fn proxy_error_text(
    status: StatusCode,
    text: &str,
    log: &Option<Arc<SessionLog>>,
    mut call: TraceCall,
) -> Response {
    log_at(
        log,
        "warn",
        "zen_proxy.upstream_error",
        &[
            ("status", status.as_u16().to_string()),
            ("detail", text.chars().take(200).collect::<String>()),
        ],
    );
    // 内容诊断日志：非 2xx 原文全文 + 可疑标记
    call.note("upstream_status", status.as_u16().to_string());
    call.write_response_text("response.txt", text);
    let flags = suspicious_flags(&SuspiciousFacts {
        upstream_http_error: !status.is_success(),
        ..SuspiciousFacts::default()
    });
    call.note("suspicious", flags_field(&flags));
    call.finish();
    let body = serde_json::from_str::<Value>(text).unwrap_or_else(|_| {
        json!({ "error": { "message": text.chars().take(400).collect::<String>(), "type": "upstream_error" } })
    });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&body).unwrap_or_else(|_| "{}".into()),
        ))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

fn error_json(status: StatusCode, msg: String) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(
                &json!({ "error": { "message": msg, "type": "zen_proxy_error" } }),
            )
            .unwrap_or_else(|_| "{}".into()),
        ))
        .unwrap_or_else(|_| Response::new(Body::from("{}")))
}

fn sse_event(name: &str, data: &Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 当前 Unix 毫秒时间戳（opencode 标识里的时间部分）。
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 当前 Unix 纳秒时间戳（仅用于随机源回落的播种）。
fn unix_now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn gen_id(prefix: &str) -> String {
    // 进程内自增 + 时间戳，保证同一会话中稳定递增且跨进程不重复
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let now = unix_now();
    format!("{prefix}_{now}_{n:x}")
}

// ---------------------------------------------------------------------------
// 请求翻译：Responses → Chat Completions
// ---------------------------------------------------------------------------

/// 历史净化统计：被改写的非法工具参数条数、补出的悬空工具结果条数。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RepairReport {
    invalid_arguments: usize,
    missing_tool_outputs: usize,
}

/// Responses 角色 → Chat Completions 角色。
/// `developer` 是 Responses 的开发者指令角色（codex 用它下发开发者消息），但多数 OpenAI
/// 兼容端点不认它——DeepSeek 直接回 400 `unknown variant developer`——统一降级为 `system`；
/// 其余角色原样透传，交给上游自行校验。
fn chat_role(role: &str) -> &str {
    match role {
        "developer" => "system",
        other => other,
    }
}

/// 把 Responses 请求体转换为 Chat Completions 请求体（纯函数，便于单测）。
///
/// 同时净化历史：工具调用参数被截断/非法时改写为 `{}`，并有工具调用却缺少
/// 对应结果时补一条合成 `tool` 消息——Chat Completions 要求二者严格配对，
/// 否则整条会话会被上游以 400/500 永久拒绝。
fn responses_to_chat(
    req: &Value,
    want_stream: bool,
    attach_reasoning: bool,
) -> Result<(Value, RepairReport), String> {
    if !req.is_object() {
        return Err("请求体必须是 JSON 对象".into());
    }
    let mut repairs = RepairReport::default();
    let model = req.get("model").cloned().unwrap_or(Value::Null);
    let mut messages: Vec<Value> = Vec::new();

    if let Some(instructions) = req.get("instructions").and_then(|v| v.as_str()) {
        if !instructions.is_empty() {
            messages.push(json!({ "role": "system", "content": instructions }));
        }
    }

    let input = req.get("input");
    match input {
        Some(Value::String(s)) if !s.is_empty() => {
            messages.push(json!({ "role": "user", "content": s }));
        }
        Some(Value::Array(items)) => {
            // 连续的 function_call 聚合成一条 assistant + tool_calls 消息
            let mut pending: Option<Value> = None;
            // 待回传的思维链文本：来自回放的 `reasoning` item，只有真的需要
            // （`attach_reasoning`）时才挂到下一条工具调用消息上。
            let mut pending_reasoning: Option<String> = None;
            // 本段历史里已经失败的自定义工具调用：对应工具结果要补一条格式纠错提示，
            // 否则弱模型只会换一种错法继续重试（见 `patch_failure_hint`）。
            let failed_patches = patch_failure_call_ids(items);
            let flush = |messages: &mut Vec<Value>, pending: &mut Option<Value>| {
                if let Some(p) = pending.take() {
                    messages.push(p);
                }
            };
            for item in items {
                let Some(obj) = item.as_object() else {
                    continue;
                };
                match obj.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "message" => {
                        let role = obj.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                        let role = chat_role(role);
                        if role == "assistant" {
                            // 同一轮的文本消息也要带该轮思维链：上游声明 tools 时（DeepSeek 实测）
                            // 校验会一直查到这条"进度文本"消息，只补 tool_calls 那条不够。
                            // 先推文本、再落地 tool_calls 聚合消息，保证 tool_calls 紧跟其工具结果。
                            let mut message = json!({
                                "role": role,
                                "content": message_content(item),
                            });
                            if attach_reasoning {
                                message["reasoning_content"] =
                                    Value::String(pending_reasoning.clone().unwrap_or_default());
                            }
                            messages.push(message);
                            flush(&mut messages, &mut pending);
                        } else {
                            // 非 assistant 的 message 意味着进入新一轮，丢弃未被消费的思维链，
                            // 避免把上一轮的推理挂到下一轮的工具调用上。
                            pending_reasoning = None;
                            flush(&mut messages, &mut pending);
                            messages.push(json!({
                                "role": role,
                                "content": message_content(item),
                            }));
                        }
                    }
                    // 思维链：Responses 用 `reasoning` item 回放，chat 侧按需挂到本轮 assistant 消息上。
                    "reasoning" => {
                        if let Some(text) = reasoning_item_text(item) {
                            pending_reasoning = Some(text);
                        }
                    }
                    "function_call" | "custom_tool_call" => {
                        let call_id = obj
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        // 历史里的命名空间调用（`name` + `namespace`）：回放时也要用扁平名，
                        // 与工具声明的写法一致，否则上游会看到「未声明过的函数名」。
                        let raw_name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let name = match obj.get("namespace").and_then(|v| v.as_str()) {
                            Some(namespace) if !namespace.is_empty() && !raw_name.is_empty() => {
                                flat_namespace_tool_name(namespace, raw_name)
                            }
                            _ => raw_name.to_string(),
                        };
                        // 自由格式工具的历史调用：把 `input` 包成上游函数调用形态（`{"input": …}`），
                        // 与工具声明的单参数 schema 一致；普通函数调用照旧规范化 arguments。
                        let arguments = if obj.get("type").and_then(Value::as_str)
                            == Some("custom_tool_call")
                        {
                            let input = obj.get("input").map(value_to_text).unwrap_or_default();
                            json!({ "input": input }).to_string()
                        } else {
                            let raw_arguments =
                                obj.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                            let (arguments, repaired) = normalize_arguments(raw_arguments);
                            if repaired {
                                repairs.invalid_arguments += 1;
                            }
                            arguments
                        };
                        let tc = json!({
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments }
                        });
                        if let Some(p) = pending.as_mut() {
                            p["tool_calls"].as_array_mut().map(|arr| arr.push(tc));
                        } else {
                            let mut message = json!({
                                "role": "assistant",
                                "content": Value::Null,
                                "tool_calls": [tc]
                            });
                            if attach_reasoning {
                                // 上游（DeepSeek 思考模式）要求带 tool_calls 的历史消息回传
                                // reasoning_content；没有可用文本时写空串（实测同样被接受）。
                                message["reasoning_content"] =
                                    Value::String(pending_reasoning.take().unwrap_or_default());
                            }
                            pending = Some(message);
                        }
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        flush(&mut messages, &mut pending);
                        let call_id = obj
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let mut content = value_to_text(obj.get("output").unwrap_or(&Value::Null));
                        // 补丁校验失败：把格式纠错提示附在同一条工具结果后面（只影响本次
                        // 发往上游的历史，codex 自己的记录与 rollout 不变）。
                        if failed_patches.contains(&call_id) {
                            content.push('\n');
                            content.push_str(patch_failure_hint());
                        }
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": content
                        }));
                    }
                    _ => {
                        // 其它未知类型：无法映射，忽略
                    }
                }
            }
            flush(&mut messages, &mut pending);
            repair_tool_pairing(&mut messages, &mut repairs);
        }
        _ => {}
    }

    let mut chat = json!({
        "model": model,
        "messages": messages,
        "stream": want_stream,
    });

    if let Some(tools) = req.get("tools").and_then(|t| t.as_array()) {
        let mut chat_tools: Vec<Value> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for tool in tools {
            let Some(obj) = tool.as_object() else {
                continue;
            };
            // 命名空间工具（codex 用它承载 codexui / 多智能体等分组）：chat 侧没有命名空间，
            // 展开成 `{namespace}_{tool}` 扁平函数，参数与说明取自子工具。
            if obj.get("type").and_then(Value::as_str) == Some("namespace") {
                let Some(namespace) = obj.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let Some(children) = obj.get("tools").and_then(Value::as_array) else {
                    continue;
                };
                for child in children {
                    let Some(name) = child.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let flat = flat_namespace_tool_name(namespace, name);
                    if !seen.insert(flat.clone()) {
                        continue;
                    }
                    let mut function = json!({ "name": flat });
                    if let Some(desc) = child.get("description").and_then(Value::as_str) {
                        function["description"] = Value::String(desc.to_string());
                    }
                    if let Some(params) = child.get("parameters") {
                        function["parameters"] = params.clone();
                    }
                    chat_tools.push(json!({ "type": "function", "function": function }));
                }
                continue;
            }
            // 自由格式工具（codex 的 `apply_patch`）：上游只有函数调用，暴露成单参数 `input` 的函数，
            // 回译时再还原成 `custom_tool_call`（否则 codex 会判 `incompatible payload`）。
            if obj.get("type").and_then(Value::as_str) == Some("custom") {
                let Some(name) = obj.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if !seen.insert(name.to_string()) {
                    continue;
                }
                let raw_desc = obj.get("description").and_then(Value::as_str).unwrap_or("");
                chat_tools.push(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": custom_tool_description(raw_desc, name),
                        "parameters": custom_tool_parameters()
                    }
                }));
                continue;
            }
            let Some(name) = obj.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                continue;
            }
            let mut function = json!({ "name": name });
            if let Some(desc) = obj.get("description").and_then(Value::as_str) {
                function["description"] = Value::String(desc.to_string());
            }
            if let Some(params) = obj.get("parameters") {
                function["parameters"] = params.clone();
            }
            chat_tools.push(json!({ "type": "function", "function": function }));
        }
        if !chat_tools.is_empty() {
            chat["tools"] = Value::Array(chat_tools);
        }
    }

    // 标量映射：Responses 名 → Chat 名；不支持的字段（reasoning/previous_response_id/store 等）不注入
    for (dst, src) in [
        ("max_tokens", "max_output_tokens"),
        ("temperature", "temperature"),
        ("top_p", "top_p"),
        ("seed", "seed"),
    ] {
        if let Some(v) = req.get(src) {
            if !v.is_null() {
                chat[dst] = v.clone();
            }
        }
    }

    Ok((chat, repairs))
}

/// 规范化工具调用参数：空/缺失统一写 `{}`（无参工具），非空但非法 JSON 也写 `{}`
/// 并计入修复（返回值第二项表示"发生了修复"）。
fn normalize_arguments(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ("{}".to_string(), false);
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) => (trimmed.to_string(), false),
        // 数组合法等非对象形态同样视为不可用：Chat Completions 要求 object
        Ok(_) | Err(_) => ("{}".to_string(), true),
    }
}

/// 校验一条工具调用的参数是否可用（流式侧护栏，判定"畸形"）。
/// 工具名必须非空；`arguments` 允许为空串（无参工具），非空时必须是合法 JSON 对象。
/// `custom_tool`（自由格式工具，如 apply_patch）例外：它的参数就是模型给的原文
/// （codex 原描述明确要求"不要包成 JSON"），只要求非空，由 codex 的补丁解析器判好坏。
fn validate_call_arguments(name: &str, args: &str, custom_tool: bool) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("工具名为空".into());
    }
    let trimmed = args.trim();
    if trimmed.is_empty() {
        if custom_tool {
            return Err("自由格式工具内容为空".into());
        }
        return Ok(());
    }
    if custom_tool {
        return Ok(());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) => Ok(()),
        Ok(other) => Err(format!(
            "工具参数不是 JSON 对象：{}",
            truncate_chars(&other.to_string(), 80)
        )),
        Err(e) => Err(format!("工具参数不是合法 JSON：{e}")),
    }
}

/// 为"有工具调用却没有结果"的 assistant 消息补出合成 `tool` 消息。
fn repair_tool_pairing(messages: &mut Vec<Value>, repairs: &mut RepairReport) {
    let mut i = 0usize;
    while i < messages.len() {
        let ids: Vec<String> = messages[i]
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            i += 1;
            continue;
        }
        // 紧随其后的连续 tool 消息即这组调用的结果
        let mut j = i + 1;
        let mut answered: HashSet<String> = HashSet::new();
        while j < messages.len() && messages[j].get("role").and_then(|r| r.as_str()) == Some("tool")
        {
            if let Some(id) = messages[j].get("tool_call_id").and_then(|v| v.as_str()) {
                answered.insert(id.to_string());
            }
            j += 1;
        }
        let missing: Vec<String> = ids
            .into_iter()
            .filter(|id| !answered.contains(id))
            .collect();
        if missing.is_empty() {
            i = j;
            continue;
        }
        let count = missing.len();
        for (offset, id) in missing.into_iter().enumerate() {
            messages.insert(
                j + offset,
                json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": MISSING_TOOL_OUTPUT_TEXT
                }),
            );
        }
        repairs.missing_tool_outputs += count;
        i = j + count;
    }
}

/// 按字符截断（UTF-8 安全），用于日志与错误摘要。
fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect::<String>() + "…"
}

/// 提取 Responses message item 的文本内容（多段 content 拼接为单一字符串）。
fn item_content_text(item: &Value) -> String {
    match item.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                let t = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
                match t {
                    "input_text" | "output_text" => p
                        .get("text")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string()),
                    "input_image" | "input_file" => None,
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// 消息内容：纯文本消息仍用字符串（最大兼容）；含图片时改为 chat 多模态 parts。
/// Responses 的 `input_image` 是 `{type,image_url:<字符串>,detail?}`，chat 需要
/// `{type:"image_url",image_url:{url,detail?}}`；`detail` 仅透传 auto/low/high。
/// 回放的 `reasoning` item → 思维链文本：优先 `summary[].text`（codex 存下的推理内容，
/// 即上一条流里翻出去的原文），为空再退 `content[].text`。
fn reasoning_item_text(item: &Value) -> Option<String> {
    let join = |key: &str| -> Option<String> {
        let texts: Vec<&str> = item
            .get(key)
            .and_then(Value::as_array)?
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .filter(|text| !text.is_empty())
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n"))
        }
    };
    join("summary").or_else(|| join("content"))
}

fn message_content(item: &Value) -> Value {
    let Some(parts) = item.get("content").and_then(Value::as_array) else {
        return Value::String(item_content_text(item));
    };
    let has_image = parts
        .iter()
        .any(|part| part.get("type").and_then(Value::as_str) == Some("input_image"));
    if !has_image {
        return Value::String(item_content_text(item));
    }
    let mut out: Vec<Value> = Vec::new();
    for part in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "input_text" | "output_text" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        out.push(json!({ "type": "text", "text": text }));
                    }
                }
            }
            "input_image" => {
                let Some(url) = part
                    .get("image_url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.trim().is_empty())
                else {
                    continue;
                };
                let mut image = json!({ "url": url });
                if let Some(detail) = part.get("detail").and_then(Value::as_str) {
                    if matches!(detail, "auto" | "low" | "high") {
                        image["detail"] = json!(detail);
                    }
                }
                out.push(json!({ "type": "image_url", "image_url": image }));
            }
            // input_file / input_audio：chat 侧形态差异较大，本次不透传
            _ => {}
        }
    }
    if out.is_empty() {
        return Value::String(item_content_text(item));
    }
    Value::Array(out)
}

/// 任意 JSON 值 → 聊天文本（工具输出等）。
fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 响应翻译：Chat Completions → Responses
// ---------------------------------------------------------------------------

/// 非流式 chat.completion → responses 对象（纯函数，便于单测）。
/// 工具调用参数不可用时返回 Err（调用方以 502 结束，而不是把坏参数交给 codex）。
/// `shape` 为本次请求的工具形态（命名空间还原 + 自由格式工具走 `custom_tool_call`）。
/// `strip_tags` 为「回合收尾强制约束」开关：关闭时不剥离 zen 标签（见 [`TagStripper`]）。
fn chat_to_responses(
    chat: &Value,
    model: &str,
    shape: &ToolShape,
    strip_tags: bool,
) -> Result<Value, String> {
    let chat_id = chat
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("chatcmpl-zen");
    let response_id = format!("resp_{}", chat_id.trim_start_matches("chatcmpl-"));
    let mut output: Vec<Value> = Vec::new();

    if let Some(choices) = chat.get("choices").and_then(|c| c.as_array()) {
        if let Some(first) = choices.first() {
            let msg = first.get("message").cloned().unwrap_or_else(|| json!({}));
            let role = msg
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("assistant")
                .to_string();
            let content = msg
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            // 非流式路径同样按结构剥离三个 zen 标签（一次性，无跨分片问题）；
            // 「回合收尾强制约束」关闭时不剥离，正文原样交给 codex。
            let content = TagStripper::strip_once(&content, strip_tags).0;
            let mut content_parts = Vec::new();
            if !content.is_empty() {
                content_parts.push(json!({
                    "type": "output_text",
                    "text": content,
                    "annotations": []
                }));
            }
            let msg_id = gen_id("msg");
            output.push(json!({
                "id": msg_id,
                "type": "message",
                "role": role,
                "status": "completed",
                "content": content_parts
            }));
            if let Some(tcs) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tcs {
                    let call_id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = tc
                        .pointer("/function/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let raw_arguments = tc
                        .pointer("/function/arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    // 回译形态：命名空间工具写 `name` + `namespace`；自由格式工具写
                    // `custom_tool_call`（`input` 取 JSON 的 input 字段或原始文本）。
                    let (item_name, item_namespace) = shape.resolve(&name);
                    let item = if shape.is_custom(&name) {
                        json!({
                            "id": gen_id("ctc"),
                            "type": "custom_tool_call",
                            "status": "completed",
                            "call_id": call_id,
                            "name": item_name,
                            "input": custom_tool_input(raw_arguments)
                        })
                    } else {
                        let (arguments, repaired) = normalize_arguments(raw_arguments);
                        if repaired {
                            return Err(format!(
                                "上游工具调用参数不可用（{}）：{}",
                                if name.is_empty() {
                                    "未知工具"
                                } else {
                                    &name
                                },
                                truncate_chars(raw_arguments.trim(), 120)
                            ));
                        }
                        let mut item = json!({
                            "id": gen_id("fc"),
                            "type": "function_call",
                            "status": "completed",
                            "call_id": call_id,
                            "name": item_name,
                            "arguments": arguments
                        });
                        if let Some(namespace) = item_namespace {
                            item["namespace"] = Value::String(namespace);
                        }
                        item
                    };
                    output.push(item);
                }
            }
        }
    }

    let usage_out = usage_to_responses(chat.get("usage"));

    Ok(json!({
        "id": response_id,
        "object": "response",
        "created_at": unix_now(),
        "status": "completed",
        "model": model,
        "output": output,
        "error": null,
        "usage": usage_out
    }))
}

/// chat 的 `usage` → responses 的 `usage`（流式与非流式共用同一组字段名）。
/// 除三个总量外还翻译明细：`input_tokens_details.cached_tokens` /
/// `input_tokens_details.cache_write_tokens` / `output_tokens_details.reasoning_tokens`
/// ——codex 用它们填充 `cached_input_tokens` / `cache_write_input_tokens` /
/// `reasoning_output_tokens`（应用侧「缓存读取 / 缓存写入 / 推理输出」三行）。
fn usage_to_responses(usage: Option<&Value>) -> Value {
    let mut out = json!({});
    let Some(usage) = usage else {
        return out;
    };
    for (dst, src) in [
        ("input_tokens", "prompt_tokens"),
        ("output_tokens", "completion_tokens"),
        ("total_tokens", "total_tokens"),
    ] {
        if let Some(v) = usage.get(src).filter(|v| !v.is_null()) {
            out[dst] = v.clone();
        }
    }

    let mut input_details = json!({});
    if let Some(v) = pick_number(
        usage,
        &[
            "prompt_tokens_details.cached_tokens",
            "cache_read_input_tokens",
        ],
    ) {
        input_details["cached_tokens"] = v;
    }
    if let Some(v) = pick_number(
        usage,
        &[
            "cache_write_tokens",
            "prompt_tokens_details.cache_write_tokens",
            "cache_creation_input_tokens",
        ],
    ) {
        input_details["cache_write_tokens"] = v;
    }
    if input_details.as_object().is_some_and(|map| !map.is_empty()) {
        out["input_tokens_details"] = input_details;
    }

    let mut output_details = json!({});
    if let Some(v) = pick_number(
        usage,
        &[
            "completion_tokens_details.reasoning_tokens",
            "reasoning_tokens",
        ],
    ) {
        output_details["reasoning_tokens"] = v;
    }
    if output_details
        .as_object()
        .is_some_and(|map| !map.is_empty())
    {
        out["output_tokens_details"] = output_details;
    }
    out
}

/// 按路径取数值：支持 `a` 与 `a.b`；非数值、缺失、null 都视为未命中。
fn pick_number(value: &Value, paths: &[&str]) -> Option<Value> {
    for path in paths {
        let found = match path.split_once('.') {
            Some((head, tail)) => value.get(head).and_then(|v| v.get(tail)),
            None => value.get(*path),
        };
        if let Some(v) = found.filter(|v| v.is_number()) {
            return Some(v.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 流式翻译：chat.completion.chunk（SSE data 行）→ responses 事件序列
// ---------------------------------------------------------------------------

struct TextTrack {
    item_id: String,
    /// 上游原文（判据、续跑轮回显、字数统计都用它）。
    text_buf: String,
    /// 剥掉三个 zen 标签后**真正下发给 codex** 的文本（delta 与 done 都用它）。
    visible_buf: String,
    /// 结构剥离器（跨分片）。
    stripper: TagStripper,
    out_index: usize,
}

struct CallTrack {
    item_id: String,
    call_id: String,
    name: String,
    args_buf: String,
    out_index: usize,
}

struct StreamState {
    response_id: String,
    model: String,
    /// 是否剥离 zen 收尾标签（= 「回合收尾强制约束」开关）：关闭时文本直通，
    /// 与「不注入教学/不催办」保持一致。
    strip_tags: bool,
    next_index: usize,
    /// 推理摘要（上游 reasoning 增量）；codex 用它渲染「思考过程」。
    reasoning: Option<TextTrack>,
    text: Option<TextTrack>,
    calls: HashMap<usize, CallTrack>,
    closed: bool,
    /// 已判定的失败（上游随流下发的 error / 读取中断）；畸形工具调用在收尾时判定。
    failure: Option<StreamFailure>,
    /// 上游给出的结束原因（`finish_reason`），仅用于观测。
    finish_reason: Option<String>,
    /// 上游随流下发的 token 用量，收尾时映射进 `response.completed`。
    usage: Option<Value>,
    /// 本次流里出现过的 delta 顶层键（去重、有上限），仅用于诊断。
    delta_keys: BTreeSet<String>,
    /// 内容诊断日志句柄（请求体已在转发前落盘，这里继续写上游响应与摘要）。
    trace: TraceCall,
    /// `finish_reason` 后等待尾部分片是否超时（仅用于诊断标记）。
    usage_timeout: bool,
    /// 本次请求的工具形态（命名空间映射 + 自由格式工具名单）：回译模型调用时据此还原成
    /// codex 期望的 `{name, namespace}` 或 `custom_tool_call`。
    tool_shape: ToolShape,
    /// 本次请求历史里已失败的补丁调用条数（诊断用；>0 会在收尾标记 `patch_retry`）。
    patch_failures: usize,
}

impl StreamState {
    /// `strip_tags` 默认开启（与「回合收尾强制约束」的默认值一致）；代理在
    /// `run_stream_task` 里按实际开关覆写它。
    fn new(response_id: String, model: String) -> Self {
        Self {
            response_id,
            model,
            strip_tags: true,
            next_index: 0,
            reasoning: None,
            text: None,
            calls: HashMap::new(),
            closed: false,
            failure: None,
            finish_reason: None,
            usage: None,
            delta_keys: BTreeSet::new(),
            trace: TraceCall::disabled(),
            usage_timeout: false,
            tool_shape: ToolShape::default(),
            patch_failures: 0,
        }
    }
}

/// 一次流内失败：`code` 写入 `response.failed`，`detail` 仅进日志。
#[derive(Debug, Clone)]
struct StreamFailure {
    code: String,
    message: String,
    detail: String,
}

/// 处理一个 chat chunk（choice 数组），返回零到多条 responses SSE 块。
fn process_chunk(chunk: &Value, st: &mut StreamState) -> Vec<String> {
    if st.closed {
        return Vec::new();
    }
    // 上游随流下发的 error（HTTP 仍为 200）：不再静默忽略，直接判定失败
    if let Some(err) = chunk.get("error").filter(|v| !v.is_null()) {
        if st.failure.is_none() {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| "上游流内返回错误".to_string());
            st.failure = Some(StreamFailure {
                code: CODE_UPSTREAM_STREAM_ERROR.to_string(),
                message,
                detail: truncate_chars(&err.to_string(), 300),
            });
        }
        return Vec::new();
    }
    // token 用量（部分实现随任意 chunk 或末块下发）
    if let Some(usage) = chunk.get("usage").filter(|v| !v.is_null()) {
        st.usage = Some(usage.clone());
    }
    let mut out = Vec::new();
    let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
        return out;
    };
    for choice in choices {
        let delta = choice.get("delta").cloned().unwrap_or_else(|| json!({}));
        // 诊断：记录出现过的 delta 键名（只记键，不记值）
        if let Some(object) = delta.as_object() {
            for key in object.keys() {
                if st.delta_keys.len() >= DELTA_KEY_LIMIT {
                    break;
                }
                st.delta_keys.insert(key.clone());
            }
        }
        // 推理增量（DeepSeek 系 reasoning_content / 中转 reasoning）：翻成
        // responses 的 reasoning summary 事件，供「思考过程」卡片展示
        if let Some(text) = reasoning_text(&delta) {
            out.extend(reasoning_delta(text, st));
        }
        // 文本增量
        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                out.extend(text_delta(content, st));
            }
        }
        // 工具调用增量
        if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            for tc in tool_calls {
                let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                out.extend(tool_delta(idx, tc, st));
            }
        }
        // finish_reason 出现即收尾（null/空串跳过）
        if let Some(fr) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            if !fr.is_empty() && fr != "null" {
                if st.finish_reason.is_none() {
                    st.finish_reason = Some(fr.to_string());
                }
                // 不在此收尾：OpenAI 的 `include_usage` 会在 finish_reason 之后
                // 再发一个只带 usage 的分片，提前关流会把它丢掉（见流循环的宽限）。
            }
        }
    }
    out
}

/// 取增量里的推理文本。覆盖常见形态：
/// - `reasoning_content`：DeepSeek 系；
/// - `reasoning`：字符串，或带 `content` / `text` / `summary` 的对象；
/// - `reasoning_details`：OpenRouter 系的数组（取 `text` / `summary`，跳过加密项）。
fn reasoning_text(delta: &Value) -> Option<&str> {
    if let Some(text) = delta
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text);
    }
    match delta.get("reasoning") {
        Some(Value::String(text)) if !text.is_empty() => Some(text.as_str()),
        Some(Value::Object(object)) => reasoning_field(object),
        _ => delta
            .get("reasoning_details")
            .and_then(Value::as_array)
            .and_then(|details| {
                details.iter().find_map(|detail| {
                    let object = detail.as_object()?;
                    // 加密推理没有可展示文本
                    if object
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind.contains("encrypted"))
                    {
                        return None;
                    }
                    reasoning_field(object)
                })
            }),
    }
}

/// 从推理对象里按 `content` / `text` / `summary` 顺序取非空文本。
fn reasoning_field(object: &serde_json::Map<String, Value>) -> Option<&str> {
    ["content", "text", "summary"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .filter(|text| !text.is_empty())
}

/// 推理摘要增量 → `response.reasoning_summary_*` 事件序列（summary_index 固定 0）。
fn reasoning_delta(text: &str, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if st.reasoning.is_none() {
        let out_index = st.next_index;
        st.next_index += 1;
        let item_id = gen_id("rs");
        st.reasoning = Some(TextTrack {
            item_id: item_id.clone(),
            text_buf: String::new(),
            // 推理摘要不剥离标签，可见文本与原文一致（只是复用同一结构）
            visible_buf: String::new(),
            stripper: TagStripper::new(false),
            out_index,
        });
        out.push(sse_event(
            "response.output_item.added",
            &json!({
                "type": "response.output_item.added",
                "output_index": out_index,
                "item": { "id": item_id, "type": "reasoning", "summary": [] }
            }),
        ));
        out.push(sse_event(
            "response.reasoning_summary_part.added",
            &json!({
                "type": "response.reasoning_summary_part.added",
                "item_id": item_id,
                "output_index": out_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": "" }
            }),
        ));
    }
    if let Some(t) = st.reasoning.as_mut() {
        t.text_buf.push_str(text);
        out.push(sse_event(
            "response.reasoning_summary_text.delta",
            &json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "summary_index": 0,
                "delta": text
            }),
        ));
    }
    out
}

fn text_delta(text: &str, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if st.text.is_none() {
        let out_index = st.next_index;
        st.next_index += 1;
        let item_id = gen_id("msg");
        st.text = Some(TextTrack {
            item_id: item_id.clone(),
            text_buf: String::new(),
            visible_buf: String::new(),
            stripper: TagStripper::new(st.strip_tags),
            out_index,
        });
        out.push(sse_event(
            "response.output_item.added",
            &json!({
                "type": "response.output_item.added",
                "output_index": out_index,
                "item": {
                    "id": item_id,
                    "type": "message",
                    "role": "assistant",
                    "status": "in_progress",
                    "content": []
                }
            }),
        ));
        out.push(sse_event(
            "response.content_part.added",
            &json!({
                "type": "response.content_part.added",
                "item_id": item_id,
                "output_index": out_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": "", "annotations": [] }
            }),
        ));
    }
    if let Some(t) = st.text.as_mut() {
        t.text_buf.push_str(text);
        // 只把剥掉 zen 标签后的可见文本下发给 codex（空分片不发，避免下游看到空 delta）
        let visible = t.stripper.feed(text);
        if !visible.is_empty() {
            t.visible_buf.push_str(&visible);
            out.push(sse_event(
                "response.output_text.delta",
                &json!({
                    "type": "response.output_text.delta",
                    "item_id": t.item_id,
                    "output_index": t.out_index,
                    "content_index": 0,
                    "delta": visible
                }),
            ));
        }
    }
    out
}

fn tool_delta(tool_index: usize, tc: &Value, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if !st.calls.contains_key(&tool_index) {
        let out_index = st.next_index;
        st.next_index += 1;
        let item_id = gen_id("fc");
        let call_id = tc
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = tc
            .pointer("/function/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // 回译形态：命名空间工具写回 `name` + `namespace`；自由格式工具是 `custom_tool_call`
        // （`added` 与 `done` 两处必须一致，否则 codex 侧 item 对不上）。
        let (item_name, item_namespace) = st.tool_shape.resolve(&name);
        let is_custom = st.tool_shape.is_custom(&name);
        st.calls.insert(
            tool_index,
            CallTrack {
                item_id: item_id.clone(),
                call_id: call_id.clone(),
                name: name.clone(),
                args_buf: String::new(),
                out_index,
            },
        );
        let item = if is_custom {
            json!({
                "id": item_id,
                "type": "custom_tool_call",
                "status": "in_progress",
                "call_id": call_id,
                "name": item_name,
                "input": ""
            })
        } else {
            let mut item = json!({
                "id": item_id,
                "type": "function_call",
                "status": "in_progress",
                "call_id": call_id,
                "name": item_name,
                "arguments": ""
            });
            if let Some(namespace) = item_namespace {
                item["namespace"] = Value::String(namespace);
            }
            item
        };
        out.push(sse_event(
            "response.output_item.added",
            &json!({
                "type": "response.output_item.added",
                "output_index": out_index,
                "item": item
            }),
        ));
        // 首个 chunk 可能已带参数片段（某些实现）
        if let Some(args) = tc.pointer("/function/arguments").and_then(|v| v.as_str()) {
            if !args.is_empty() {
                out.extend(tool_args_delta(tool_index, args, st));
            }
        }
    } else {
        if let Some(args) = tc.pointer("/function/arguments").and_then(|v| v.as_str()) {
            if !args.is_empty() {
                out.extend(tool_args_delta(tool_index, args, st));
            }
        }
    }
    out
}

fn tool_args_delta(tool_index: usize, args: &str, st: &mut StreamState) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(c) = st.calls.get_mut(&tool_index) {
        c.args_buf.push_str(args);
        // 自由格式工具：增量事件是 `custom_tool_call_input.delta`
        let custom = st.tool_shape.is_custom(&c.name);
        out.push(sse_event(
            if custom {
                "response.custom_tool_call_input.delta"
            } else {
                "response.function_call_arguments.delta"
            },
            &json!({
                "type": if custom {
                    "response.custom_tool_call_input.delta"
                } else {
                    "response.function_call_arguments.delta"
                },
                "item_id": c.item_id,
                "output_index": c.out_index,
                "delta": args
            }),
        ));
    }
    out
}

/// 收尾：文本 item 照常补 done 事件；工具调用正常时补 function_call 的 done 事件
/// 并发 `response.completed`（带 usage）；若上游已失败或存在畸形工具调用，则不发任何
/// function_call、改发 `response.failed`，让回合以明确错误结束而不是静默完成。
/// 无论哪条路径都记一次 `zen_proxy.stream_summary`。
fn finish_stream(st: &mut StreamState, log: &ZenLog) -> Vec<String> {
    if st.closed {
        return Vec::new();
    }
    st.closed = true;
    let mut out = Vec::new();
    let reasoning_chars = st
        .reasoning
        .as_ref()
        .map(|t| t.text_buf.chars().count())
        .unwrap_or(0);
    let text_chars = st
        .text
        .as_ref()
        .map(|t| t.text_buf.chars().count())
        .unwrap_or(0);

    // 推理摘要 item 先收尾（它的 output_index 先分配）
    if let Some(t) = st.reasoning.take() {
        out.push(sse_event(
            "response.reasoning_summary_text.done",
            &json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "summary_index": 0,
                "text": t.text_buf
            }),
        ));
        out.push(sse_event(
            "response.reasoning_summary_part.done",
            &json!({
                "type": "response.reasoning_summary_part.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": t.text_buf }
            }),
        ));
        out.push(sse_event(
            "response.output_item.done",
            &json!({
                "type": "response.output_item.done",
                "output_index": t.out_index,
                "item": {
                    "id": t.item_id,
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": t.text_buf }]
                }
            }),
        ));
    }

    // 剥离掉的字符数（原始 − 可见），进收尾摘要便于对照「标签被删了多少」
    let mut stripped_chars = 0usize;
    if let Some(mut t) = st.text.take() {
        // 收尾前先把剥离器压住的尾巴交出来（未闭合标签按规则丢弃；直通模式下就是原文尾巴），
        // 保证 done / content_part.done / output_item.done 与 delta 累积完全一致
        let tail = t.stripper.finish();
        t.visible_buf.push_str(&tail);
        let visible_text = t.visible_buf.clone();
        stripped_chars = t
            .text_buf
            .chars()
            .count()
            .saturating_sub(visible_text.chars().count());
        out.push(sse_event(
            "response.output_text.done",
            &json!({
                "type": "response.output_text.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "content_index": 0,
                "text": visible_text.clone(),
                "annotations": []
            }),
        ));
        out.push(sse_event(
            "response.content_part.done",
            &json!({
                "type": "response.content_part.done",
                "item_id": t.item_id,
                "output_index": t.out_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": visible_text.clone(), "annotations": [] }
            }),
        ));
        out.push(sse_event(
            "response.output_item.done",
            &json!({
                "type": "response.output_item.done",
                "output_index": t.out_index,
                "item": {
                    "id": t.item_id,
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "text": visible_text.clone(),
                        "annotations": []
                    }]
                }
            }),
        ));
    }

    let mut calls: Vec<_> = st.calls.drain().collect();
    calls.sort_by_key(|(idx, _)| *idx);
    let call_count = calls.len();

    // 上游随流错误：补一条日志（错误详情只在检测处写入状态）
    if let Some(f) = st.failure.as_ref() {
        log_at(
            log,
            "warn",
            "zen_proxy.stream_error",
            &[("code", f.code.clone()), ("detail", f.detail.clone())],
        );
    }

    // 畸形工具调用：整轮判失败，且不发出任何 function_call——既避免半执行，
    // 也避免把非法 JSON 参数写进会话历史（那会让后续每个请求都被上游拒绝）。
    let mut malformed: Option<StreamFailure> = None;
    if st.failure.is_none() {
        for (_, c) in calls.iter() {
            let Err(reason) =
                validate_call_arguments(&c.name, &c.args_buf, st.tool_shape.is_custom(&c.name))
            else {
                continue;
            };
            log_at(
                log,
                "warn",
                "zen_proxy.malformed_tool_call",
                &[
                    ("name", c.name.clone()),
                    ("call_id", c.call_id.clone()),
                    ("args_len", c.args_buf.chars().count().to_string()),
                    ("preview", truncate_chars(c.args_buf.trim(), 200)),
                    ("reason", reason.clone()),
                ],
            );
            malformed = Some(StreamFailure {
                code: CODE_MALFORMED_TOOL_CALL.to_string(),
                message: format!(
                    "上游模型返回的工具调用参数不可用（{}），已阻止该调用进入会话；请重试本轮",
                    if c.name.trim().is_empty() {
                        "未知工具"
                    } else {
                        c.name.trim()
                    }
                ),
                detail: reason,
            });
            break;
        }
    }
    let failure = st.failure.clone().or(malformed);
    let failed = failure.is_some();
    // 被上游截断（finish_reason=length）：照常收尾，但以 incomplete 结束而不是 completed
    let truncated = st.finish_reason.as_deref() == Some("length");

    if failure.is_none() {
        for (_, c) in calls {
            // 回译形态：命名空间工具写回 `name` + `namespace`；自由格式工具写 `custom_tool_call`
            // （`input` 取 JSON 的 input 字段或原始文本）。
            let (item_name, item_namespace) = st.tool_shape.resolve(&c.name);
            let item = if st.tool_shape.is_custom(&c.name) {
                let input = custom_tool_input(&c.args_buf);
                out.push(sse_event(
                    "response.custom_tool_call_input.done",
                    &json!({
                        "type": "response.custom_tool_call_input.done",
                        "item_id": c.item_id,
                        "output_index": c.out_index,
                        "input": input
                    }),
                ));
                json!({
                    "id": c.item_id,
                    "type": "custom_tool_call",
                    "status": "completed",
                    "call_id": c.call_id,
                    "name": item_name,
                    "input": input
                })
            } else {
                out.push(sse_event(
                    "response.function_call_arguments.done",
                    &json!({
                        "type": "response.function_call_arguments.done",
                        "item_id": c.item_id,
                        "output_index": c.out_index,
                        "arguments": c.args_buf
                    }),
                ));
                let mut item = json!({
                    "id": c.item_id,
                    "type": "function_call",
                    "status": "completed",
                    "call_id": c.call_id,
                    "name": item_name,
                    "arguments": c.args_buf
                });
                if let Some(namespace) = item_namespace {
                    item["namespace"] = Value::String(namespace);
                }
                item
            };
            out.push(sse_event(
                "response.output_item.done",
                &json!({
                    "type": "response.output_item.done",
                    "output_index": c.out_index,
                    "item": item
                }),
            ));
        }
    }

    // 本次发回 codex 的收尾事件（同时写入内容日志的 summary）
    let terminal = if let Some(f) = failure {
        // codex 的 SSE 解析器认识 response.failed，会以明确的失败结束本回合
        sse_event(
            "response.failed",
            &json!({
                "type": "response.failed",
                "response": {
                    "id": st.response_id,
                    "object": "response",
                    "created_at": unix_now(),
                    "status": "failed",
                    "model": st.model,
                    "output": [],
                    "error": { "code": f.code, "message": f.message }
                }
            }),
        )
    } else if truncated {
        let mut response = json!({
            "id": st.response_id,
            "object": "response",
            "created_at": unix_now(),
            "status": "incomplete",
            "model": st.model,
            "output": [],
            "error": null,
            "incomplete_details": { "reason": "max_output_tokens" }
        });
        if let Some(usage) = st.usage.as_ref() {
            response["usage"] = usage_to_responses(Some(usage));
        }
        sse_event(
            "response.incomplete",
            &json!({ "type": "response.incomplete", "response": response }),
        )
    } else {
        let mut response = json!({
            "id": st.response_id,
            "object": "response",
            "created_at": unix_now(),
            "status": "completed",
            "model": st.model,
            "output": [],
            "error": null
        });
        if let Some(usage) = st.usage.as_ref() {
            response["usage"] = usage_to_responses(Some(usage));
        }
        sse_event(
            "response.completed",
            &json!({ "type": "response.completed", "response": response }),
        )
    };
    out.push(terminal.clone());

    if let Some(usage) = st.usage.as_ref() {
        log_at(
            log,
            "info",
            "zen_proxy.usage",
            &[("detail", truncate_chars(&usage.to_string(), 300))],
        );
    }

    // 内容诊断日志：收尾事实 + 发回 codex 的收尾事件原文 + 可疑结束标记
    let delta_keys = st.delta_keys.iter().cloned().collect::<Vec<_>>().join(",");
    let flags = suspicious_flags(&SuspiciousFacts {
        finish_reason: st.finish_reason.as_deref(),
        text_chars,
        reasoning_chars,
        call_count,
        failed,
        usage_present: st.usage.is_some(),
        usage_timeout: st.usage_timeout,
        upstream_http_error: false,
        patch_retry: st.patch_failures > 0,
    });
    let suspicious = flags_field(&flags);
    let call = &mut st.trace;
    call.note(
        "finish_reason",
        st.finish_reason.clone().unwrap_or_default(),
    );
    call.note("reasoning_chars", reasoning_chars.to_string());
    call.note("text_chars", text_chars.to_string());
    call.note("stripped_chars", stripped_chars.to_string());
    call.note("call_count", call_count.to_string());
    call.note("failed", failed.to_string());
    call.note("patch_failures", st.patch_failures.to_string());
    if let Some(f) = st.failure.as_ref() {
        call.note("failure_code", f.code.clone());
        call.note("failure_detail", f.detail.clone());
    }
    if let Some(usage) = st.usage.as_ref() {
        call.note("usage_raw", truncate_chars(&usage.to_string(), 300));
    }
    call.note(
        "usage",
        if st.usage.is_some() {
            "present"
        } else {
            "none"
        },
    );
    call.note("delta_keys", delta_keys.clone());
    call.note("suspicious", suspicious.clone());
    call.note("translated_terminal_event", terminal.trim().to_string());
    call.finish();

    log_at(
        log,
        "info",
        "zen_proxy.stream_summary",
        &[
            (
                "finish_reason",
                st.finish_reason.clone().unwrap_or_default(),
            ),
            ("reasoning_chars", reasoning_chars.to_string()),
            ("text_chars", text_chars.to_string()),
            ("stripped_chars", stripped_chars.to_string()),
            ("call_count", call_count.to_string()),
            ("failed", failed.to_string()),
            ("patch_failures", st.patch_failures.to_string()),
            (
                "usage",
                if st.usage.is_some() {
                    "present"
                } else {
                    "none"
                }
                .to_string(),
            ),
            ("delta_keys", delta_keys),
            ("suspicious", suspicious),
        ],
    );
    out
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod test_support;
