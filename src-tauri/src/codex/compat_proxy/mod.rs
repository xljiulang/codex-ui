//! 兼容代理：把 codex 的 OpenAI Responses 请求翻译为 Chat Completions 请求并转发到
//! 任意 OpenAI 兼容上游（默认 OpenCode Zen，可在设置页改地址）；响应反向翻译回
//! Responses 格式（含流式 SSE）。
//!
//! - 上游 base URL 与 User-Agent 硬编码（模块私有，不暴露到前端）。
//! - API Key 不固定：读取入站请求的 `Authorization` 头原样转发。
//! - 仅绑定回环地址，专供 codex-ui 自身使用，无额外鉴权。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
#[cfg(test)]
use axum::Json;
use axum::Router;
use futures_util::stream::{self, StreamExt};
use futures_util::Stream;
use serde_json::{json, Value};
use tokio::task::AbortHandle;

use crate::codex::compat_trace::{TraceCall, TraceSink};
use crate::codex::session_log::SessionLog;

/// 兼容代理默认端口（可在设置页修改）。
pub(crate) const DEFAULT_COMPAT_PROXY_PORT: u16 = 18080;
/// 通用兼容代理没有默认上游；新安装必须显式填写 base URL。
pub(crate) const DEFAULT_COMPAT_BASE_URL: &str = "";
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
/// `compat_proxy.stream_summary` 里记录的 delta 键上限（去重后按字典序取前若干个）。
const DELTA_KEY_LIMIT: usize = 16;
/// 翻译分支的请求体上限：长会话实测已近 1MB，axum 默认 2MB 会撞 413。
const RESPONSES_BODY_LIMIT: usize = 64 * 1024 * 1024;
/// 直通分支判定「协议不支持」时要读的上游错误体上限（错误体很小；超限按普通错误原样回）。
const PASSTHROUGH_ERROR_BODY_LIMIT: usize = 1024 * 1024;
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
/// `compat_proxy.request` 的 `mode_src` 取值：模式来自协议登记表（权威）/ 关键词兜底判据。
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
/// 供兼容代理按入站请求头 `session-id`（= codex 线程 id）直接取用。
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
/// 归一化见 [`normalize_client_session`]（登记表按原始线程 id 存）。
/// 缺失、纯空白或非 ASCII 时返回 None（调用方退回关键词兜底判据）。
fn request_header_thread_id(headers: &HeaderMap) -> Option<String> {
    normalize_client_session(headers.get("session-id")?.to_str().ok()?)
}

/// 本次入站请求的协作模式与来源：先按请求头里的线程 id 查协议登记表（权威，命中即不做任何
/// 关键词扫描），查不到才退回关键词判据（只看**最后一个**协作模式块）。
/// 返回 `(是否计划模式, mode_src)`，`mode_src` 进 `compat_proxy.request` 日志便于排查。
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
pub struct CompatProxyHandle {
    pub port: u16,
    /// 生效中的启动配置（已归一化，用于判断是否需要重启）。
    config: CompatProxyConfig,
    abort: AbortHandle,
}

/// 代理启动配置：端口、上游地址与收尾助推开关。任一项变化都要重启代理，
/// 因此整体作为 [`CompatProxyHandle`] 的比对基准。
#[derive(Debug, Clone)]
pub(crate) struct CompatProxyConfig {
    pub port: u16,
    /// 上游 base_url（已归一化：去空白与末尾 `/`）。
    pub base_url: String,
    /// 「回合收尾约束和助推」开关：是否启用口嗨检测 + 自动续跑（含首轮教学与标签剥离）。
    pub nudge_enabled: bool,
}

impl CompatProxyConfig {
    /// 归一化构造：base_url 去空白与末尾 `/`。
    pub(crate) fn new(port: u16, base_url: &str, nudge_enabled: bool) -> Self {
        Self {
            port,
            base_url: normalize_base_url(base_url),
            nudge_enabled,
        }
    }

    /// 是否与运行中的实例等价（用于「要不要重启代理」的判定）：端口、归一化后的上游地址
    /// 与收尾助推开关全部相同才算未变。
    fn matches(&self, other: &Self) -> bool {
        self.port == other.port
            && self.base_url == other.base_url
            && self.nudge_enabled == other.nudge_enabled
    }
}

/// 兼容代理的诊断日志句柄（复用应用会话日志；None 时不落盘）。
pub(crate) type ZenLog = Option<Arc<SessionLog>>;

impl CompatProxyHandle {
    pub fn stop(&self) {
        self.abort.abort();
    }
}

/// 代理状态（供设置页与 `compat_proxy_status` 命令展示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompatProxyStatus {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
}

impl CompatProxyStatus {
    pub fn stopped() -> Self {
        Self {
            running: false,
            port: DEFAULT_COMPAT_PROXY_PORT,
            error: None,
        }
    }
}

/// 归一化上游 base_url：去首尾空白并去掉末尾所有 `/`；空值保持为空。
/// `https://a/` 与 `https://a`、`https://a/zen/v2/` 与 `https://a/zen/v2` 因此等价，
/// 既影响翻译路由的派生，也影响「上游地址是否变化」的重启判定。
fn normalize_base_url(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
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

/// 在当前 tokio runtime 上启动本地代理；端口被占用时返回 Err。
pub(crate) async fn start(
    config: CompatProxyConfig,
    log: ZenLog,
    trace: TraceSink,
    modes: Arc<ThreadModeRegistry>,
) -> Result<CompatProxyHandle, String> {
    let port = config.port;
    let base_url = config.base_url.clone();
    if base_url.trim().is_empty() {
        return Err("兼容代理未配置上游 base_url，无法启动".to_string());
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    log_at(
        &log,
        "info",
        "compat_proxy.transport",
        &[
            ("transport", "reqwest".to_string()),
            (
                "proxy",
                if proxy_env_configured() { "on" } else { "off" }.to_string(),
            ),
        ],
    );
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
        base_url: base_url.clone(),
        client: reqwest::Client::new(),
        log,
        trace,
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        nudge_enabled: config.nudge_enabled,
        protocols: protocol_cache(),
        modes,
    });
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(CompatProxyHandle {
        port,
        config,
        abort: task.abort_handle(),
    })
}

fn proxy_env_configured() -> bool {
    [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    })
}

/// 按需启停代理；端口、上游地址或行为开关变化时重启。
pub(crate) async fn apply(
    handle: &mut Option<CompatProxyHandle>,
    enabled: bool,
    config: CompatProxyConfig,
    log: ZenLog,
    trace: TraceSink,
    modes: Arc<ThreadModeRegistry>,
) -> CompatProxyStatus {
    let port = config.port;
    // 端口、上游地址与收尾助推开关都没变（含 `https://a/` 与 `https://a` 这类等价写法）
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
                CompatProxyStatus {
                    running: true,
                    port,
                    error: None,
                }
            }
            Err(e) => CompatProxyStatus {
                running: false,
                port,
                error: Some(e),
            },
        }
    } else if enabled {
        CompatProxyStatus {
            running: true,
            port,
            error: None,
        }
    } else {
        CompatProxyStatus::stopped()
    }
}

// ---------------------------------------------------------------------------
// HTTP 入口
// ---------------------------------------------------------------------------

/// 代理运行期共享状态。
#[derive(Clone)]
struct ProxyState {
    base_url: String,
    /// 复用连接池的通用上游 HTTP 客户端。
    client: reqwest::Client,
    log: ZenLog,
    /// 内容诊断日志（常开）：每次上游尝试的请求体/响应原文/收尾事件。
    trace: TraceSink,
    /// 上游是否要求历史里带 `tool_calls` 的 assistant 消息回传 `reasoning_content`
    /// （DeepSeek 思考模式）。默认关闭，收到明确报错后学习并粘滞到本代理实例结束。
    requires_reasoning_rc: Arc<AtomicBool>,
    /// 「回合收尾约束和助推」开关：是否启用口嗨检测 + 自动续跑（含首轮教学与标签剥离）。
    nudge_enabled: bool,
    /// 上游模型协议记忆（key = model id）：默认按 [`ModelProtocol::Chat`] 翻译，上游以
    /// 「协议不支持」拒绝后改判 [`ModelProtocol::Responses`] 直通并**粘滞**到本代理实例结束
    /// （重启后重学一次，最多多打一次不会计费的 400 往返），见 [`handle_responses_auto`]。
    protocols: ProtocolCache,
    /// 协作模式登记表（key = codex 线程 id）：由 app-server 侧登记，代理解析模式时优先查它。
    modes: Arc<ThreadModeRegistry>,
}

/// 上游支持的请求协议：`chat` 走 Responses→Chat 翻译，`responses` 按原协议直通。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelProtocol {
    Chat,
    Responses,
}

/// 模型协议记忆表（key = model id）。
type ProtocolCache = Arc<Mutex<HashMap<String, ModelProtocol>>>;

/// 新建空的模型协议记忆表（代理实例级；测试与 [`start`] 共用）。
fn protocol_cache() -> ProtocolCache {
    Arc::new(Mutex::new(HashMap::new()))
}

/// 日志口径的协议名。
fn protocol_label(protocol: ModelProtocol) -> &'static str {
    match protocol {
        ModelProtocol::Chat => "chat",
        ModelProtocol::Responses => "responses",
    }
}

/// 上游「这个模型不支持当前协议」的错误标记（两个方向都返回这条 4xx，见
/// `ModelProtocolUnsupported`）：命中即改判另一条协议重发一次。
const PROTOCOL_UNSUPPORTED_NEEDLES: [&str; 2] =
    ["modelprotocolunsupported", "does not support this protocol"];

/// 上游错误体是否在说「本模型不支持这个协议」。
fn is_protocol_unsupported(text: &str) -> bool {
    mentions_field(text, &PROTOCOL_UNSUPPORTED_NEEDLES)
}

/// 取本次入站请求该用的上游协议：默认 chat，学到过就按记忆；返回值第二项是日志口径的来源。
fn resolve_protocol(state: &ProxyState, model: Option<&str>) -> (ModelProtocol, &'static str) {
    let Some(model) = model.filter(|model| !model.trim().is_empty()) else {
        return (ModelProtocol::Chat, "default");
    };
    let learned = state
        .protocols
        .lock()
        .ok()
        .and_then(|protocols| protocols.get(model).copied());
    match learned {
        Some(protocol) => (protocol, "learned"),
        None => (ModelProtocol::Chat, "default"),
    }
}

/// 记住某模型的上游协议；返回是否真的发生了变化（没有 model id 时不记）。
fn learn_protocol(state: &ProxyState, model: Option<&str>, protocol: ModelProtocol) -> bool {
    let Some(model) = model.filter(|model| !model.trim().is_empty()) else {
        return false;
    };
    match state.protocols.lock() {
        Ok(mut protocols) => protocols.insert(model.to_string(), protocol) != Some(protocol),
        Err(_) => false,
    }
}

/// 写一条 compat_proxy 诊断日志；句柄为 None 或写盘失败时静默忽略。
fn log_at(log: &Option<Arc<SessionLog>>, level: &str, event: &str, kv: &[(&str, String)]) {
    let Some(log) = log else { return };
    let kv: Vec<(String, String)> = kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    log.write(level, None, event, &kv);
}

/// 归一化入站会话 id：trim 后剥掉可能存在的 `ses_` 前缀（codex 上报的是裸线程 id），
/// 结果为空时返回 None（调用方视为「客户端没给会话 id」）。
fn normalize_client_session(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let bare = trimmed.strip_prefix("ses_").unwrap_or(trimmed).trim();
    (!bare.is_empty()).then(|| bare.to_string())
}

/// `compat_proxy.request` 的诊断列（纯函数，便于单测）：体积（提示词 / 历史字符数）
/// 用于判断是否逼近上游窗口；`reasoning_effort` 记录 codex 本次实际请求的推理强度
/// （`-` 表示没请求推理），排查「思考过程为空」时先看这一格；
/// `mode` 是口嗨检测识别出的协作模式（`plan`/`default`）——排查「默认模式被注入计划提醒」
/// 这类问题时先看这一格；`mode_src` 说明它的来源（`registry` = 协议登记表，
/// `heuristic` = 关键词兜底判据，见 [`resolve_nudge_mode`]）；
/// `patch_failures` 是本次历史里已失败的补丁调用条数（>0 说明模型正在补丁上打转，
/// 代理已对这些工具结果追加格式纠错提示）；`call_id` 与内容日志
/// （`logs/compat/<时间>-<call_id>-a<n>.*`）一一对应，便于两者对照。
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
        return handle_responses_auto(&state, &headers, &uri, body).await;
    }
    // 客户端把 `/responses` 打到了别的路径：本地 provider 的 base_url 路径与上游不一致。
    if uri.path().ends_with("/responses") {
        log_at(
            &state.log,
            "warn",
            "compat_proxy.path_unmatched",
            &[
                ("method", method.to_string()),
                ("path", uri.path().to_string()),
                ("expected", expected),
            ],
        );
    }
    forward_passthrough(&state, method, &uri, &headers, body).await
}

/// 一条协议路径的处理结果：正常响应，或「上游说本模型不支持该协议」——后者供
/// [`handle_responses_auto`] 改判另一条协议重发一次，不直接回给客户端。
enum ProtocolOutcome {
    Response(Response),
    Unsupported { detail: String },
}

/// Responses 入口：**自动判定上游协议**。
///
/// 默认按 chat 翻译（历史行为）；上游以「协议不支持」拒绝时改判 Responses 直通并**只重发一次**
/// 另一条路径（判据见 [`is_protocol_unsupported`]），学到的结论按 model id 粘滞到本代理实例
/// 结束（重启后重学，最多多一次不会计费的上游 400）。其它 4xx（鉴权/限流/地区限制等）
/// 一律不改判、不写记忆。
///
/// 读取或解析请求体失败时返回明确错误。
async fn handle_responses_auto(
    state: &ProxyState,
    headers: &HeaderMap,
    uri: &Uri,
    body: Body,
) -> Response {
    let bytes = match axum::body::to_bytes(body, RESPONSES_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(e) => {
            return error_json(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("请求体过大或读取失败：{e}"),
            );
        }
    };
    let req: Value = match serde_json::from_slice(&bytes) {
        Ok(req) => req,
        Err(e) => {
            return error_json(StatusCode::BAD_REQUEST, format!("请求体不是合法 JSON：{e}"));
        }
    };
    let model = req.get("model").and_then(Value::as_str).map(str::to_string);
    let (protocol, reason) = resolve_protocol(state, model.as_deref());
    log_at(
        &state.log,
        "info",
        "compat_proxy.protocol",
        &[
            ("model", model.clone().unwrap_or_default()),
            ("protocol", protocol_label(protocol).to_string()),
            ("reason", reason.to_string()),
        ],
    );
    match protocol {
        ModelProtocol::Chat => match handle_translated_responses(state, headers, &req).await {
            ProtocolOutcome::Response(response) => response,
            ProtocolOutcome::Unsupported { detail } => {
                note_protocol_switch(state, model.as_deref(), ModelProtocol::Responses, &detail);
                settle_protocol_outcome(
                    handle_responses_passthrough(state, headers, uri, &req).await,
                )
            }
        },
        ModelProtocol::Responses => {
            match handle_responses_passthrough(state, headers, uri, &req).await {
                ProtocolOutcome::Response(response) => response,
                ProtocolOutcome::Unsupported { detail } => {
                    note_protocol_switch(state, model.as_deref(), ModelProtocol::Chat, &detail);
                    settle_protocol_outcome(handle_translated_responses(state, headers, &req).await)
                }
            }
        }
    }
}

/// 两条协议都被上游拒绝（罕见：该模型的协议既不是 chat 也不是 responses）时收尾：
/// 原样回最后一条上游错误，不再来回切换。
fn settle_protocol_outcome(outcome: ProtocolOutcome) -> Response {
    match outcome {
        ProtocolOutcome::Response(response) => response,
        ProtocolOutcome::Unsupported { detail } => Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(detail))
            .unwrap_or_else(|_| error_json(StatusCode::BAD_REQUEST, "上游不支持该模型协议".into())),
    }
}

/// 改判某模型的上游协议并记一条 warn（结论粘滞到本代理实例结束）。
fn note_protocol_switch(
    state: &ProxyState,
    model: Option<&str>,
    protocol: ModelProtocol,
    detail: &str,
) {
    let changed = learn_protocol(state, model, protocol);
    log_at(
        &state.log,
        "warn",
        "compat_proxy.protocol_switched",
        &[
            ("model", model.unwrap_or_default().to_string()),
            ("to", protocol_label(protocol).to_string()),
            ("changed", changed.to_string()),
            ("detail", detail.to_string()),
        ],
    );
}

/// 翻译路径：把 Responses 请求翻译成 chat/completions 发给上游（历史实现，行为不变）。
async fn handle_translated_responses(
    state: &ProxyState,
    headers: &HeaderMap,
    req: &Value,
) -> ProtocolOutcome {
    let mut req = req.clone();
    let want_stream = req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    // 本次入站请求的稳定标识：贯穿内容日志文件名与 session 日志行。
    let call_id = gen_id("req");
    // 协作模式先按协议登记表解析（查不到才退回关键词），日志里连同来源一起记下
    let (plan_mode, mode_src) = resolve_nudge_mode(state, headers, &req);
    let mode = if plan_mode { "plan" } else { "default" };
    // 首轮教学：把收尾契约追加到 instructions 尾部（必须在模式解析之后——契约文本里不含
    // `<collaboration_mode>` 块，模式判据与 mode_src 因此完全不受影响）
    // 「回合收尾约束和助推」关闭时整条链路（首轮教学、终局判定、续跑、标签剥离）都不参与
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
    log_at(&state.log, "info", "compat_proxy.request", &kv);

    let started = Instant::now();
    let base_url = state.base_url.clone();
    match forward(&req, headers, want_stream, state, &call_id).await {
        Ok(forwarded) => {
            let status = forwarded.status;
            log_at(
                &state.log,
                "info",
                "compat_proxy.forward",
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
                ],
            );
            if let Some(change) = forwarded.reasoning_rc_change.as_ref() {
                log_at(
                    &state.log,
                    "warn",
                    if change.enabled {
                        "compat_proxy.reasoning_content_enabled"
                    } else {
                        "compat_proxy.reasoning_content_disabled"
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
                    "compat_proxy.history_repaired",
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
                    "compat_proxy.optional_fields_dropped",
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
                    // 上游说「不支持这个协议」→ 不当作终局错误回给客户端，改判另一条协议重发
                    if !status.is_success() && is_protocol_unsupported(&text) {
                        return protocol_unsupported_outcome(status, &text, forwarded.trace);
                    }
                    ProtocolOutcome::Response(proxy_error_text(
                        status,
                        &text,
                        &state.log,
                        forwarded.trace,
                    ))
                }
                ForwardPayload::Live(resp) => {
                    if !status.is_success() {
                        let text = resp.text().await.unwrap_or_default();
                        if is_protocol_unsupported(&text) {
                            return protocol_unsupported_outcome(status, &text, forwarded.trace);
                        }
                        return ProtocolOutcome::Response(proxy_error_text(
                            status,
                            &text,
                            &state.log,
                            forwarded.trace,
                        ));
                    }
                    if want_stream {
                        ProtocolOutcome::Response(
                            proxy_stream_response(
                                state.clone(),
                                headers.clone(),
                                call_id.clone(),
                                req,
                                resp,
                                forwarded.trace,
                            )
                            .await,
                        )
                    } else {
                        ProtocolOutcome::Response(
                            proxy_json_response(
                                req,
                                resp,
                                &state.log,
                                state.nudge_enabled,
                                forwarded.trace,
                            )
                            .await,
                        )
                    }
                }
            }
        }
        Err((status, msg)) => {
            log_at(
                &state.log,
                "warn",
                "compat_proxy.forward_error",
                &[("url", base_url), ("error", msg.clone())],
            );
            ProtocolOutcome::Response(error_json(status, msg))
        }
    }
}

/// 上游以「协议不支持」拒绝时：把原文写进内容日志（随后会换协议重发），并返回可换协议的信号。
fn protocol_unsupported_outcome(
    status: StatusCode,
    text: &str,
    mut call: TraceCall,
) -> ProtocolOutcome {
    call.note("upstream_status", status.as_u16().to_string());
    call.write_response_text("response.txt", text);
    call.note("protocol_unsupported", "true");
    call.finish();
    ProtocolOutcome::Unsupported {
        detail: text.chars().take(200).collect(),
    }
}

/// 直通路径的响应侧回译状态：把「被 [`normalize_responses_tools`] 转成 function 的 custom 工具」
/// 调用还原成 codex 认的 `custom_tool_call`（`added`/`done` 两处必须一致，否则 codex 侧 item
/// 对不上）。只有本次请求确实含 custom 工具时才启用，其余请求完全逐字透传。
struct ResponsesToolRewrite {
    /// 本次请求里由 `custom` 转成 `function` 的工具名。
    customs: HashSet<String>,
    /// 挂起中的调用：等收敛（item.done）后再决定整组下发还是抑制。
    pending: HashMap<String, PendingCall>,
    /// 被判定畸形、整体抑制的 `item_id`（收尾时从 `output[]` 里剔除）。
    malformed_ids: HashSet<String>,
    /// 本次流里是否出现过畸形调用（收尾改发 `response.failed`）。
    malformed: bool,
}

/// 一个挂起中的 custom 调用。
struct PendingCall {
    /// 已改写成 `custom_tool_call` 的 `response.output_item.added` 帧原文。
    added: String,
    /// 该调用的 `output_index`（合成增量/完成事件要用）。
    out_index: Option<u64>,
    /// 该调用的 `call_id`（合成 item 用）。
    call_id: String,
    /// 工具名。
    name: String,
    /// 上游累积的参数文本（来自 `.done` 事件或 item 自身）。
    arguments: String,
}

impl ResponsesToolRewrite {
    /// 由请求侧转换出的工具名集合构造；空集合表示本次不需要任何改写。
    fn new(customs: Vec<String>) -> Self {
        Self {
            customs: customs.into_iter().collect(),
            pending: HashMap::new(),
            malformed_ids: HashSet::new(),
            malformed: false,
        }
    }

    fn enabled(&self) -> bool {
        !self.customs.is_empty()
    }

    /// 收敛一个挂起调用：返回要下发给 codex 的帧文本（`added` + 一次性 `input.delta` +
    /// `input.done` + 改写后的 `item.done`）。参数为空或不可用时记畸形、返回空串（整组抑制）。
    ///
    /// `item_done` 为 `Some` 表示由上游的 `response.output_item.done` 触发（可用它兜底取参数）。
    fn finish_call(&mut self, item_id: &str, item_done: Option<&Value>, log: &ZenLog) -> String {
        let Some(mut call) = self.pending.remove(item_id) else {
            return String::new();
        };
        if call.arguments.is_empty() {
            if let Some(arguments) = item_done
                .and_then(|item| item.get("arguments"))
                .and_then(Value::as_str)
            {
                call.arguments = arguments.to_string();
            }
        }
        let input = custom_tool_input(&call.arguments);
        if input.trim().is_empty() {
            self.malformed = true;
            self.malformed_ids.insert(item_id.to_string());
            log_at(
                log,
                "warn",
                "compat_proxy.malformed_tool_call",
                &[
                    ("name", call.name.clone()),
                    ("call_id", call.call_id.clone()),
                    ("args_len", call.arguments.chars().count().to_string()),
                    ("preview", truncate_chars(call.arguments.trim(), 200)),
                    (
                        "reason",
                        "自定义工具（已按函数形态声明）返回的调用内容为空，已阻止该调用进入会话"
                            .to_string(),
                    ),
                ],
            );
            return String::new();
        }
        let mut out = call.added;
        out.push_str(&sse_event(
            "response.custom_tool_call_input.delta",
            &json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": item_id,
                "output_index": call.out_index,
                "delta": input
            }),
        ));
        out.push_str(&sse_event(
            "response.custom_tool_call_input.done",
            &json!({
                "type": "response.custom_tool_call_input.done",
                "item_id": item_id,
                "output_index": call.out_index,
                "input": input
            }),
        ));
        out.push_str(&sse_event(
            "response.output_item.done",
            &json!({
                "type": "response.output_item.done",
                "output_index": call.out_index,
                "item": {
                    "id": item_id,
                    "type": "custom_tool_call",
                    "status": "completed",
                    "call_id": call.call_id,
                    "name": call.name,
                    "input": input
                }
            }),
        ));
        out
    }
}

/// SSE 帧边界：返回「含分隔空行」的字节长度（`\n\n` 或 `\r\n\r\n`）；未收全一帧时返回 None。
fn sse_frame_end(buf: &[u8]) -> Option<usize> {
    for index in 0..buf.len() {
        if buf[index] == b'\n' && buf.get(index + 1) == Some(&b'\n') {
            return Some(index + 2);
        }
        if buf[index..].starts_with(b"\r\n\r\n") {
            return Some(index + 4);
        }
    }
    None
}

/// 从一帧 SSE 文本里取 `(event 名, data 原文)`；缺 data 行时返回 None。
fn parse_sse_frame(text: &str) -> Option<(&str, &str)> {
    let mut event = "";
    let mut data: Option<&str> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = rest.trim();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data = Some(rest.trim());
        }
    }
    data.map(|data| (event, data))
}

/// 改写一帧上游 Responses SSE。返回要发给 codex 的字节：
/// - 与 custom 回译无关（或不是 SSE/不是 JSON）的帧**逐字**返回；
/// - 挂起中（等收敛）或被抑制的帧返回空；
/// - 收敛时可以返回多帧拼接。
fn rewrite_responses_frame(frame: &[u8], st: &mut ResponsesToolRewrite, log: &ZenLog) -> Vec<u8> {
    if !st.enabled() {
        return frame.to_vec();
    }
    let Ok(text) = std::str::from_utf8(frame) else {
        return frame.to_vec();
    };
    let Some((event, data_text)) = parse_sse_frame(text) else {
        return frame.to_vec();
    };
    let Ok(data) = serde_json::from_str::<Value>(data_text) else {
        return frame.to_vec();
    };
    let kind = data
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(event)
        .to_string();
    match kind.as_str() {
        "response.output_item.added" => {
            let Some(item) = data.get("item") else {
                return frame.to_vec();
            };
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            if item.get("type").and_then(Value::as_str) != Some("function_call")
                || !st.customs.contains(name)
            {
                return frame.to_vec();
            }
            let Some(item_id) = item.get("id").and_then(Value::as_str) else {
                return frame.to_vec();
            };
            let mut rewritten = data.clone();
            let mut input = String::new();
            if let Some(obj) = rewritten.get_mut("item").and_then(Value::as_object_mut) {
                if let Some(arguments) = obj.get("arguments").and_then(Value::as_str) {
                    input = custom_tool_input(arguments);
                }
                obj.insert("type".to_string(), json!("custom_tool_call"));
                obj.remove("arguments");
                obj.insert("input".to_string(), json!(input));
            }
            st.pending.insert(
                item_id.to_string(),
                PendingCall {
                    added: sse_event("response.output_item.added", &rewritten),
                    out_index: rewritten.get("output_index").and_then(Value::as_u64),
                    call_id: item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    name: name.to_string(),
                    arguments: String::new(),
                },
            );
            Vec::new()
        }
        "response.function_call_arguments.delta" => {
            let Some(item_id) = data.get("item_id").and_then(Value::as_str) else {
                return frame.to_vec();
            };
            if st.pending.contains_key(item_id) {
                // 挂起：增量在收敛时按解码后的完整文本一次性下发，避免转义碎片
                Vec::new()
            } else {
                frame.to_vec()
            }
        }
        "response.function_call_arguments.done" => {
            let Some(item_id) = data.get("item_id").and_then(Value::as_str) else {
                return frame.to_vec();
            };
            match st.pending.get_mut(item_id) {
                Some(call) => {
                    if let Some(arguments) = data.get("arguments").and_then(Value::as_str) {
                        call.arguments = arguments.to_string();
                    }
                    Vec::new()
                }
                None => frame.to_vec(),
            }
        }
        "response.output_item.done" => {
            let Some(item) = data.get("item") else {
                return frame.to_vec();
            };
            let Some(item_id) = item.get("id").and_then(Value::as_str) else {
                return frame.to_vec();
            };
            if !st.pending.contains_key(item_id) {
                return frame.to_vec();
            }
            st.finish_call(item_id, Some(item), log).into_bytes()
        }
        "response.completed" | "response.incomplete" | "response.failed" => {
            // 上游提前终止：把还挂着的调用按现状收尾（空参数按畸形抑制）
            let pending: Vec<String> = st.pending.keys().cloned().collect();
            let mut out = String::new();
            for item_id in pending {
                out.push_str(&st.finish_call(&item_id, None, log));
            }
            let mut rewritten = data.clone();
            if let Some(items) = rewritten
                .get_mut("response")
                .and_then(|response| response.get_mut("output"))
                .and_then(Value::as_array_mut)
            {
                for item in items.iter_mut() {
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if item.get("type").and_then(Value::as_str) != Some("function_call")
                        || !st.customs.contains(&name)
                    {
                        continue;
                    }
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let input = custom_tool_input(&arguments);
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("type".to_string(), json!("custom_tool_call"));
                        obj.remove("arguments");
                        obj.insert("input".to_string(), json!(input));
                    }
                }
                let malformed_ids = st.malformed_ids.clone();
                items.retain(|item| {
                    !item
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| malformed_ids.contains(id))
                });
            }
            if st.malformed {
                // 畸形调用：整轮判失败（与翻译路径口径一致），不让半截调用进会话
                let response = rewritten
                    .get("response")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let failed = json!({
                    "type": "response.failed",
                    "response": {
                        "id": response.get("id").cloned().unwrap_or(Value::Null),
                        "object": "response",
                        "created_at": unix_now(),
                        "status": "failed",
                        "model": response.get("model").cloned().unwrap_or(Value::Null),
                        "output": [],
                        "error": {
                            "code": CODE_MALFORMED_TOOL_CALL,
                            "message": "上游模型返回的工具调用参数不可用，已阻止该调用进入会话；请重试本轮"
                        }
                    }
                });
                out.push_str(&sse_event("response.failed", &failed));
                return out.into_bytes();
            }
            out.push_str(&sse_event(&kind, &rewritten));
            out.into_bytes()
        }
        _ => frame.to_vec(),
    }
}

/// 把上游 SSE 字节流按帧改写后下发（只在本次请求含 custom 工具时使用）。
fn rewrite_responses_stream(
    stream: impl Stream<Item = Result<axum::body::Bytes, reqwest::Error>> + Send + 'static,
    state: ResponsesToolRewrite,
    log: ZenLog,
) -> impl Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send + 'static {
    /// 上游字节流（装箱固定，`unfold` 的 carry 需要可移动）
    type Inner = std::pin::Pin<
        Box<dyn Stream<Item = Result<axum::body::Bytes, reqwest::Error>> + Send + 'static>,
    >;
    struct Carry {
        inner: Inner,
        buf: Vec<u8>,
        state: ResponsesToolRewrite,
        log: ZenLog,
        ended: bool,
    }
    stream::unfold(
        Carry {
            inner: Box::pin(stream),
            buf: Vec::new(),
            state,
            log,
            ended: false,
        },
        |mut carry| async move {
            loop {
                if let Some(end) = sse_frame_end(&carry.buf) {
                    let frame: Vec<u8> = carry.buf.drain(..end).collect();
                    let out = rewrite_responses_frame(&frame, &mut carry.state, &carry.log);
                    if out.is_empty() {
                        continue;
                    }
                    return Some((Ok(axum::body::Bytes::from(out)), carry));
                }
                if carry.ended {
                    if carry.buf.is_empty() {
                        return None;
                    }
                    let tail: Vec<u8> = std::mem::take(&mut carry.buf);
                    let out = rewrite_responses_frame(&tail, &mut carry.state, &carry.log);
                    if out.is_empty() {
                        return None;
                    }
                    return Some((Ok(axum::body::Bytes::from(out)), carry));
                }
                match carry.inner.next().await {
                    Some(Ok(chunk)) => carry.buf.extend_from_slice(&chunk),
                    Some(Err(e)) => {
                        return Some((
                            Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
                            carry,
                        ));
                    }
                    None => carry.ended = true,
                }
            }
        },
    )
}

/// Responses 直通路径：仅做通用协议兼容后把请求体发给上游**同一路径**，上游 SSE 回传。
///
/// 与翻译路径的差异（有意为之）：不注入收尾契约、不做口嗨续跑/标签剥离/历史净化/
/// 畸形工具调用保护/usage 明细映射/命名空间展开 —— 这条路径只服务「上游只认 Responses 协议」
/// 的模型，codex 要的就是上游原生 Responses 流（实测 codex 的 `namespace` 工具与
/// `store`/`include`/`reasoning`/`text` 字段上游都原样接受，不需要任何改写）。
async fn handle_responses_passthrough(
    state: &ProxyState,
    headers: &HeaderMap,
    uri: &Uri,
    req: &Value,
) -> ProtocolOutcome {
    // 「回合收尾约束和助推」在这条路径上不生效（整条口嗨判定/续跑/标签剥离都在翻译管线里）：
    // 开着时记一条，避免"看着开了实际没生效"
    if state.nudge_enabled {
        log_at(
            &state.log,
            "info",
            "compat_proxy.nudge_skipped",
            &[
                ("原因", "responses_protocol".to_string()),
                ("轮次", "1".to_string()),
                ("说明", NUDGE_SKIP_NOTE_RESPONSES_PROTOCOL.to_string()),
            ],
        );
    }
    let mut body = req.clone();
    // 上游 `/responses` 拒收 `custom` 工具（codex 的 apply_patch 只能这样声明）：
    // 先归一化成同名 function，响应侧再回译成 `custom_tool_call`（见 ResponsesToolRewrite）。
    // 这是 Responses 协议兼容处理，与上游身份或客户端版本无关。
    let custom_tools = normalize_responses_tools(&mut body);
    if !custom_tools.is_empty() {
        log_at(
            &state.log,
            "info",
            "compat_proxy.responses_custom_tools",
            &[
                ("count", custom_tools.len().to_string()),
                ("names", custom_tools.join(",")),
            ],
        );
    }
    // 历史里的思维链 item：翻译路径合成的那种「有 id、没 encrypted_content」上游会按 id 查而拒，
    // 统一把这类 item 的 id 剥掉（只留 summary），属于通用协议兼容处理。
    let stripped = sanitize_responses_reasoning_items(&mut body);
    if stripped > 0 {
        log_at(
            &state.log,
            "info",
            "compat_proxy.reasoning_ids_stripped",
            &[("count", stripped.to_string())],
        );
    }
    let bytes = match serde_json::to_vec(&body) {
        Ok(bytes) => bytes,
        Err(e) => {
            return ProtocolOutcome::Response(error_json(
                StatusCode::BAD_REQUEST,
                format!("请求体序列化失败：{e}"),
            ));
        }
    };
    let response = if custom_tools.is_empty() {
        forward_passthrough(state, Method::POST, uri, headers, Body::from(bytes)).await
    } else {
        forward_passthrough_rewriting(
            state,
            uri,
            headers,
            bytes,
            ResponsesToolRewrite::new(custom_tools),
        )
        .await
    };
    if response.status().is_success() {
        return ProtocolOutcome::Response(response);
    }
    // 非 2xx：错误体很小，读出来判断是否「协议不支持」；不是的话按原状态码与原文回给客户端
    let status = response.status();
    let text = match axum::body::to_bytes(response.into_body(), PASSTHROUGH_ERROR_BODY_LIMIT).await
    {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(e) => {
            return ProtocolOutcome::Response(error_json(
                StatusCode::BAD_GATEWAY,
                format!("读取上游错误响应失败：{e}"),
            ));
        }
    };
    if is_protocol_unsupported(&text) {
        return ProtocolOutcome::Unsupported {
            detail: text.chars().take(200).collect(),
        };
    }
    ProtocolOutcome::Response(
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(text))
            .unwrap_or_else(|_| error_json(StatusCode::BAD_GATEWAY, "上游错误响应构造失败".into())),
    )
}

/// 与 [`forward_passthrough`] 相同，但 2xx 响应体交给 [`rewrite_responses_stream`] 改写
/// （把 custom 工具调用的 `function_call` 回译成 `custom_tool_call`）；非 2xx 原样透传，
/// 由调用方读错误体判断协议/门禁。
async fn forward_passthrough_rewriting(
    state: &ProxyState,
    uri: &Uri,
    headers: &HeaderMap,
    body: Vec<u8>,
    rewrite: ResponsesToolRewrite,
) -> Response {
    let url = match passthrough_url(&state.base_url, uri) {
        Ok(url) => url,
        Err(e) => {
            log_passthrough(state, &Method::POST, uri.path(), "error");
            return error_json(StatusCode::BAD_GATEWAY, e);
        }
    };
    let rq = state
        .client
        .request(to_reqwest_method(&Method::POST), url)
        .headers(forwarded_request_headers(headers))
        .body(body);
    match rq.send().await {
        Ok(resp) => {
            let status = resp.status();
            log_passthrough(
                state,
                &Method::POST,
                uri.path(),
                &status.as_u16().to_string(),
            );
            if !status.is_success() {
                return passthrough_response(resp).await;
            }
            let out_headers = forwarded_response_headers(resp.headers());
            let mut builder = Response::builder().status(status);
            for (name, value) in out_headers.iter() {
                builder = builder.header(name.clone(), value.clone());
            }
            let stream = rewrite_responses_stream(resp.bytes_stream(), rewrite, state.log.clone());
            builder.body(Body::from_stream(stream)).unwrap_or_else(|_| {
                error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into())
            })
        }
        Err(e) => {
            log_passthrough(state, &Method::POST, uri.path(), "error");
            error_json(StatusCode::BAD_GATEWAY, format!("上游请求失败：{e}"))
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
    let mut rq = state
        .client
        .request(to_reqwest_method(&method), url)
        .headers(forwarded_request_headers(headers));
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

/// 透传日志：2xx 记 info、其余记 warn，配合 `compat_proxy.path_unmatched` 判断请求走了哪条分支。
fn log_passthrough(state: &ProxyState, method: &Method, path: &str, status: &str) {
    let level = if status.starts_with('2') {
        "info"
    } else {
        "warn"
    };
    log_at(
        &state.log,
        level,
        "compat_proxy.passthrough",
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
        .map_err(|e| format!("兼容代理转发地址无效: {e}"))?;
    url.set_path(uri.path());
    url.set_query(uri.query());
    url.set_fragment(None);
    Ok(url)
}

/// 构造转发给上游的请求头：保留入站头并剔除逐跳头。
fn forwarded_request_headers(headers: &HeaderMap) -> HeaderMap {
    let mut out = headers.clone();
    for name in HOP_BY_HOP_HEADERS {
        out.remove(name);
    }
    out.remove(header::HOST);
    out.remove(header::CONTENT_LENGTH);
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

/// Responses 直通路径的**工具归一化**：把 `type:"custom"`（codex 的 `apply_patch` 只能这样声明）
/// 改写成同名 `function`（单个 `input` 字符串参数 + 补丁语法说明），返回被转换的名字集合。
///
/// 为什么必须转：上游 `/responses` 明确拒收 `custom` 工具（实测 `400 invalid_request_error:
/// \`custom\` tools are not supported on this endpoint`），而 codex 的模型目录字段
/// `apply_patch_tool_type` **只有 `freeform` 一个合法取值**（`codex debug models` 实测报告
/// `unknown variant 'function', expected 'freeform'`），所以 codex 永远会声明成 custom ——
/// 这层转换只能由代理做。
///
/// 契约：保持原有顺序；同名时以客户端已声明的非 custom 工具为准（不覆盖）；重复的 custom 只转第一个；
/// 没有 `name` 的 custom 条目直接丢弃（无法作为函数声明）。`tools` 缺失或不是数组时原样返回空集合。
fn normalize_responses_tools(body: &mut Value) -> Vec<String> {
    let Some(tools) = body.get("tools").and_then(Value::as_array).cloned() else {
        return Vec::new();
    };
    // 已被非 custom 工具占用的名字（含 function / namespace 等），custom 版本不再重复声明
    let mut seen: HashSet<String> = tools
        .iter()
        .filter(|tool| tool.get("type").and_then(Value::as_str) != Some("custom"))
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    let mut out: Vec<Value> = Vec::with_capacity(tools.len());
    let mut converted: Vec<String> = Vec::new();
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("custom") {
            out.push(tool);
            continue;
        }
        let Some(name) = tool.get("name").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let raw_desc = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push(json!({
            "type": "function",
            "name": name,
            "description": custom_tool_description(raw_desc, &name),
            "parameters": custom_tool_parameters(),
            "strict": false
        }));
        converted.push(name);
    }
    body["tools"] = Value::Array(out);
    converted
}

/// 直通路径的**历史净化**：把 `input[]` 里「没有 `encrypted_content` 的 `reasoning` item」的
/// `id` 字段删掉（`summary` 等其余字段逐字保留），返回被剥的条数。
///
/// 为什么必须剥：翻译路径产出的思维链 item 是 `gen_id("rs")` 合成的 id + 只有 `summary`
/// （chat 侧没有可回传的加密思维链），而上游 `/responses` 对这类 item **按 id 查**——
/// 带上查不到的 id 会被拒（`Referenced reasoning item … was not found or has expired.`）。
/// 2026-10-15 实测：同一份历史里，带 `encrypted_content` 的 item 无论 id 是否存在都 200、
/// 只有 summary + 未知 id 是 400、只有 summary 且无 id 是 200。codex 默认
/// `disable_response_storage = true`（`store:false`），id 本来就不可解析，因此
/// 「剥 id 留 summary」是无损的最小修法。其它 item 的合成 id 无关痛痒
/// （`message` / `custom_tool_call` 实测 200，`function_call` 只校验调用与结果的配对）。
fn sanitize_responses_reasoning_items(body: &mut Value) -> usize {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut stripped = 0usize;
    for item in input.iter_mut() {
        if item.get("type").and_then(Value::as_str) != Some("reasoning") {
            continue;
        }
        let has_encrypted = item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .is_some_and(|content| !content.trim().is_empty());
        if has_encrypted {
            continue;
        }
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        if obj.remove("id").is_some() {
            stripped += 1;
        }
    }
    stripped
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
fn note_request_meta(
    call: &mut TraceCall,
    url: &str,
    headers: &HeaderMap,
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

/// 翻译请求并转发到通用 Chat Completions 上游（含历史净化与可选字段降级重试）。
/// `state` 提供上游地址、HTTP 客户端与代理级状态。
/// `call_id` 用于把本次入站请求的多次尝试（内容日志）关联到同一条 session 日志。
async fn forward(
    req: &Value,
    headers: &HeaderMap,
    want_stream: bool,
    state: &ProxyState,
    call_id: &str,
) -> Result<Forwarded, (StatusCode, String)> {
    let url = format!("{}/chat/completions", state.base_url.trim_end_matches('/'));
    let optional = optional_fields(req, want_stream);
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
        for field in optional.iter() {
            if dropped.contains(&field.key) {
                continue;
            }
            body[field.key] = field.value.clone();
        }
        // 内容诊断日志：先落盘本次尝试实际发出的请求体与关键元信息（不含 API Key）
        let mut call = state.trace.begin(call_id, attempt_no);
        note_request_meta(&mut call, &url, headers, &body, &dropped, reasoning_rc);
        call.write_request_json(&body);
        let resp = build_chat_request(&url, state, headers, &body)
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
                "compat_proxy.forward_attempt",
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

/// 构造发往上游的 chat/completions 请求：透传入站 Authorization，不生成客户端身份头。
fn build_chat_request(
    url: &str,
    state: &ProxyState,
    headers: &HeaderMap,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut rq = state.client.post(url).json(body);
    for (name, value) in forwarded_request_headers(headers).iter() {
        rq = rq.header(name.clone(), value.clone());
    }
    rq
}

/// 非流式：把 Chat Completions JSON 翻译为 Responses 对象。
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
            "compat_proxy.response_parse_error",
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
                "compat_proxy.malformed_tool_call",
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
            "compat_proxy.usage",
            &[("detail", truncate_chars(&usage.to_string(), 300))],
        );
    }
    let body = serde_json::to_string(&out).unwrap_or_else(|_| "{}".into());
    log_at(
        log,
        "info",
        "compat_proxy.response",
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
                        "compat_proxy.usage_timeout",
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
                        log_at(log, "info", "compat_proxy.stream_done", &[]);
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
                    "compat_proxy.stream_end",
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
    // 标签剥离与「回合收尾约束和助推」同开关：关闭时文本直通（不剪标签、不催办）
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
    let session = headers
        .get("session-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("-")
        .to_string();

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
        // 「回合收尾约束和助推」关闭：终局判定与续跑整段跳过（本轮怎么收尾完全由上游决定），
        // 每个入站请求只记一条诊断；后续分支（标题线程放行、催办、催办上限）都不会走到。
        // 放在标题线程判定之前：关闭开关后标题线程也无需再单独记一条 `title_task`
        if !state.nudge_enabled {
            log_at(
                &log,
                "info",
                "compat_proxy.nudge_skipped",
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
                "compat_proxy.nudge_skipped",
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
                "compat_proxy.nudge_skipped",
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
                "compat_proxy.nudge_skipped",
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
                "compat_proxy.nudge_skipped",
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
                "compat_proxy.nudge_skipped",
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
                "compat_proxy.nudge_limited",
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
            "compat_proxy.nudge_injected",
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
                        "compat_proxy.nudge_skipped",
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
                    "compat_proxy.nudge_skipped",
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
                    "compat_proxy.nudge_skipped",
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

/// `compat_proxy.nudge_injected` 的中文 `说明`：一句话讲清「为什么催、要求什么」。
fn nudge_injected_note(plan_mode: bool) -> &'static str {
    if plan_mode {
        "模型以纯文本收尾且未交付计划，已注入计划续跑提醒（要求给出 <proposed_plan> 或两个出口标签）"
    } else {
        "模型以纯文本收尾且未宣告完成，已注入续跑提醒（要求继续调用工具或回 <zen_task_completed>）"
    }
}

/// `compat_proxy.nudge_skipped` 里会话标题线程的中文 `说明`（其余原因的说明就地写在日志调用处）。
const NUDGE_SKIP_NOTE_TITLE_TASK: &str = "会话标题线程，整轮放行";
/// `compat_proxy.nudge_skipped` 里「回合收尾约束和助推」开关关闭时的中文 `说明`。
const NUDGE_SKIP_NOTE_DISABLED: &str = "回合收尾约束和助推已关闭，不注入也不催办";
/// `compat_proxy.nudge_skipped` 里走 Responses 直通（上游只认该协议）时的中文 `说明`。
const NUDGE_SKIP_NOTE_RESPONSES_PROTOCOL: &str =
    "该模型只支持 Responses 协议，本请求按原协议直通（收尾教学/续跑/标签剥离等翻译管线能力不参与）";

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
    /// 是否启用剥离（= 「回合收尾约束和助推」开关）：关闭时**直通**——文本原样下发、
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
        "compat_proxy.upstream_error",
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
                &json!({ "error": { "message": msg, "type": "compat_proxy_error" } }),
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

/// 当前 Unix 纳秒时间戳（用于本地生成请求 id）。
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
    let now = unix_now_nanos();
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
/// `strip_tags` 为「回合收尾约束和助推」开关：关闭时不剥离 zen 标签（见 [`TagStripper`]）。
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
            // 「回合收尾约束和助推」关闭时不剥离，正文原样交给 codex。
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
    /// 是否剥离 zen 收尾标签（= 「回合收尾约束和助推」开关）：关闭时文本直通，
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
    /// `strip_tags` 默认开启（与「回合收尾约束和助推」的默认值一致）；代理在
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
/// 无论哪条路径都记一次 `compat_proxy.stream_summary`。
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
            "compat_proxy.stream_error",
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
                "compat_proxy.malformed_tool_call",
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
            "compat_proxy.usage",
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
        "compat_proxy.stream_summary",
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
