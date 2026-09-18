//! Zen 本地代理：把 codex 的 OpenAI Responses 请求翻译为 OpenCode Zen 的
//! Chat Completions 请求并转发；响应反向翻译回 Responses 格式（含流式 SSE）。
//!
//! - Zen base URL 与 User-Agent 硬编码（模块私有，不暴露到前端）。
//! - API Key 不固定：读取入站请求的 `Authorization` 头原样转发。
//! - 仅绑定回环地址，专供 codex-ui 自身使用，无额外鉴权。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use axum::Router;
#[cfg(test)]
use axum::Json;
use futures_util::stream::{self, StreamExt};
use serde_json::{Value, json};
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
const ZEN_USER_AGENT: &str =
    "opencode/1.18.29 ai-sdk/provider-utils/4.0.23 runtime/node.js/24";
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
const MISSING_TOOL_OUTPUT_TEXT: &str =
    "该工具调用未执行（参数非法或调用被中止），请重新发起。";
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
/// 因此应用聊天里看不到这条消息（模型按提醒回出的 [`TASK_COMPLETED_MARKER`] 标签是模型
/// 自己的输出，会留在聊天里、以可见字面文本显示）。
///
/// **二选一强收尾**：默认模式没有独立判定请求可用了（Zen 免费层会拒那条后台
/// `chat/completions`），所以判定搬进原对话——被催办时模型只有两条路：
/// ① 还有活没做完 → 必须实际调用工具继续做；
/// ② 任务确实已经结束 → 用一行 [`TASK_COMPLETED_MARKER`] 成对标签宣告结束，
/// 代理据此（纯代码前缀判据，见 [`is_task_completed`]）直接收尾。
///
/// 第二句显式排除「重复执行已完成的操作」——操作型请求（如「请 git 提交并推送」）若在更早
/// 回合已经做完，被催办也不能让模型再提交/再推送一次；末句给出标签的**结构不变量**
/// （成对闭合、独占一行、不换行、本轮最多一个、不进代码块），未来的代理层过滤/分桶只依赖
/// 这些结构与标签前缀，不依赖载荷词汇（载荷保持自由文本）。
const NUDGE_TEXT: &str = "【自动续跑】你上一条回复没有调用任何工具就结束了回合。请立刻走下面两条路之一，不要只写正文，也不要写「接下来我会…／马上做…」这类将来时承诺：\n- 还有工作没做完：必须实际调用工具把剩余工作做完；做完后按下面第二条收尾。\n- 任务确实已经结束：不要重复上一条回复的正文，也不要在更早的回合已经用工具执行过的操作上重复执行（例如已经 git 提交或推送过）——用一行 <zen_task_completed>已完成</zen_task_completed> 收尾。标签里只写这四个词之一：已完成／无需改动／已放弃／做不下去，不要写别的说明；标签必须成对闭合、独占一行、不换行、本轮最多只写一个，不要放进代码块，不要改写标签。";
/// 计划模式专用的续跑提醒：计划模式的交付物是「计划」而不是「动手改代码」，
/// 所以不能沿用 [`NUDGE_TEXT`] 的执行口径（否则会把模型逼去在计划模式里改代码）。
/// **排除式三选一**：被催办时先判两种「不需要给方案」的情况——用户已放弃
/// （[`PLAN_CANCEL_MARKER`]）与「问题本身无法或无需产出实现计划」
/// （[`PLAN_UNACHIEVABLE_MARKER`]，如事实问题/纯查询/闲聊），两者都不成立才必须把完整方案
/// 落进 `<proposed_plan>` 骨架。骨架必须带完整形态：弱模型经常只把计划写成普通 Markdown，
/// 而 codex 只在终局文本里出现 `<proposed_plan>` 包裹时才生成计划条目（否则应用里没有
/// 「计划已就绪」），所以提醒里直接给出骨架，并要求标签原样保留、各自独占一行、不要放进代码块。
/// 「计划已被认可、无需改动、保持现状」不属于「无法/无需计划」：这同样是一个评估结论，
/// 应把该结论或重申的原计划写进 `<proposed_plan>` 收尾，而不是逃到非方案标签。
const PLAN_NUDGE_TEXT: &str = "【自动续跑】你上一条回复没有交付计划就结束了回合。请先判断下面两种不需要给方案的情况是否成立，都不成立时才必须给出完整方案。标签必须原样保留、各自独占一行，不要放进代码块，不要改写标签，也不要只写正文：\n- 用户已经放弃这个计划（例如让你不要再处理、先不做了）：不要重新给方案，也不要只写正文，直接用一行 <zen_plan_cancelled>已放弃</zen_plan_cancelled> 收尾（标签里只写「已放弃」这三个字，不要写别的说明）。\n- 问题本身无法或无需产出实现计划（例如「1+1=？」这类事实问题、纯查询或闲聊，本就不产出计划这种交付物）：不要先回答正文，只用一行 <zen_plan_unachievable>无法计划</zen_plan_unachievable> 收尾（标签里只写「无法计划」这四个字，不要写别的说明）。\n以上两种情况都不成立时，必须想办法把完整方案写进下面这个结构里：\n\n<proposed_plan>\n# 计划标题\n- 步骤 1\n- 步骤 2\n</proposed_plan>\n\n即便你的结论是无需改动、保持现状或原有计划已经可以，也要把该结论（或重申原计划）写进这个结构里收尾。";
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
/// 计划模式下「用户已取消这个计划」的约定标记（由 [`PLAN_NUDGE_TEXT`] 教给模型，
/// 教学形态是成对标签 `<zen_plan_cancelled>放弃计划原因</zen_plan_cancelled>`，与计划产物
/// 标签 [`PLAN_OUTPUT_MARKER`] 同形：`<proposed_plan>` = 计划已交付、本标记 = 计划已取消）。
/// 应用侧 Markdown 渲染会把尖括号转义成可见字面文本（`&lt;zen_plan_cancelled&gt;…`），
/// 不会被当成 HTML 吞掉。
/// **计划模式**下终局文本出现即视为计划话题已终结、直接收尾：不再续跑，也**不生成计划条目**
/// （聊天里只是一句普通结论加一行可见的标签，不弹「计划已就绪」）。
/// 判定与 [`PLAN_OUTPUT_MARKER`] 同口径（大小写不敏感 + 前缀匹配）：兼容大写、缺闭合标签
/// 等写法；计划标签判据优先于本判据。**不兼容旧的无前缀写法**（`<cancelled_plan>`）。
const PLAN_CANCEL_MARKER: &str = "<zen_plan_cancelled";
/// 计划模式下「问题本身无法或无需产出实现计划」的约定标记（由 [`PLAN_NUDGE_TEXT`] 教给
/// 模型，教学形态是成对标签 `<zen_plan_unachievable>原因</zen_plan_unachievable>`，与
/// [`PLAN_OUTPUT_MARKER`] / [`PLAN_CANCEL_MARKER`] 同形）：`<proposed_plan>` = 计划已交付、
/// `<zen_plan_cancelled>` = 计划已取消、本标记 = 无法/无需计划（如「1+1=？」这类事实问题、
/// 纯查询或闲聊，本就不产出计划这种交付物）。「计划已被认可、无需改动、保持现状」**不属于**
/// 本标记：那是评估结论，应写进 `<proposed_plan>` 收尾，不逃到非方案标签。
/// 应用侧 Markdown 渲染会把尖括号转义成可见字面文本（`&lt;zen_plan_unachievable&gt;…`），
/// 不会被当成 HTML 吞掉。
/// **计划模式**下终局文本出现即视为计划话题已终结、直接收尾：不再续跑，也**不生成计划条目**。
/// 判定与 [`PLAN_OUTPUT_MARKER`] 同口径（大小写不敏感 + 前缀匹配）：兼容大写、缺闭合标签
/// 等写法；计划标签判据优先于本判据。**不兼容旧的无前缀写法**（`<unachievable_plan>`）。
const PLAN_UNACHIEVABLE_MARKER: &str = "<zen_plan_unachievable";
/// **默认模式**下「任务已经结束」的约定标记（由首轮教学的 [`DEFAULT_MODE_CONTRACT_TEXT`] 与
/// [`NUDGE_TEXT`] 教给模型，教学形态是成对标签
/// `<zen_task_completed>一句话结论</zen_task_completed>`）：默认模式的判定搬进原对话——
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
/// 不换行、一整轮最多一个、不放进代码块。有了这些，过滤 = 删掉整段闭合标签（缺闭合时删到该行
/// 行尾），与载荷内容无关；分桶 = 对载荷首词做软映射（已完成/无需改动/已放弃/无法继续 → 枚举），
/// 映射不到落 `unknown`。若将来真需要机器可读的强枚举，优先加**第二个标签名**（沿用 plan 系
/// 「词表在标签名里、载荷自由」的风格），而不是把枚举塞进载荷。
///
/// 应用侧 Markdown 渲染会把尖括号转义成可见字面文本（`&lt;zen_task_completed&gt;…`），不会被
/// 当成 HTML 吞掉。默认模式下终局文本出现即视为任务已终结、直接收尾；计划模式不使用本判据。
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
/// 本轮最多一个、不进代码块）——单测对两者都做断言，防半改。
/// 准入见 [`contract_injection_eligible`]：只给「流式 + 声明了工具 + 非会话标题线程」的请求
/// 注入；非流式没有催办路径可消费，标题线程只产出标题，压缩/摘要类后台请求也通常不带工具。
const DEFAULT_MODE_CONTRACT_TEXT: &str = "【回合收尾约定】当你结束回合、且本轮没有调用任何工具时：任务已全部完成就用一行 <zen_task_completed>已完成</zen_task_completed> 收尾——标签里只写这四个词之一：已完成／无需改动／已放弃／做不下去，不要写别的说明（成对闭合、独占一行、不换行、本轮最多只写一个，不要放进代码块）；还有没做完的工作必须实际调用工具继续做，不要只写「接下来我会…」这类承诺；不要重复执行更早回合已经用工具做过的操作。";
/// **首轮教学**（计划模式）：只前置两个「不需要给方案」的出口标签，不前置「必须给完整方案」——
/// 计划模式的正常形态是 chat your way，强制口径留在 [`PLAN_NUDGE_TEXT`] 里，避免把中间闲聊
/// 回合逼出假方案。边界（「无需改动、保持现状、原计划已认可」属于评估结论、要写进
/// `<proposed_plan>`、不算 unachievable）必须写在这里，否则会重现 2026-09-18 那次误逃。
const PLAN_MODE_CONTRACT_TEXT: &str = "【计划模式收尾约定】若用户已放弃这个计划（让你不要再处理、先不做了），用一行 <zen_plan_cancelled>已放弃</zen_plan_cancelled> 收尾（标签里只写「已放弃」，不要写别的说明）；若问题本身无法或无需产出实现计划（事实问题、纯查询、闲聊），用一行 <zen_plan_unachievable>无法计划</zen_plan_unachievable> 收尾（标签里只写「无法计划」，不要写别的说明）；其余情况照常把完整方案写进 <proposed_plan>（「无需改动、保持现状、原计划已认可」属于评估结论，要写进 <proposed_plan>，不算 unachievable）。这两个标签必须成对闭合、独占一行、不换行、本轮最多只写一个，不要放进代码块。";

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
/// （用户放弃后看到的就是模型那句普通结论加一行可见标签）；默认模式不使用本判据。
fn is_plan_cancelled(text: &str) -> bool {
    text.to_lowercase().contains(PLAN_CANCEL_MARKER)
}

/// 终局文本是否带「问题无法或无需产出实现计划」标记（见 [`PLAN_UNACHIEVABLE_MARKER`]）：
/// **计划模式**下命中即视为计划话题已终结并直接收尾——不再注入续跑提醒，也**不生成计划条目**
/// （例如「1+1=？」这类事实问题，聊天里就是模型那句结论加一行可见标签）；默认模式不使用本判据。
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
    /// 生效中的上游 base_url（已归一化，用于判断是否需要重启）。
    base_url: String,
    abort: AbortHandle,
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

/// 上游是否为 OpenCode Zen（host 是 `opencode.ai` 或其子域，大小写不敏感）。
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
    host == "opencode.ai" || host.ends_with(".opencode.ai")
}

/// 在当前 tokio runtime 上启动本地代理；端口被占用时返回 Err。
pub(crate) async fn start(
    port: u16,
    base_url: String,
    log: ZenLog,
    trace: TraceSink,
    modes: Arc<ThreadModeRegistry>,
) -> Result<ZenProxyHandle, String> {
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
    let base_url = normalize_base_url(&base_url);
    let app = Router::new()
        .fallback(handle_any)
        .with_state(ProxyState {
            session: opencode_id("ses", true),
            session_map: Arc::new(SessionMap::default()),
            base_url: base_url.clone(),
            log,
            trace,
            requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
            zen_body_patch: is_zen_upstream(&base_url),
            modes,
        });
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(ZenProxyHandle {
        port,
        base_url,
        abort: task.abort_handle(),
    })
}

/// 由 `CodexServer` 调用：按设置启停代理并返回当前状态。
pub(crate) async fn apply(
    handle: &mut Option<ZenProxyHandle>,
    enabled: bool,
    port: u16,
    base_url: String,
    log: ZenLog,
    trace: TraceSink,
    modes: Arc<ThreadModeRegistry>,
) -> ZenProxyStatus {
    let base_url = normalize_base_url(&base_url);
    // 端口与上游地址都没变（含 `https://a/` 与 `https://a` 这类等价写法）才复用现有实例。
    let unchanged = handle
        .as_ref()
        .is_some_and(|h| h.port == port && h.base_url == base_url);
    let need_start = enabled && !unchanged;
    if !enabled || !unchanged {
        if let Some(h) = handle.take() {
            h.stop();
        }
    }
    if need_start {
        match start(port, base_url, log, trace, modes).await {
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
    /// 是否按 Zen 免费层的请求体门禁补形状（`max_tokens` + 内置工具名，见
    /// [`patch_zen_request_body`]）：上游是 opencode.ai 时为 true，其它上游保持请求体原样。
    zen_body_patch: bool,
    /// 协作模式登记表（key = codex 线程 id）：由 app-server 侧登记，代理解析模式时优先查它。
    modes: Arc<ThreadModeRegistry>,
}

/// 写一条 zen_proxy 诊断日志；句柄为 None 或写盘失败时静默忽略。
fn log_at(log: &Option<Arc<SessionLog>>, level: &str, event: &str, kv: &[(&str, String)]) {
    let Some(log) = log else { return };
    let kv: Vec<(String, String)> = kv
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    log.write(level, None, event, &kv);
}

/// opencode 客户端标识的字符集（与 `sst/opencode` 的 `Identifier.create` 一致）。
const OPENCODE_ID_CHARS: &[u8] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
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
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(ZEN_USER_AGENT)
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
            return error_json(
                StatusCode::BAD_REQUEST,
                format!("请求体不是合法 JSON：{e}"),
            );
        }
    };
    let want_stream = req
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // 本次入站请求的稳定标识：贯穿内容日志文件名与 session 日志行。
    let call_id = opencode_id("req", false);
    // 协作模式先按协议登记表解析（查不到才退回关键词），日志里连同来源一起记下
    let (plan_mode, mode_src) = resolve_nudge_mode(state, headers, &req);
    let mode = if plan_mode { "plan" } else { "default" };
    // 首轮教学：把收尾契约追加到 instructions 尾部（必须在模式解析之后——契约文本里不含
    // `<collaboration_mode>` 块，模式判据与 mode_src 因此完全不受影响）
    let contract = if contract_injection_eligible(
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
                        if state.zen_body_patch { "on" } else { "off" }.to_string(),
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
                    &[
                        (
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
                        ),
                    ],
                );
            }
            if forwarded.repairs.invalid_arguments > 0 || forwarded.repairs.missing_tool_outputs > 0 {
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
                        proxy_json_response(req, resp, &state.log, forwarded.trace).await
                    }
                }
            }
        }
        Err((status, msg)) => {
            log_at(
                &state.log,
                "warn",
                "zen_proxy.forward_error",
                &[
                    ("url", base_url),
                    ("error", msg.clone()),
                ],
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
    let level = if status.starts_with('2') { "info" } else { "warn" };
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

/// 构造转发给上游的请求头：保留入站头，剔除逐跳头并附加固定 opencode 识别头。
fn forwarded_request_headers(headers: &HeaderMap, state: &ProxyState) -> HeaderMap {
    let mut out = headers.clone();
    for name in HOP_BY_HOP_HEADERS {
        out.remove(name);
    }
    out.remove(header::HOST);
    out.remove(header::CONTENT_LENGTH);
    // 保持固定 opencode User-Agent，不沿用入站客户端 UA
    out.remove(header::USER_AGENT);
    out.insert(
        "x-opencode-client",
        HeaderValue::from_static(OPENCODE_CLIENT),
    );
    out.insert(
        "x-opencode-project",
        HeaderValue::from_static(OPENCODE_PROJECT),
    );
    if let Ok(value) = HeaderValue::from_str(&opencode_id("msg", false)) {
        out.insert("x-opencode-request", value);
    }
    if let Ok(value) = HeaderValue::from_str(&opencode_session(state, headers)) {
        out.insert("x-opencode-session", value);
    }
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
    for name in ZEN_REQUIRED_TOOL_NAMES {
        if !seen.insert(name.to_string()) {
            continue;
        }
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
        .filter(|item| {
            item.get("type").and_then(Value::as_str) == Some("custom_tool_call_output")
        })
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
    call.note("x_opencode_session", session.to_string());
    call.note("session_codex", session_codex.unwrap_or("-").to_string());
    call.note("x_opencode_request", request_id.to_string());
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
    // 补丁**同受 `zen_body_patch` 约束**——只有上游 host 是 opencode.ai 及其子域时才加，
    // 换成 DeepSeek 等自建/第三方端点时两项都不加（无法只开其中一项）。放进可选字段列表
    // 是为了上游指名拒绝它能走既有「摘掉后重试一次」的降级。
    if state.zen_body_patch {
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
        // 与上面的 `max_tokens` 完全同源，同样只在上游 host 是 opencode.ai 时为真。
        if state.zen_body_patch {
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
            &body,
            &dropped,
            reasoning_rc,
        );
        call.write_request_json(&body);
        let resp = build_chat_request(client, &url, headers, &session, &request_id, &body)
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
                state.requires_reasoning_rc.store(enabled, Ordering::Relaxed);
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

/// 构造发往 Zen 的 chat/completions 请求（固定识别头 + 透传的 Authorization）。
fn build_chat_request(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    session: &str,
    request_id: &str,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut rq = client
        .post(url)
        .json(body)
        .header("x-opencode-client", OPENCODE_CLIENT)
        .header("x-opencode-project", OPENCODE_PROJECT)
        .header("x-opencode-request", request_id)
        .header("x-opencode-session", session);
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
    call.note(
        "usage",
        if usage.is_some() { "present" } else { "none" },
    );
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
    mut call: TraceCall,
) -> Response {
    let status = resp.status();
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            call.note("read_error", e.to_string());
            call.note("suspicious", "upstream_read_error");
            call.finish();
            return error_json(
                StatusCode::BAD_GATEWAY,
                format!("读取上游响应失败：{e}"),
            );
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
                "上游响应不是合法 JSON：".to_string()
                    + &text.chars().take(200).collect::<String>(),
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
    let out = match chat_to_responses(&chat, &model, &tool_shape(&req)) {
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
    let created = sse_event("response.created", &json!({
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
    }));
    // 通道 + 独立任务：事件一边产出一边下发，续跑都发生在同一个响应流里
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    tokio::spawn(async move {
        run_stream_task(
            state, headers, call_id, req, resp, call, response_id, model, created, tx,
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
                    (
                        "说明",
                        NUDGE_SKIP_NOTE_TITLE_TASK.to_string(),
                    ),
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
                        format!(
                            "本请求已催办 {NUDGE_MAX_INJECTIONS} 次仍未收尾，停止催办"
                        ),
                    ),
                ],
            );
            break;
        }
        // 两个模式各用各的提醒：默认模式那条负责把 `<zen_task_completed>` 教给模型
        // （判定搬进原对话，不再有独立的判定请求）
        let nudge_text = if plan_mode { PLAN_NUDGE_TEXT } else { NUDGE_TEXT };
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
        match forward(&body, &headers, true, &state, &nudge_call_id).await
        {
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
                            (
                                "说明",
                                "续跑轮上游返回错误，已放弃本次催办".to_string(),
                            ),
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
                        (
                            "说明",
                            "续跑轮上游非 2xx，已放弃本次催办".to_string(),
                        ),
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
    fn new() -> Self {
        Self {
            line_start_ws: true,
            ..Self::default()
        }
    }

    /// 喂一段上游文本，返回可以下发给 codex 的可见文本（可能为空）。
    fn feed(&mut self, text: &str) -> String {
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
    fn strip_once(text: &str) -> (String, String) {
        let mut stripper = Self::new();
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
        .body(Body::from(serde_json::to_string(&body).unwrap_or_else(|_| "{}".into())))
        .unwrap_or_else(|_| error_json(StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败".into()))
}

fn error_json(status: StatusCode, msg: String) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({ "error": { "message": msg, "type": "zen_proxy_error" } }))
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
                let Some(obj) = item.as_object() else { continue };
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
                            let input = obj
                                .get("input")
                                .map(value_to_text)
                                .unwrap_or_default();
                            json!({ "input": input }).to_string()
                        } else {
                            let raw_arguments = obj
                                .get("arguments")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
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
                            p["tool_calls"]
                                .as_array_mut()
                                .map(|arr| arr.push(tc));
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
                        let mut content =
                            value_to_text(obj.get("output").unwrap_or(&Value::Null));
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
            let Some(obj) = tool.as_object() else { continue };
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
                    .filter_map(|tc| {
                        tc.get("id").and_then(|v| v.as_str()).map(str::to_string)
                    })
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
        while j < messages.len()
            && messages[j].get("role").and_then(|r| r.as_str()) == Some("tool")
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
fn chat_to_responses(
    chat: &Value,
    model: &str,
    shape: &ToolShape,
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
            // 非流式路径同样按结构剥离三个 zen 标签（一次性，无跨分片问题）
            let content = TagStripper::strip_once(&content).0;
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
                                if name.is_empty() { "未知工具" } else { &name },
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
        &["prompt_tokens_details.cached_tokens", "cache_read_input_tokens"],
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
        &["completion_tokens_details.reasoning_tokens", "reasoning_tokens"],
    ) {
        output_details["reasoning_tokens"] = v;
    }
    if output_details.as_object().is_some_and(|map| !map.is_empty()) {
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
    fn new(response_id: String, model: String) -> Self {
        Self {
            response_id,
            model,
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
                let idx = tc
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(0) as usize;
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
            stripper: TagStripper::new(),
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
            stripper: TagStripper::new(),
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
        // 收尾前先把剥离器压住的尾巴交出来（未闭合标签按规则丢弃），
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
    let delta_keys = st
        .delta_keys
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
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
                if st.usage.is_some() { "present" } else { "none" }.to_string(),
            ),
            (
                "delta_keys",
                delta_keys,
            ),
            ("suspicious", suspicious),
        ],
    );
    out
}

/// codex 0.154 实测的计划模式协作块（标题行是 `# Plan Mode (Conversational)`；旧版 codex
/// 的 `# Collaboration Mode: Plan` 已不再出现，但仍保留兼容分支）。测试共用。
#[cfg(test)]
const REAL_PLAN_MODE_BLOCK: &str = "<collaboration_mode># Plan Mode (Conversational)\r\n\r\nYou work in 3 phases, and you should *chat your way* to a great plan before finalizing it.\r\n\r\n## Mode rules (strict)\r\n\r\nYou are in **Plan Mode** until a developer message explicitly ends it.\r\n\r\nEventually issuing a `<proposed_plan>` block.\r\n</collaboration_mode>";
/// codex 0.154 实测的默认模式协作块：正文第一句就含 `(e.g. Plan mode)`，只认标题行才不会误判。
#[cfg(test)]
const REAL_DEFAULT_MODE_BLOCK: &str = "<collaboration_mode># Collaboration Mode: Default\r\n\r\nYou are now in Default mode. Any previous instructions for other modes (e.g. Plan mode) are no longer active.\r\n\r\nYour active mode changes only when new developer instructions with a different `<collaboration_mode>...</collaboration_mode>` change it.\r\n</collaboration_mode>";

/// 断言 `id` 是 opencode 形状：`prefix_` + 26 位 `[0-9A-Za-z]`（Zen 服务端的要求）。
/// 两个测试模块共用：`tests` 断言纯函数产物、`integration_tests` 断言实际发出的头。
#[cfg(test)]
fn assert_is_opencode_id(id: &str, prefix: &str) {
    let body = id
        .strip_prefix(&format!("{prefix}_"))
        .unwrap_or_else(|| panic!("应以 {prefix}_ 开头：{id}"));
    assert_eq!(body.len(), 26, "后段应为 26 位：{id}");
    assert!(
        body.chars().all(|c| c.is_ascii_alphanumeric()),
        "后段只允许 0-9A-Za-z：{id}"
    );
}

/// 测试用代理状态：会话映射表独立（避免跨用例串味），基址可指向本地 mock。
#[cfg(test)]
fn test_proxy_state(session: &str, base_url: &str, log: ZenLog, trace: TraceSink) -> ProxyState {
    ProxyState {
        session: session.to_string(),
        session_map: Arc::new(SessionMap::default()),
        base_url: base_url.to_string(),
        log,
        trace,
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        zen_body_patch: is_zen_upstream(base_url),
        modes: Arc::new(ThreadModeRegistry::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- 口嗨检测：纯函数与分会话计数 ----------

    #[test]
    fn request_is_plan_mode_reads_collaboration_mode_heading() {
        // 计划模式：codex 把模式块拼在 developer 条目末尾（前面还有 skills 等文本）
        let plan = json!({
            "instructions": "You are a coding agent running in the Codex CLI…",
            "input": [
                { "type": "message", "role": "developer", "content": [{
                    "type": "input_text",
                    "text": format!("<skills_instructions>## Skills …</skills_instructions><multi_agent_mode>…</multi_agent_mode>{REAL_PLAN_MODE_BLOCK}")
                }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "继续" }] }
            ]
        });
        assert!(request_is_plan_mode(&plan));
        assert_eq!(last_mode_block_declares_plan(REAL_PLAN_MODE_BLOCK), Some(true));

        // 旧版 codex 文案（`# Collaboration Mode: Plan`）：保留兼容
        let legacy = json!({
            "input": [
                { "type": "message", "role": "developer", "content": [{
                    "type": "input_text",
                    "text": "</permissions instructions><collaboration_mode># Collaboration Mode: PLAN\r\n\r\nYou are now in Plan mode …</collaboration_mode>"
                }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "继续" }] }
            ]
        });
        assert!(request_is_plan_mode(&legacy));

        // 默认模式：正文里的「(e.g. Plan mode)」不能误判（只认标题行）
        let default_mode = json!({
            "input": [
                { "type": "message", "role": "developer", "content": [{
                    "type": "input_text",
                    "text": REAL_DEFAULT_MODE_BLOCK
                }] }
            ]
        });
        assert!(!request_is_plan_mode(&default_mode));
        assert_eq!(
            last_mode_block_declares_plan(REAL_DEFAULT_MODE_BLOCK),
            Some(false)
        );
        // 用户正文里出现「Plan mode / proposed_plan」同样不因此被判成计划模式
        assert!(!request_is_plan_mode(&json!({
            "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "计划模式(Plan mode)下要输出 <proposed_plan>" }] }]
        })));

        // 模式块放在 instructions 里、或 input 是字符串简写：同样生效
        assert!(request_is_plan_mode(&json!({
            "instructions": format!("前文…\r\n{REAL_PLAN_MODE_BLOCK}")
        })));
        assert!(request_is_plan_mode(&json!({
            "input": format!("{REAL_PLAN_MODE_BLOCK}\r\n继续")
        })));

        // 标签后先空行再标题、以及文本被截断（块未闭合）：仍按标题行判定
        assert_eq!(
            last_mode_block_declares_plan(
                "<collaboration_mode>\r\n\r\n# Plan Mode (Standard)\r\n…（内容被截断）"
            ),
            Some(true)
        );
        // 标签后直接就是正文（没有标题行）时按非计划模式处理
        assert_eq!(
            last_mode_block_declares_plan("<collaboration_mode>You are now in Plan mode"),
            Some(false)
        );

        // 没有模式块时返回 None（调用方据此继续扫下一段文本）
        assert_eq!(last_mode_block_declares_plan("普通文本，没有模式标签"), None);

        // 完全没有模式信息（例如其它客户端）：按非计划模式处理
        assert!(!request_is_plan_mode(&json!({ "input": "hi" })));
        assert!(!request_is_plan_mode(&json!({})));
    }

    /// 线上误判的最小复现：codex 把历次模式块都留在历史里，切回默认模式后请求里
    /// 「旧的计划块在前、当前的默认块在后」——必须按**最后一块**判定为默认模式。
    #[test]
    fn request_is_plan_mode_takes_the_last_mode_block() {
        // 历史里先有旧的计划模式块，切回默认模式后 codex 追加了默认模式块
        let switched_back = json!({
            "input": [
                { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": REAL_PLAN_MODE_BLOCK }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "先出计划" }] },
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "…" }] },
                { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": REAL_DEFAULT_MODE_BLOCK }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "继续" }] }
            ]
        });
        // 计划块在前、默认块在后 → 默认模式（修复前这里会误判成计划模式）
        assert!(!request_is_plan_mode(&switched_back));

        // 反向：默认块在前、计划块在后 → 计划模式
        let back_to_plan = json!({
            "input": [
                { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": REAL_DEFAULT_MODE_BLOCK }] },
                { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": REAL_PLAN_MODE_BLOCK }] }
            ]
        });
        assert!(request_is_plan_mode(&back_to_plan));

        // 同一段文本里多个模式块：同样以最后一个为准
        assert!(!request_is_plan_mode(&json!({
            "input": format!("{REAL_PLAN_MODE_BLOCK}\r\n中间内容\r\n{REAL_DEFAULT_MODE_BLOCK}")
        })));
        assert!(request_is_plan_mode(&json!({
            "input": format!("{REAL_DEFAULT_MODE_BLOCK}\r\n中间内容\r\n{REAL_PLAN_MODE_BLOCK}")
        })));
        assert_eq!(
            last_mode_block_declares_plan(&format!(
                "{REAL_PLAN_MODE_BLOCK}\r\n{REAL_DEFAULT_MODE_BLOCK}"
            )),
            Some(false)
        );
    }

    #[test]
    fn thread_mode_from_params_reads_request_and_notification_shapes() {
        // turn/start、thread/settings/update 的参数形状
        assert_eq!(
            thread_mode_from_params(&json!({
                "threadId": "01a0aaf8-abc",
                "collaborationMode": { "mode": "plan", "settings": { "model": "m" } }
            })),
            Some(("01a0aaf8-abc".to_string(), "plan".to_string()))
        );
        // thread/settings/updated 通知的形状
        assert_eq!(
            thread_mode_from_params(&json!({
                "threadId": "t2",
                "threadSettings": {
                    "collaborationMode": { "mode": "default", "settings": { "model": "m" } }
                }
            })),
            Some(("t2".to_string(), "default".to_string()))
        );
        // 线程 id 前后空白先 trim
        assert_eq!(
            thread_mode_from_params(&json!({
                "threadId": " t3 ",
                "collaborationMode": { "mode": "plan" }
            })),
            Some(("t3".to_string(), "plan".to_string()))
        );

        // collaborationMode 为 null / 缺失 / 未知取值 → None（调用方保持已有登记不变）
        assert_eq!(
            thread_mode_from_params(&json!({ "threadId": "t", "collaborationMode": null })),
            None
        );
        assert_eq!(thread_mode_from_params(&json!({ "threadId": "t" })), None);
        assert_eq!(
            thread_mode_from_params(&json!({
                "threadId": "t",
                "collaborationMode": { "mode": "chatty" }
            })),
            None
        );
        // 缺线程 id / 纯空白线程 id → None
        assert_eq!(
            thread_mode_from_params(&json!({ "collaborationMode": { "mode": "plan" } })),
            None
        );
        assert_eq!(
            thread_mode_from_params(&json!({
                "threadId": "   ",
                "collaborationMode": { "mode": "plan" }
            })),
            None
        );
        // 与协作模式无关的通知（如 turn/started）→ None
        assert_eq!(
            thread_mode_from_params(&json!({ "threadId": "t", "turn": { "id": "x" } })),
            None
        );
    }

    #[test]
    fn thread_mode_registry_records_overwrites_and_isolates() {
        let registry = ThreadModeRegistry::default();
        assert_eq!(registry.mode("t1"), None, "未登记应查不到");
        assert!(registry.record("t1", "plan"), "首次登记算变化");
        assert_eq!(registry.mode("t1").as_deref(), Some("plan"));
        // 同值重复登记：不算变化（也不影响取值）
        assert!(!registry.record("t1", "plan"));
        assert_eq!(registry.mode("t1").as_deref(), Some("plan"));
        // 覆盖为默认模式
        assert!(registry.record("t1", "default"));
        assert_eq!(registry.mode("t1").as_deref(), Some("default"));
        // 登记时线程 id 先 trim
        assert!(registry.record("  t2  ", "plan"));
        assert_eq!(registry.mode("t2").as_deref(), Some("plan"));
        // 非法取值与空白线程 id 一律忽略，不改动已有值
        assert!(!registry.record("t1", "chatty"));
        assert_eq!(registry.mode("t1").as_deref(), Some("default"));
        assert!(!registry.record("   ", "plan"));
        assert_eq!(registry.mode("   "), None);
        // 会话之间互不影响
        assert_eq!(registry.mode("t3"), None);
    }

    #[test]
    fn thread_mode_registry_clears_when_full() {
        let registry = ThreadModeRegistry::default();
        for i in 0..THREAD_MODE_LIMIT {
            assert!(registry.record(&format!("t{i}"), "plan"));
        }
        // 到达上限后再登记**新**线程：整表清空后只留这一条（模式每轮 turn 都会重新登记）
        assert!(registry.record("t-new", "default"));
        assert_eq!(registry.mode("t-new").as_deref(), Some("default"));
        assert_eq!(registry.mode("t0"), None, "清空后旧条目不再保留");
        // 表内的已有线程照常覆盖，不会触发清空
        assert!(registry.record("t-new", "plan"));
        assert_eq!(registry.mode("t-new").as_deref(), Some("plan"));
    }

    #[test]
    fn request_header_thread_id_normalizes_client_value() {
        // 本模块不依赖集成测试里的构造器，这里就地造一个带 `session-id` 的请求头
        let session_headers = |id: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("session-id", HeaderValue::from_str(id).unwrap());
            headers
        };
        // codex 上报裸线程 id（`ses_` 前缀只可能来自别的客户端，归一化时剥掉）
        assert_eq!(
            request_header_thread_id(&session_headers("01a0aaf8-abc")).as_deref(),
            Some("01a0aaf8-abc")
        );
        // 客户端已带前缀：去掉前缀后查登记表（与 SessionMap 的 key 归一化同一口径）
        assert_eq!(
            request_header_thread_id(&session_headers("ses_abc")).as_deref(),
            Some("abc")
        );
        // 前后空白先 trim
        assert_eq!(
            request_header_thread_id(&session_headers("  abc  ")).as_deref(),
            Some("abc")
        );
        // 缺失 / 纯前缀 / 纯空白 / 非 ASCII → None（调用方退回关键词兜底判据）
        assert_eq!(request_header_thread_id(&HeaderMap::new()), None);
        assert_eq!(request_header_thread_id(&session_headers("ses_")), None);
        assert_eq!(request_header_thread_id(&session_headers("   ")), None);
        let mut invalid = HeaderMap::new();
        invalid.insert("session-id", HeaderValue::from_bytes(b"caf\xe9").unwrap());
        assert_eq!(request_header_thread_id(&invalid), None);
    }

    #[test]
    fn is_plan_deliverable_matches_proposed_plan_wrapper() {
        assert!(is_plan_deliverable(
            "<proposed_plan>\n# 标题\n…\n</proposed_plan>"
        ));
        assert!(is_plan_deliverable("前言\n<PROPOSED_PLAN>\n…"));
        assert!(is_plan_deliverable("前后有文字的 <proposed_plan 片段"));
        assert!(!is_plan_deliverable("我看完了代码，结论是不需要改动。"));
        assert!(!is_plan_deliverable(""));
    }

    #[test]
    fn is_plan_cancelled_matches_tag_marker() {
        // 标签名与命名口径锁死：自研标签统一 `zen_plan_` 前缀
        assert_eq!(PLAN_CANCEL_MARKER, "<zen_plan_cancelled");
        assert!(
            PLAN_CANCEL_MARKER.starts_with("<zen_plan_"),
            "计划模式出口标签必须走 zen_plan_ 命名：{PLAN_CANCEL_MARKER}"
        );
        // 教学里给的标签形态（用户中途放弃计划时的正确收尾）
        assert!(is_plan_cancelled(
            "好，那就不处理了。\n<zen_plan_cancelled>用户说不用改了</zen_plan_cancelled>"
        ));
        // 与 `<proposed_plan>` 同口径：大小写不敏感 + 前缀匹配，缺闭合标签也命中
        assert!(is_plan_cancelled(
            "<ZEN_PLAN_CANCELLED>用户说不用改了</ZEN_PLAN_CANCELLED>"
        ));
        assert!(is_plan_cancelled("<zen_plan_cancelled>用户说不用改了"));
        assert!(is_plan_cancelled("<zen_plan_cancelled>"));
        // 普通结论、空串、裸中文措辞与正常交付的计划都不算「已取消」
        assert!(!is_plan_cancelled("我看完了代码，结论是不需要改动。"));
        assert!(!is_plan_cancelled(""));
        assert!(!is_plan_cancelled("用户已经放弃计划，所以不改了"));
        assert!(!is_plan_cancelled(
            "<proposed_plan>\n# 标题\n- 步骤 1\n</proposed_plan>"
        ));
        // 旧的无前缀写法不再被识别（只认新标签）
        assert!(!is_plan_cancelled(
            "<cancelled_plan>用户说不用改了</cancelled_plan>"
        ));
    }

    #[test]
    fn is_plan_unachievable_matches_tag_marker() {
        assert_eq!(PLAN_UNACHIEVABLE_MARKER, "<zen_plan_unachievable");
        assert!(
            PLAN_UNACHIEVABLE_MARKER.starts_with("<zen_plan_"),
            "计划模式出口标签必须走 zen_plan_ 命名：{PLAN_UNACHIEVABLE_MARKER}"
        );
        // 教学里给的标签形态（问题本身无法或无需产出实现计划时的正确收尾）
        assert!(is_plan_unachievable(
            "1+1=2。\n<zen_plan_unachievable>这是事实问题，不产出实现计划</zen_plan_unachievable>"
        ));
        // 与 `<proposed_plan>` 同口径：大小写不敏感 + 前缀匹配，缺闭合标签也命中
        assert!(is_plan_unachievable(
            "<ZEN_PLAN_UNACHIEVABLE>纯查询，无需计划</ZEN_PLAN_UNACHIEVABLE>"
        ));
        assert!(is_plan_unachievable("<zen_plan_unachievable>闲聊"));
        assert!(is_plan_unachievable("<zen_plan_unachievable>"));
        // 普通结论、空串、裸中文措辞与正常交付/取消的计划都不算「无法/无需计划」
        assert!(!is_plan_unachievable("我看完了代码，结论是不需要改动。"));
        assert!(!is_plan_unachievable(""));
        assert!(!is_plan_unachievable("这个问题没法做"));
        assert!(!is_plan_unachievable(
            "<proposed_plan>\n# 标题\n- 步骤 1\n</proposed_plan>"
        ));
        // 两个前缀（zen_plan_cancelled / zen_plan_unachievable）互不误命中
        assert!(!is_plan_unachievable(
            "<zen_plan_cancelled>用户说不用改了</zen_plan_cancelled>"
        ));
        assert!(!is_plan_cancelled(
            "<zen_plan_unachievable>这是事实问题</zen_plan_unachievable>"
        ));
        // 旧的无前缀写法不再被识别（只认新标签）
        assert!(!is_plan_unachievable(
            "<unachievable_plan>这是事实问题</unachievable_plan>"
        ));
    }

    #[test]
    fn is_task_completed_matches_tag_marker() {
        // 标签名与命名口径锁死：自研标签统一 `zen_` 前缀（本标记用 zen_task_）
        assert_eq!(TASK_COMPLETED_MARKER, "<zen_task_completed");
        assert!(
            TASK_COMPLETED_MARKER.starts_with("<zen_task_"),
            "默认模式收尾标签必须走 zen_task_ 命名：{TASK_COMPLETED_MARKER}"
        );
        // 教学里给的标签形态（默认模式下模型自己宣告任务结束的正确收尾）
        assert!(is_task_completed(
            "结论：已把 base_url 路径测试补上。\n<zen_task_completed>已完成：补了 3 个用例</zen_task_completed>"
        ));
        // 与 plan 系标签同口径：大小写不敏感 + 前缀匹配，缺闭合标签也命中
        assert!(is_task_completed(
            "<ZEN_TASK_COMPLETED>已完成：无需改动</ZEN_TASK_COMPLETED>"
        ));
        assert!(is_task_completed("<zen_task_completed>已完成：已推送"));
        assert!(is_task_completed("<zen_task_completed>"));
        // 普通结论、空串、裸中文措辞都不算「任务已结束」
        assert!(!is_task_completed("我看完了代码，结论是不需要改动。"));
        assert!(!is_task_completed(""));
        assert!(!is_task_completed("任务已经完成，无需改动"));
        // 三个计划标签不命中本判据（模式教学不串味）
        assert!(!is_task_completed(
            "<proposed_plan>\n# 标题\n- 步骤 1\n</proposed_plan>"
        ));
        assert!(!is_task_completed(
            "<zen_plan_cancelled>用户说不用改了</zen_plan_cancelled>"
        ));
        assert!(!is_task_completed(
            "<zen_plan_unachievable>这是事实问题</zen_plan_unachievable>"
        ));
        // 旧的无前缀写法不再被识别（只认新标签）
        assert!(!is_task_completed(
            "<task_completed>已完成</task_completed>"
        ));
    }

    #[test]
    fn request_is_title_task_matches_app_prompt_prefix() {
        let title_req = |text: &str| {
            json!({
                "input": [
                    { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": "…" }] },
                    { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }
                ]
            })
        };

        // 应用 autoTitleThread 的实际提示词（含前导空白/换行时同样命中）
        assert!(request_is_title_task(&title_req(
            "给下面用户消息生成一个不超过 30 字的中文会话标题，只输出标题本身，不要任何解释、引号或 Markdown。\n\n用户消息：\n把 zen 代理的看门狗改一下"
        )));
        assert!(request_is_title_task(&title_req(
            "  \n 给下面用户消息生成一个不超过 30 字的中文会话标题，只输出标题本身"
        )));
        // 简写形态（input 为字符串）同样支持
        assert!(request_is_title_task(&json!({
            "input": "给下面用户消息生成一个不超过 30 字的中文会话标题，只输出标题本身"
        })));

        // 真实会话正文里引用这句话（不在开头）不算标题任务
        assert!(!request_is_title_task(&title_req(
            "帮我把「给下面用户消息生成一个不超过 30 字的中文会话标题」这句提示词改短一点"
        )));
        assert!(!request_is_title_task(&title_req("把 base_url 的测试补上")));
        assert!(!request_is_title_task(&title_req("")));
        assert!(!request_is_title_task(&json!({})));
    }

    #[test]
    fn last_user_text_picks_latest_user_message() {
        let req = json!({
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "第一问" }] },
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "第一次回答" }] },
                { "type": "function_call", "call_id": "c1", "name": "shell", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "c1", "output": "ok" },
                { "role": "user", "content": [{ "type": "input_text", "text": "第二问" }] }
            ]
        });
        assert_eq!(last_user_text(&req), "第二问");
        // 简写：input 为字符串
        assert_eq!(last_user_text(&json!({ "input": "你好" })), "你好");
        // 没有 user 消息 / 没有 input
        assert_eq!(
            last_user_text(&json!({ "input": [{ "type": "message", "role": "assistant", "content": "x" }] })),
            ""
        );
        assert_eq!(last_user_text(&json!({})), "");
    }

    #[test]
    fn nudge_continuation_body_appends_messages_without_touching_original() {
        let original = json!({
            "model": "m",
            "stream": true,
            "instructions": "keep me",
            "tools": [{ "type": "function", "name": "shell" }],
            "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "改文件" }] }]
        });
        let before = original.clone();
        let body = nudge_continuation_body(&original, "Now update the test", NUDGE_TEXT);
        assert_eq!(original, before, "原请求不应被修改");
        assert_eq!(body["instructions"], "keep me");
        assert_eq!(body["tools"], before["tools"]);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[1]["content"][0]["text"], "Now update the test");
        assert_eq!(input[2]["role"], "user");
        assert_eq!(input[2]["content"][0]["type"], "input_text");
        assert_eq!(input[2]["content"][0]["text"], NUDGE_TEXT);

        // 默认模式提醒：二选一（继续干 / 用标签宣告结束），并教出成对标签
        assert!(
            NUDGE_TEXT.contains("必须实际调用工具"),
            "必须保留执行口径，否则弱模型继续只写承诺：{NUDGE_TEXT}"
        );
        assert!(
            NUDGE_TEXT.contains(&format!("{TASK_COMPLETED_MARKER}>")),
            "必须与模型约定「任务已结束」标签，否则判定删掉后没有收尾出口：{NUDGE_TEXT}"
        );
        assert!(
            NUDGE_TEXT.contains("</zen_task_completed>"),
            "必须给出闭合标签，否则弱模型只写正文：{NUDGE_TEXT}"
        );
        assert!(
            is_task_completed(NUDGE_TEXT),
            "提醒文本自身就带标记，前缀判据必须命中：{NUDGE_TEXT}"
        );
        // 结构不变量：未来的代理层过滤/分桶只依赖这些（与载荷词汇无关）
        for needle in ["成对闭合", "独占一行", "不换行", "本轮最多只写一个", "不要放进代码块"] {
            assert!(
                NUDGE_TEXT.contains(needle),
                "提醒必须写明标签结构约定「{needle}」：{NUDGE_TEXT}"
            );
        }
        // 默认模式提醒不教计划标签（模式教学不串味）
        assert!(!NUDGE_TEXT.contains(PLAN_OUTPUT_MARKER), "{NUDGE_TEXT}");
        assert!(!NUDGE_TEXT.contains(PLAN_CANCEL_MARKER), "{NUDGE_TEXT}");
        assert!(!NUDGE_TEXT.contains(PLAN_UNACHIEVABLE_MARKER), "{NUDGE_TEXT}");

        // 计划模式：注入计划专用提醒（同一段构造逻辑，只是文本不同）
        let body = nudge_continuation_body(&original, "先给方案", PLAN_NUDGE_TEXT);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[2]["content"][0]["text"], PLAN_NUDGE_TEXT);
        assert!(PLAN_NUDGE_TEXT.contains("<proposed_plan>"));
        assert!(
            PLAN_NUDGE_TEXT.contains("</proposed_plan>"),
            "必须给出闭合标签，否则弱模型只写正文：{PLAN_NUDGE_TEXT}"
        );
        assert!(
            PLAN_NUDGE_TEXT.contains("不要放进代码块"),
            "必须禁止把标签包进代码块：{PLAN_NUDGE_TEXT}"
        );
        // 用户中途放弃计划时的约定标记：提醒里教的是成对标签，代码按前缀判据识别
        assert!(
            PLAN_NUDGE_TEXT.contains(&format!("{PLAN_CANCEL_MARKER}>")),
            "必须与模型约定「已取消计划」标签，否则用户放弃计划后会被逼重给方案：{PLAN_NUDGE_TEXT}"
        );
        assert!(
            is_plan_cancelled(PLAN_NUDGE_TEXT),
            "提醒文本自身就带标记，前缀判据必须命中：{PLAN_NUDGE_TEXT}"
        );
        // 问题本身无法或无需产出实现计划时的约定标记：同样教成对标签、代码按前缀判据识别
        assert!(
            PLAN_NUDGE_TEXT.contains(&format!("{PLAN_UNACHIEVABLE_MARKER}>")),
            "必须与模型约定「无法/无需计划」标签，否则事实问题会被逼重给方案：{PLAN_NUDGE_TEXT}"
        );
        assert!(
            is_plan_unachievable(PLAN_NUDGE_TEXT),
            "提醒文本自身就带标记，前缀判据必须命中：{PLAN_NUDGE_TEXT}"
        );
        // 排除式判定：先判两种「不需要给方案」的情况，都不成立才必须给方案；保持现状/无需改动
        // 属于评估结论、应走 proposed_plan，因此提醒里不出现在 unachievable 的分支描述中
        assert!(
            PLAN_NUDGE_TEXT.contains("先判断下面两种不需要给方案的情况是否成立"),
            "必须让模型先排除不方案情形：{PLAN_NUDGE_TEXT}"
        );
        assert!(
            PLAN_NUDGE_TEXT.contains("都必须给出完整方案") ||
                PLAN_NUDGE_TEXT.contains("必须想办法把完整方案"),
            "排除两种后必须给方案：{PLAN_NUDGE_TEXT}"
        );
        assert!(
            !PLAN_NUDGE_TEXT.contains("请明确提出问题"),
            "已删除「需要先确认信息」的自由文本出口：{PLAN_NUDGE_TEXT}"
        );
        assert!(
            !PLAN_NUDGE_TEXT.contains("说不用改了"),
            "「不用改了」是认可既有计划的收尾，不属于放弃：{PLAN_NUDGE_TEXT}"
        );
        assert!(!PLAN_NUDGE_TEXT.contains("必须实际调用工具"));

        // 改名后**任何教学面都不许再出现旧的无前缀标签**（防半改）
        for legacy in ["<cancelled_plan", "<unachievable_plan", "<task_completed"] {
            for text in [
                NUDGE_TEXT,
                PLAN_NUDGE_TEXT,
                DEFAULT_MODE_CONTRACT_TEXT,
                PLAN_MODE_CONTRACT_TEXT,
            ] {
                assert!(!text.contains(legacy), "残留旧标签字面量「{legacy}」：{text}");
            }
        }

        // 首轮教学的两段契约：教新标签、与判据强耦合、结构不变量写全
        assert!(DEFAULT_MODE_CONTRACT_TEXT.contains(&format!("{TASK_COMPLETED_MARKER}>")));
        assert!(DEFAULT_MODE_CONTRACT_TEXT.contains("</zen_task_completed>"));
        assert!(is_task_completed(DEFAULT_MODE_CONTRACT_TEXT));
        assert!(!DEFAULT_MODE_CONTRACT_TEXT.contains(PLAN_CANCEL_MARKER));
        assert!(!DEFAULT_MODE_CONTRACT_TEXT.contains(PLAN_UNACHIEVABLE_MARKER));
        for needle in [
            "成对闭合",
            "独占一行",
            "不换行",
            "本轮最多只写一个",
            "不要放进代码块",
        ] {
            assert!(
                DEFAULT_MODE_CONTRACT_TEXT.contains(needle),
                "默认模式契约缺少结构约定「{needle}」：{DEFAULT_MODE_CONTRACT_TEXT}"
            );
            assert!(
                PLAN_MODE_CONTRACT_TEXT.contains(needle),
                "计划模式契约缺少结构约定「{needle}」：{PLAN_MODE_CONTRACT_TEXT}"
            );
        }
        // 计划模式契约：两个出口标签 + 边界（不算 unachievable 的评估结论要走 proposed_plan）
        assert!(PLAN_MODE_CONTRACT_TEXT.contains(&format!("{PLAN_CANCEL_MARKER}>")));
        assert!(PLAN_MODE_CONTRACT_TEXT.contains("</zen_plan_cancelled>"));
        assert!(is_plan_cancelled(PLAN_MODE_CONTRACT_TEXT));
        assert!(PLAN_MODE_CONTRACT_TEXT.contains(&format!("{PLAN_UNACHIEVABLE_MARKER}>")));
        assert!(PLAN_MODE_CONTRACT_TEXT.contains("</zen_plan_unachievable>"));
        assert!(is_plan_unachievable(PLAN_MODE_CONTRACT_TEXT));
        assert!(PLAN_MODE_CONTRACT_TEXT.contains("不算 unachievable"));
        assert!(PLAN_MODE_CONTRACT_TEXT.contains(PLAN_OUTPUT_MARKER));
        assert!(!PLAN_MODE_CONTRACT_TEXT.contains(TASK_COMPLETED_MARKER));
        // 首轮教学不前置「必须给完整方案」：强制口径只留在催办提醒里
        assert!(!PLAN_MODE_CONTRACT_TEXT.contains("都必须给出完整方案"));
        assert!(!PLAN_MODE_CONTRACT_TEXT.contains("必须想办法把完整方案"));

        // 简写 input（字符串）也要能续跑
        let body = nudge_continuation_body(&json!({ "input": "hi" }), "text", NUDGE_TEXT);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["text"], "hi");
    }

    #[test]
    fn contract_injection_eligible_only_for_streaming_tool_requests() {
        // 流式 + 声明了工具 + 非会话标题线程：注入首轮教学
        assert!(contract_injection_eligible(true, true, false));
        // 非流式：没有催办路径消费这份契约
        assert!(!contract_injection_eligible(false, true, false));
        // 没有工具声明：与催办同一门槛（压缩/摘要类后台请求通常也不带工具）
        assert!(!contract_injection_eligible(true, false, false));
        // 会话标题线程：只产出标题
        assert!(!contract_injection_eligible(true, true, true));
    }

    #[test]
    fn contract_text_follows_mode() {
        assert_eq!(contract_text(true), PLAN_MODE_CONTRACT_TEXT);
        assert_eq!(contract_text(false), DEFAULT_MODE_CONTRACT_TEXT);
    }

    #[test]
    fn inject_contract_appends_without_touching_prefix() {
        // 有 instructions：原内容逐字保留、契约在末尾（只多一个空行分隔）
        let mut req = json!({ "instructions": "keep me", "input": "hi" });
        inject_contract(&mut req, DEFAULT_MODE_CONTRACT_TEXT);
        assert_eq!(
            req["instructions"],
            format!("keep me\n\n{DEFAULT_MODE_CONTRACT_TEXT}").as_str()
        );
        // 其余字段一字不动
        assert_eq!(req["input"], json!("hi"));
        // 幂等：重复注入不叠加
        inject_contract(&mut req, DEFAULT_MODE_CONTRACT_TEXT);
        assert_eq!(
            req["instructions"],
            format!("keep me\n\n{DEFAULT_MODE_CONTRACT_TEXT}").as_str()
        );

        // 没有 instructions / 空 instructions：契约本身就是 instructions
        let mut req = json!({ "input": "hi" });
        inject_contract(&mut req, PLAN_MODE_CONTRACT_TEXT);
        assert_eq!(req["instructions"], PLAN_MODE_CONTRACT_TEXT);
        let mut req = json!({ "instructions": "", "input": "hi" });
        inject_contract(&mut req, PLAN_MODE_CONTRACT_TEXT);
        assert_eq!(req["instructions"], PLAN_MODE_CONTRACT_TEXT);
    }

    #[test]
    fn inject_contract_keeps_mode_block_verdict() {
        // 契约里没有 `<collaboration_mode>` 块：注入不改变关键词兜底判据的结论
        let mut plan_req = json!({ "instructions": REAL_PLAN_MODE_BLOCK, "input": "hi" });
        assert!(request_is_plan_mode(&plan_req));
        inject_contract(&mut plan_req, PLAN_MODE_CONTRACT_TEXT);
        assert!(
            request_is_plan_mode(&plan_req),
            "注入契约不应把计划模式判成默认模式：{plan_req}"
        );

        let mut default_req = json!({ "instructions": REAL_DEFAULT_MODE_BLOCK, "input": "hi" });
        assert!(!request_is_plan_mode(&default_req));
        inject_contract(&mut default_req, DEFAULT_MODE_CONTRACT_TEXT);
        assert!(
            !request_is_plan_mode(&default_req),
            "注入契约不应把默认模式判成计划模式：{default_req}"
        );
    }

    #[test]
    fn nudge_injection_limit_is_per_request() {
        // 上限常量只有一个旋钮：单请求最多注入 4 次（= 首轮 + 4 次续跑）
        assert_eq!(NUDGE_MAX_INJECTIONS, 4);
    }

    /// 结构剥离：**载荷写什么、是不是教学里要求的固定短词，一律不影响剥离**。
    #[test]
    fn tag_stripper_removes_spans_by_structure_not_payload() {
        let cases = [
            // 教学要求的固定短词
            ("已补好测试。\n<zen_task_completed>已完成</zen_task_completed>"),
            // 不守约定：写了一整句话
            ("已补好测试。\n<zen_task_completed>我判断已经全部完成了，改了 3 个文件</zen_task_completed>"),
            // 不守约定：英文
            ("done\n<zen_task_completed>all good</zen_task_completed>"),
            // 空载荷
            ("已停。\n<zen_plan_cancelled></zen_plan_cancelled>"),
            // 乱码/符号
            ("结论。\n<zen_plan_unachievable>??? 只需查询</zen_plan_unachievable>"),
        ];
        for (case, label) in cases.into_iter().zip([
            "固定短词",
            "一整句话",
            "英文",
            "空载荷",
            "乱码",
        ]) {
            let (visible, payload) = TagStripper::strip_once(case);
            assert!(
                !visible.to_lowercase().contains("<zen_"),
                "{label}：标签必须整段剥离：{visible}"
            );
            assert!(
                !payload.contains("</zen_"),
                "{label}：日志里的载荷不应带上闭标签：{payload}"
            );
        }

        // 整行标签：连行尾换行一起删，不残留空行；正文原样保留
        let (visible, payload) = TagStripper::strip_once(
            "已补好测试。\n<zen_task_completed>我判断已经完成了</zen_task_completed>\n",
        );
        assert_eq!(visible, "已补好测试。\n", "整行标签连换行一起删");
        assert_eq!(payload, "我判断已经完成了", "日志里记实际载荷");

        // 行内标签：只删标签本身，同行其余文本保留
        let (visible, _) =
            TagStripper::strip_once("已完成改动<zen_task_completed>已完成</zen_task_completed>，请查看");
        assert_eq!(visible, "已完成改动，请查看");

        // 大小写不敏感（判据同口径）
        let (visible, _) = TagStripper::strip_once("收尾\n<ZEN_TASK_COMPLETED>DONE</ZEN_TASK_COMPLETED>");
        assert!(!visible.to_lowercase().contains("zen_task_completed"), "{visible}");

        // 一行里多个标签逐个处理
        let (visible, payload) = TagStripper::strip_once(
            "前言\n<zen_plan_cancelled>已放弃</zen_plan_cancelled><zen_task_completed>已完成</zen_task_completed>\n",
        );
        assert!(!visible.to_lowercase().contains("<zen_"), "{visible}");
        assert!(payload.contains("已放弃") && payload.contains("已完成"), "{payload}");
    }

    /// 跨分片、写错闭标签、未闭合这几种弱模型常见形态。
    #[test]
    fn tag_stripper_handles_split_chunks_and_odd_closers() {
        // 逐字符喂：开闭标签都被切碎也不能漏出去
        let raw = "正文\n<zen_task_completed>已完成</zen_task_completed>";
        let mut stripper = TagStripper::new();
        let mut visible = String::new();
        for ch in raw.chars() {
            visible.push_str(&stripper.feed(&ch.to_string()));
        }
        visible.push_str(&stripper.finish());
        assert_eq!(visible, "正文\n");
        assert_eq!(stripper.stripped_note(), "已完成");

        // 弱模型把闭标签写成另一个 zen 标签：照样闭合，不会把后面整段吞掉
        let (visible, payload) = TagStripper::strip_once(
            "正文\n<zen_task_completed>已完成</zen_plan_cancelled>尾部",
        );
        assert_eq!(visible, "正文\n尾部", "容错闭合后保留后续正文：{visible}");
        assert_eq!(payload, "已完成");

        // 未闭合（流结束仍在标签里）：丢掉标签开头到结尾，载荷照样进日志
        let mut stripper = TagStripper::new();
        let mut visible = stripper.feed("正文\n<zen_task_completed>没有闭合");
        visible.push_str(&stripper.finish());
        assert_eq!(visible, "正文\n");
        assert_eq!(stripper.stripped_note(), "没有闭合");

        // 只是「看起来像标签开头」的普通文本：原样下发（不能被吞掉）
        let mut stripper = TagStripper::new();
        let mut visible = stripper.feed("这里有 <zen_ 但不是标签");
        visible.push_str(&stripper.finish());
        assert_eq!(visible, "这里有 <zen_ 但不是标签");
        assert!(stripper.stripped_note().is_empty());

        // 剥掉 `proposed_plan` 之外的正文不受影响（代理不碰 codex 自己的标签）
        let (visible, _) = TagStripper::strip_once(
            "<proposed_plan>\n1. 做 A\n</proposed_plan>\n<zen_task_completed>已完成</zen_task_completed>",
        );
        assert!(visible.contains("<proposed_plan>"), "{visible}");
        assert!(!visible.to_lowercase().contains("zen_task_completed"), "{visible}");
    }

    /// 四个教学面都改成固定短词表，且原有结构不变量与口径仍在。
    #[test]
    fn teaching_texts_use_fixed_short_payloads() {
        for text in [NUDGE_TEXT, DEFAULT_MODE_CONTRACT_TEXT] {
            assert!(
                text.contains("<zen_task_completed>已完成</zen_task_completed>"),
                "默认模式教学应给出固定短词的标签：{text}"
            );
            for word in ["已完成", "无需改动", "已放弃", "做不下去"] {
                assert!(text.contains(word), "缺少固定词「{word}」：{text}");
            }
            assert!(
                text.contains("不要写别的说明"),
                "必须禁止标签里写长说明（省 token）：{text}"
            );
        }
        for text in [PLAN_NUDGE_TEXT, PLAN_MODE_CONTRACT_TEXT] {
            assert!(
                text.contains("<zen_plan_cancelled>已放弃</zen_plan_cancelled>"),
                "计划模式教学应给出固定短词：{text}"
            );
            assert!(
                text.contains("<zen_plan_unachievable>无法计划</zen_plan_unachievable>"),
                "计划模式教学应给出固定短词：{text}"
            );
        }
    }

    fn sse_data_lines(block: &str) -> Vec<String> {
        block
            .lines()
            .filter(|l| l.starts_with("data:"))
            .map(|l| l["data:".len()..].trim().to_string())
            .collect()
    }

    #[test]
    fn passthrough_url_keeps_incoming_path_and_query_verbatim() {
        // 本地 provider 与上游同路径（都是 /zen/v1）→ 原样透传。
        let uri: Uri = "/zen/v1/models?foo=bar".parse().unwrap();
        let url = passthrough_url("https://opencode.ai/zen/v1", &uri).unwrap();
        assert_eq!(url.as_str(), "https://opencode.ai/zen/v1/models?foo=bar");

        // 上游无路径（如 DeepSeek 官方 base_url）→ 入站 /models 原样转发。
        let uri: Uri = "/models".parse().unwrap();
        let url = passthrough_url("https://api.deepseek.com/", &uri).unwrap();
        assert_eq!(url.as_str(), "https://api.deepseek.com/models");

        // 上游带自定义路径、入站同路径（含尾斜杠写法）→ 原样透传。
        let uri: Uri = "/api/v1/chat/completions?x=1".parse().unwrap();
        let url = passthrough_url("https://custom.example.com/api/v1/", &uri).unwrap();
        assert_eq!(
            url.as_str(),
            "https://custom.example.com/api/v1/chat/completions?x=1"
        );

        // 两侧路径不一致（上游 /v1、入站 /health）也不做任何转换，原样发出。
        let uri: Uri = "/health".parse().unwrap();
        let url = passthrough_url("https://example.com/v1", &uri).unwrap();
        assert_eq!(url.as_str(), "https://example.com/health");
    }

    #[test]
    fn normalize_base_url_trims_whitespace_and_trailing_slashes() {
        assert_eq!(normalize_base_url("https://a/"), "https://a");
        assert_eq!(normalize_base_url("https://a"), "https://a");
        assert_eq!(normalize_base_url("https://a///"), "https://a");
        assert_eq!(
            normalize_base_url("  https://a/zen/v2/  "),
            "https://a/zen/v2"
        );
        assert_eq!(normalize_base_url(""), DEFAULT_ZEN_BASE_URL);
        assert_eq!(normalize_base_url("   "), DEFAULT_ZEN_BASE_URL);
        assert_eq!(normalize_base_url("/"), DEFAULT_ZEN_BASE_URL);
    }

    #[test]
    fn responses_path_follows_base_url_path() {
        assert_eq!(responses_path("https://api.deepseek.com"), "/responses");
        assert_eq!(responses_path("https://api.deepseek.com/"), "/responses");
        assert_eq!(responses_path("https://opencode.ai/zen/v1"), "/zen/v1/responses");
        assert_eq!(responses_path("https://opencode.ai/zen/v2/"), "/zen/v2/responses");
        assert_eq!(responses_path("https://a/v1"), "/v1/responses");
        // 空/非法地址回退默认上游，派生出默认路径。
        assert_eq!(
            responses_path(""),
            responses_path(DEFAULT_ZEN_BASE_URL)
        );
    }

    #[test]
    fn passthrough_url_rejects_invalid_base() {
        let uri: Uri = "/v1/models".parse().unwrap();
        assert!(passthrough_url("not a url", &uri).is_err());
    }

    #[test]
    fn responses_to_chat_string_input() {
        let req = json!({
            "model": "zen/gpt-5-mini",
            "input": "你好",
            "stream": false
        });
        let (chat, repairs) = responses_to_chat(&req, false, false).unwrap();
        assert_eq!(repairs, RepairReport::default(), "纯文本历史不应触发净化");
        assert_eq!(chat["model"], "zen/gpt-5-mini");
        assert_eq!(chat["stream"], false);
        assert_eq!(chat["messages"][0]["role"], "user");
        assert_eq!(chat["messages"][0]["content"], "你好");
    }

    #[test]
    fn responses_to_chat_maps_developer_role_to_system() {
        // Responses 的 developer 角色在 OpenAI 兼容端点上普遍不被接受（DeepSeek 直接 400）。
        let req = json!({
            "model": "m",
            "instructions": "系统指令",
            "input": [
                { "type": "message", "role": "developer", "content": "开发者指令" },
                { "type": "message", "role": "user", "content": "你好" }
            ]
        });
        let (chat, repairs) = responses_to_chat(&req, false, false).unwrap();
        assert_eq!(repairs, RepairReport::default());
        assert_eq!(chat["messages"][0]["role"], "system");
        assert_eq!(chat["messages"][1]["role"], "system");
        assert_eq!(chat["messages"][1]["content"], "开发者指令");
        assert_eq!(chat["messages"][2]["role"], "user");
        assert_eq!(chat_role("developer"), "system");
        assert_eq!(chat_role("assistant"), "assistant");
        assert_eq!(chat_role("tool"), "tool");
    }

    /// 带思维链回放 + 文本 + 工具调用的历史（codex 的真实形状）。
    fn reasoning_history() -> Value {
        json!({
            "model": "m",
            "input": [
                { "type": "reasoning", "id": "rs_1", "summary": [
                    { "type": "summary_text", "text": "先看代码" }
                ] },
                { "type": "message", "role": "assistant", "content": [
                    { "type": "output_text", "text": "我先看一下" }
                ] },
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "ok" }
            ]
        })
    }

    #[test]
    fn responses_to_chat_omits_reasoning_content_by_default() {
        let (chat, _) = responses_to_chat(&reasoning_history(), false, false).unwrap();
        assert_eq!(chat["messages"][0]["role"], "assistant");
        assert!(chat["messages"][0].get("reasoning_content").is_none());
        assert!(chat["messages"][1].get("tool_calls").is_some());
        assert!(chat["messages"][1].get("reasoning_content").is_none());
    }

    #[test]
    fn responses_to_chat_attaches_reasoning_content_when_required() {
        let (chat, _) = responses_to_chat(&reasoning_history(), false, true).unwrap();
        // 文本消息也带同一轮的思维链：上游声明 tools 时会一路校验到这条进度文本消息
        assert_eq!(chat["messages"][0]["content"], "我先看一下");
        assert_eq!(chat["messages"][0]["reasoning_content"], "先看代码");
        // 工具调用消息带，且内容就是回放的思维链文本
        assert_eq!(chat["messages"][1]["reasoning_content"], "先看代码");
        // 工具结果消息不受影响
        assert_eq!(chat["messages"][2]["role"], "tool");
    }

    #[test]
    fn responses_to_chat_content_message_gets_empty_reasoning_without_item() {
        // 该轮没有回放的 reasoning item：文本消息与工具调用消息都写空串（DeepSeek 实测接受）。
        let req = json!({
            "model": "m",
            "input": [
                { "type": "message", "role": "user", "content": "hi" },
                { "type": "message", "role": "assistant", "content": "我先看一下" },
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "ok" }
            ]
        });
        let (chat, _) = responses_to_chat(&req, false, true).unwrap();
        assert_eq!(chat["messages"][1]["reasoning_content"], "");
        assert_eq!(chat["messages"][2]["reasoning_content"], "");
    }

    #[test]
    fn responses_to_chat_uses_empty_reasoning_content_without_reasoning_item() {
        // 该轮模型没产推理（reasoning_chars=0）→ 回传空串，DeepSeek 同样接受。
        let req = json!({
            "model": "m",
            "input": [
                { "type": "message", "role": "user", "content": "hi" },
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "ok" }
            ]
        });
        let (chat, _) = responses_to_chat(&req, false, true).unwrap();
        assert_eq!(chat["messages"][1]["reasoning_content"], "");
    }

    #[test]
    fn responses_to_chat_does_not_leak_reasoning_across_turns() {
        let req = json!({
            "model": "m",
            "input": [
                { "type": "reasoning", "id": "rs_1", "summary": [
                    { "type": "summary_text", "text": "上一轮的思维链" }
                ] },
                { "type": "message", "role": "assistant", "content": "上一轮的答复" },
                { "type": "message", "role": "user", "content": "下一轮" },
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "ok" }
            ]
        });
        let (chat, _) = responses_to_chat(&req, false, true).unwrap();
        // 该轮的文本消息拿到的仍是本轮思维链
        assert_eq!(chat["messages"][0]["reasoning_content"], "上一轮的思维链");
        // 新轮开始后旧思维链被丢弃 → 空串而不是上一轮的文本
        assert_eq!(chat["messages"][2]["reasoning_content"], "");
    }

    #[test]
    fn responses_to_chat_instructions_and_item_array() {
        let req = json!({
            "model": "m",
            "instructions": "你是一个助手",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "请调用工具" }
                    ]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "search",
                    "arguments": "{\"q\":\"x\"}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": { "result": 1 }
                }
            ],
            "tools": [
                { "type": "function", "name": "search", "description": "搜索", "parameters": { "type": "object" } }
            ],
            "max_output_tokens": 100
        });
        let (chat, repairs) = responses_to_chat(&req, true, false).unwrap();
        assert_eq!(repairs, RepairReport::default(), "配对完整的工具调用不应触发净化");
        assert_eq!(chat["stream"], true);
        assert_eq!(chat["messages"].as_array().unwrap().len(), 4);
        assert_eq!(chat["messages"][0]["role"], "system");
        assert_eq!(chat["messages"][0]["content"], "你是一个助手");
        assert_eq!(chat["messages"][1]["content"], "请调用工具");
        assert_eq!(chat["messages"][2]["role"], "assistant");
        assert_eq!(chat["messages"][2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(chat["messages"][2]["tool_calls"][0]["function"]["name"], "search");
        assert_eq!(chat["messages"][3]["role"], "tool");
        assert_eq!(chat["messages"][3]["tool_call_id"], "call_1");
        assert_eq!(chat["tools"][0]["function"]["name"], "search");
        assert_eq!(chat["max_tokens"], 100);
        // Zen 不支持的字段不注入
        assert!(chat.get("store").is_none());
        assert!(chat.get("reasoning").is_none());
    }

    #[test]
    fn responses_to_chat_ignores_unsupported_fields() {
        let req = json!({
            "model": "m",
            "input": "hi",
            "store": true,
            "reasoning": { "effort": "high" },
            "previous_response_id": "resp_x"
        });
        let (chat, repairs) = responses_to_chat(&req, false, false).unwrap();
        assert_eq!(repairs, RepairReport::default());
        assert!(chat.get("store").is_none());
        assert!(chat.get("reasoning").is_none());
        assert!(chat.get("previous_response_id").is_none());
    }

    #[test]
    fn chat_to_responses_basic() {
        let chat = json!({
            "id": "chatcmpl-abc",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "好的" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        });
        let resp = chat_to_responses(&chat, "zen/gpt-5-mini", &ToolShape::default()).unwrap();
        assert_eq!(resp["status"], "completed");
        assert_eq!(resp["model"], "zen/gpt-5-mini");
        assert_eq!(resp["output"][0]["type"], "message");
        assert_eq!(resp["output"][0]["content"][0]["text"], "好的");
        assert_eq!(resp["usage"]["input_tokens"], 10);
        assert_eq!(resp["usage"]["output_tokens"], 5);
    }

    /// 非流式翻译路径同样按结构剥离三个 zen 标签（一次性，无跨分片问题）。
    #[test]
    fn chat_to_responses_strips_zen_tags() {
        let chat = json!({
            "id": "chatcmpl-strip",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "结论：无需改动。\n<zen_task_completed>无需改动</zen_task_completed>"
                },
                "finish_reason": "stop"
            }]
        });
        let resp = chat_to_responses(&chat, "zen/gpt-5-mini", &ToolShape::default()).unwrap();
        let text = resp["output"][0]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, "结论：无需改动。\n", "{resp}");
        assert!(!text.contains("zen_"), "{resp}");
    }

    #[test]
    fn chat_to_responses_tool_calls() {
        let chat = json!({
            "id": "chatcmpl-xyz",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_9",
                        "type": "function",
                        "function": { "name": "search", "arguments": "{\"q\":1}" }
                    }]
                }
            }]
        });
        let resp = chat_to_responses(&chat, "m", &ToolShape::default()).unwrap();
        assert_eq!(resp["output"].as_array().unwrap().len(), 2);
        assert_eq!(resp["output"][1]["type"], "function_call");
        assert_eq!(resp["output"][1]["call_id"], "call_9");
        assert_eq!(resp["output"][1]["name"], "search");
        assert_eq!(resp["output"][1]["arguments"], "{\"q\":1}");
    }

    #[test]
    fn stream_text_chunks_produce_events() {
        let mut st = StreamState::new("resp_1".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "role": "assistant", "content": "你" }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [{ "delta": { "content": "好" }, "finish_reason": null }]
        });
        let c3 = json!({
            "choices": [{ "delta": {}, "finish_reason": "stop" }]
        });
        let mut lines: Vec<String> = Vec::new();
        lines.extend(process_chunk(&c1, &mut st));
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        // finish_reason 只记录，收尾由流循环在尾部（usage 分片/结束）触发
        lines.extend(finish_stream(&mut st, &None));
        let events: Vec<&str> = lines
            .iter()
            .filter_map(|b| b.lines().next())
            .collect();
        assert_eq!(
            events,
            vec![
                "event: response.output_item.added",
                "event: response.content_part.added",
                "event: response.output_text.delta",
                "event: response.output_text.delta",
                "event: response.output_text.done",
                "event: response.content_part.done",
                "event: response.output_item.done",
                "event: response.completed",
            ]
        );
        // 验证 delta 内容累积
        let deltas: Vec<String> = lines
            .iter()
            .filter(|l| l.contains("response.output_text.delta"))
            .filter_map(|l| {
                let data = l.split("data:").nth(1)?;
                Some(serde_json::from_str::<Value>(data.trim()).ok()?["delta"]
                    .as_str()?
                    .to_string())
            })
            .collect();
        assert_eq!(deltas, vec!["你", "好"]);
    }

    #[test]
    fn stream_tool_calls_produce_function_call_events() {
        let mut st = StreamState::new("resp_2".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_a",
                "type": "function",
                "function": { "name": "search", "arguments": "" }
            }] }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "function": { "arguments": "{\"q\":" }
            }] }, "finish_reason": null }]
        });
        let c3 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "function": { "arguments": "1}" }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines: Vec<String> = Vec::new();
        lines.extend(process_chunk(&c1, &mut st));
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        lines.extend(finish_stream(&mut st, &None));
        let joined = lines.join("\n");
        assert!(joined.contains("event: response.output_item.added"));
        assert!(joined.contains(r#""type":"function_call""#));
        assert!(joined.contains("event: response.function_call_arguments.delta"));
        assert!(joined.contains("event: response.function_call_arguments.done"));
        assert!(joined.contains("event: response.completed"));
        // 参数累积正确
        let done = lines
            .iter()
            .find(|l| l.contains("function_call_arguments.done"))
            .unwrap();
        let data = done.split("data:").nth(1).unwrap().trim();
        let v: Value = serde_json::from_str(data).unwrap();
        assert_eq!(v["arguments"], "{\"q\":1}");
    }

    /// 命名空间工具（codex 用 `{"type":"namespace","tools":[…]}` 声明）：
    /// 上游按扁平名调用，回译必须还原成 codex 期望的 `name` + `namespace`。
    #[test]
    fn namespace_tools_are_flattened_upstream_and_restored_back() {
        let req = json!({
            "model": "m",
            "input": "hi",
            "tools": [
                { "type": "function", "name": "exec_command", "parameters": { "type": "object" } },
                {
                    "type": "namespace",
                    "name": "codexui",
                    "description": "codex-ui 管理工具",
                    "tools": [
                        {
                            "type": "function",
                            "name": "add_scheduled_task",
                            "description": "创建定时任务",
                            "parameters": { "type": "object", "properties": {} }
                        },
                        { "type": "function", "name": "get_usage", "description": "查询用量" }
                    ]
                }
            ]
        });
        // ① 工具声明：命名空间展开成扁平函数，且不再暴露裸命名空间名
        let (chat, _) = responses_to_chat(&req, true, false).unwrap();
        let names: Vec<&str> = chat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["exec_command", "codexui_add_scheduled_task", "codexui_get_usage"],
            "{names:?}"
        );
        assert!(chat["tools"][1]["function"]["description"]
            .as_str()
            .is_some_and(|d| d == "创建定时任务"));
        assert!(chat["tools"][1]["function"]["parameters"]["type"] == "object");

        // ② 工具形态：扁平名 → （命名空间, 子工具名）
        let shape = tool_shape(&req);
        assert_eq!(
            shape.namespaces.get("codexui_add_scheduled_task"),
            Some(&("codexui".to_string(), "add_scheduled_task".to_string()))
        );
        assert_eq!(shape.namespaces.get("exec_command"), None);

        // ③ 回译（流式）：output_item.added / done 都带 name + namespace
        let mut st = StreamState::new("resp_ns".into(), "m".into());
        st.tool_shape = shape.clone();
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_ns",
                "type": "function",
                "function": { "name": "codexui_add_scheduled_task", "arguments": "" }
            }] }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "function": { "arguments": "{\"name\":\"x\",\"prompt\":\"y\",\"cron\":\"0 0 12 * * 1-5\"}" }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines: Vec<String> = Vec::new();
        lines.extend(process_chunk(&c1, &mut st));
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));
        let items: Vec<Value> = lines
            .iter()
            .filter(|l| {
                l.contains("response.output_item.added") || l.contains("response.output_item.done")
            })
            .map(|l| {
                serde_json::from_str::<Value>(l.split("data:").nth(1).unwrap().trim()).unwrap()
            })
            .collect();
        assert_eq!(items.len(), 2, "{items:?}");
        for item in items {
            assert_eq!(item["item"]["name"], "add_scheduled_task");
            assert_eq!(item["item"]["namespace"], "codexui");
        }

        // ④ 回译（非流式）：同一份映射也生效
        let chat_resp = json!({
            "id": "chatcmpl-1",
            "choices": [{ "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_ns",
                    "type": "function",
                    "function": { "name": "codexui_get_usage", "arguments": "{}" }
                }]
            } }]
        });
        let out = chat_to_responses(&chat_resp, "m", &shape).unwrap();
        let call = out["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call")
            .expect("应输出 function_call 项");
        assert_eq!(call["name"], "get_usage");
        assert_eq!(call["namespace"], "codexui");
    }

    /// 命名空间历史回放：codex 回放的是 `name` + `namespace`，上游要看到扁平名。
    #[test]
    fn namespace_history_calls_are_flattened_for_upstream() {
        let req = json!({
            "model": "m",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "生成定时任务" }]
                },
                {
                    "type": "function_call",
                    "name": "add_scheduled_task",
                    "namespace": "codexui",
                    "arguments": "{\"name\":\"x\"}",
                    "call_id": "call_1"
                },
                { "type": "function_call_output", "call_id": "call_1", "output": "已创建" }
            ]
        });
        let (chat, _) = responses_to_chat(&req, true, false).unwrap();
        let calls = &chat["messages"][1]["tool_calls"];
        assert_eq!(calls[0]["function"]["name"], "codexui_add_scheduled_task");
        assert_eq!(calls[0]["id"], "call_1");
        assert_eq!(chat["messages"][2]["role"], "tool");
    }

    /// 命名空间不合法/无子工具时不产生工具；扁平名撞车时只保留一个。
    #[test]
    fn namespace_expansion_skips_empty_and_dedupes_flat_names() {
        let req = json!({
            "model": "m",
            "input": "hi",
            "tools": [
                { "type": "namespace", "name": "empty", "tools": [] },
                {
                    "type": "namespace",
                    "name": "codexui",
                    "tools": [{ "type": "function", "name": "get_usage" }]
                },
                {
                    "type": "namespace",
                    "name": "codexui",
                    "tools": [{ "type": "function", "name": "get_usage" }]
                }
            ]
        });
        let (chat, _) = responses_to_chat(&req, true, false).unwrap();
        let names: Vec<&str> = chat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["codexui_get_usage"], "{names:?}");
    }

    // -----------------------------------------------------------------------
    // 自由格式工具（type:"custom"，如 apply_patch）：单参数声明 + custom_tool_call 回译
    // -----------------------------------------------------------------------

    const PATCH: &str = "*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch\n";

    /// codex 对 freeform 模型的 apply_patch 声明原文（含会误导模型的 FREEFORM 措辞）。
    fn custom_tool_request() -> Value {
        json!({
            "model": "m",
            "input": "hi",
            "tools": [
                {
                    "type": "custom",
                    "name": "apply_patch",
                    "description": "The `apply_patch` tool can be used to edit files. \
                        This is a FREEFORM tool, so do not wrap the patch in JSON.",
                    "format": { "type": "grammar", "syntax": "lark", "definition": "start: ..." }
                }
            ]
        })
    }

    #[test]
    fn custom_tool_is_exposed_as_single_input_function() {
        let (chat, _) = responses_to_chat(&custom_tool_request(), true, false).unwrap();
        let tool = &chat["tools"][0]["function"];
        assert_eq!(chat["tools"][0]["type"], "function");
        assert_eq!(tool["name"], "apply_patch");
        let desc = tool["description"].as_str().unwrap();
        assert!(
            !desc.contains("FREEFORM") && !desc.contains("do not wrap"),
            "描述里不应保留会误导模型的 FREEFORM 提示：{desc}"
        );
        assert!(desc.contains("input"), "{desc}");
        // 语法只存在于 codex 的 `format.definition`（grammar）里，代理不下发 grammar，
        // 因此必须把提炼后的语法规范补进描述，否则弱模型只会自创 hunk 头。
        assert!(desc.contains("*** Begin Patch"), "{desc}");
        assert!(desc.contains("*** End Patch"), "{desc}");
        assert!(desc.contains("@@ <"), "{desc}");
        assert!(desc.contains("Never write `@@ some description @@`"), "{desc}");
        assert!(desc.contains("Edit files (apply_patch patch language)."), "{desc}");
        assert_eq!(tool["parameters"]["properties"]["input"]["type"], "string");
        assert_eq!(tool["parameters"]["required"][0], "input");
        assert!(
            tool["parameters"]["properties"]["input"]["description"]
                .as_str()
                .unwrap()
                .contains("*** Begin Patch"),
            "{tool}"
        );
        // grammar 不下发
        assert!(tool.get("format").is_none());

        let shape = tool_shape(&custom_tool_request());
        assert!(shape.is_custom("apply_patch"));
        assert!(!shape.is_custom("exec_command"));
    }

    /// 非 apply_patch 的自由格式工具：只换基底 + 追加函数调用约定，不塞补丁语法。
    #[test]
    fn other_custom_tools_keep_short_description() {
        let req = json!({
            "model": "m",
            "input": "hi",
            "tools": [
                {
                    "type": "custom",
                    "name": "other_tool",
                    "description": "This is a FREEFORM tool, so do not wrap the input in JSON."
                }
            ]
        });
        let (chat, _) = responses_to_chat(&req, true, false).unwrap();
        let desc = chat["tools"][0]["function"]["description"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(desc.contains("Edit files (other_tool patch language)."), "{desc}");
        assert!(desc.contains("This channel is a function call"), "{desc}");
        assert!(!desc.contains("*** Begin Patch"), "{desc}");
        assert!(!desc.contains("FREEFORM"), "{desc}");
    }

    /// 补丁校验失败：该工具结果后追加格式纠错提示，其它输出逐字不变。
    #[test]
    fn failed_patch_output_gets_format_correction_hint() {
        let req = json!({
            "model": "m",
            "input": [
                { "type": "message", "role": "user", "content": "改个文案" },
                {
                    "type": "custom_tool_call",
                    "call_id": "call_patch",
                    "name": "apply_patch",
                    "input": PATCH
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_patch",
                    "output": "apply_patch verification failed: Failed to find context \
                        'version_too_old field @@' in D:\\repo\\app_server.rs"
                },
                { "type": "function_call", "call_id": "call_exec", "name": "exec_command",
                  "arguments": "{}" },
                { "type": "function_call_output", "call_id": "call_exec",
                  "output": "Exit code: 0" }
            ]
        });
        let (chat, _) = responses_to_chat(&req, true, false).unwrap();
        let messages = chat["messages"].as_array().unwrap();
        let patch = messages
            .iter()
            .find(|m| m["tool_call_id"] == json!("call_patch"))
            .expect("补丁工具结果应在历史里");
        let patch_text = patch["content"].as_str().unwrap();
        assert!(
            patch_text.starts_with("apply_patch verification failed"),
            "原有失败原文必须保留：{patch_text}"
        );
        assert!(patch_text.contains("Never write `@@ some description @@`"), "{patch_text}");
        assert!(patch_text.contains("A hunk header must be"), "{patch_text}");
        // 未失败的普通工具输出逐字不变
        let exec = messages
            .iter()
            .find(|m| m["tool_call_id"] == json!("call_exec"))
            .unwrap();
        assert_eq!(exec["content"], json!("Exit code: 0"));
    }

    /// 补丁失败计数只认自定义工具调用；普通命令输出里恰好出现同样字样时不计。
    #[test]
    fn patch_failure_count_only_counts_custom_patch_failures() {
        let clean = json!({
            "model": "m",
            "input": [
                { "type": "custom_tool_call", "call_id": "c1", "name": "apply_patch", "input": PATCH },
                { "type": "custom_tool_call_output", "call_id": "c1", "output": "Done!" }
            ]
        });
        assert_eq!(patch_failure_count(&clean), 0);

        let failed = json!({
            "model": "m",
            "input": [
                { "type": "custom_tool_call", "call_id": "c1", "name": "apply_patch", "input": PATCH },
                { "type": "custom_tool_call_output", "call_id": "c1",
                  "output": "apply_patch verification failed: Failed to find context 'x @@'" },
                { "type": "custom_tool_call", "call_id": "c2", "name": "apply_patch", "input": PATCH },
                { "type": "custom_tool_call_output", "call_id": "c2",
                  "output": "Invalid Context: line 3" }
            ]
        });
        assert_eq!(patch_failure_count(&failed), 2);

        // 普通工具（命令）输出里出现同样字样：不是补丁调用，不计也不加提示
        let plain = json!({
            "model": "m",
            "input": [
                { "type": "function_call", "call_id": "e1", "name": "exec_command",
                  "arguments": "{}" },
                { "type": "function_call_output", "call_id": "e1",
                  "output": "apply_patch verification failed: Failed to find context 'x @@'" }
            ]
        });
        assert_eq!(patch_failure_count(&plain), 0);
        let (chat, _) = responses_to_chat(&plain, true, false).unwrap();
        assert_eq!(chat["messages"][1]["content"], json!(
            "apply_patch verification failed: Failed to find context 'x @@'"
        ));
    }

    /// 自由格式工具：流式回译成 `custom_tool_call` 事件序列，参数允许非 JSON 原文。
    #[test]
    fn custom_tool_call_streams_custom_events_with_raw_arguments() {
        let mut st = StreamState::new("resp_custom".into(), "m".into());
        st.tool_shape = tool_shape(&custom_tool_request());
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_patch",
                "type": "function",
                "function": { "name": "apply_patch", "arguments": "" }
            }] }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "function": { "arguments": PATCH }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines: Vec<String> = Vec::new();
        lines.extend(process_chunk(&c1, &mut st));
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));
        let joined = lines.join("\n");
        assert!(joined.contains("event: response.output_item.added"), "{joined}");
        assert!(
            joined.contains(r#""type":"custom_tool_call""#),
            "{joined}"
        );
        assert!(
            joined.contains("event: response.custom_tool_call_input.delta"),
            "{joined}"
        );
        assert!(
            joined.contains("event: response.custom_tool_call_input.done"),
            "{joined}"
        );
        assert!(
            !joined.contains("function_call_arguments"),
            "自由格式工具不应发 function_call 事件：{joined}"
        );
        let done = lines
            .iter()
            .find(|l| l.contains("response.custom_tool_call_input.done"))
            .unwrap();
        let data = done.split("data:").nth(1).unwrap().trim();
        let v: Value = serde_json::from_str(data).unwrap();
        assert_eq!(v["input"], PATCH);
        // 原始（非 JSON）参数不再判畸形，本轮照常收尾
        assert!(joined.contains("event: response.completed"), "{joined}");
        assert!(!joined.contains("event: response.failed"), "{joined}");
    }

    /// 模型把补丁包进 `{"input": …}` 时也要取出该字段。
    #[test]
    fn custom_tool_call_accepts_json_input_wrapper() {
        let mut st = StreamState::new("resp_custom_json".into(), "m".into());
        st.tool_shape = tool_shape(&custom_tool_request());
        let args = json!({ "input": PATCH }).to_string();
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_patch",
                "type": "function",
                "function": { "name": "apply_patch", "arguments": args }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(finish_stream(&mut st, &None));
        let joined = lines.join("\n");
        let done = lines
            .iter()
            .find(|l| l.contains("response.custom_tool_call_input.done"))
            .unwrap();
        let v: Value =
            serde_json::from_str(done.split("data:").nth(1).unwrap().trim()).unwrap();
        assert_eq!(v["input"], PATCH);
        assert!(!joined.contains("event: response.failed"), "{joined}");
    }

    /// 自由格式工具内容为空时仍按畸形处理（避免把空补丁交给 codex）。
    #[test]
    fn custom_tool_call_with_empty_arguments_is_malformed() {
        let mut st = StreamState::new("resp_custom_empty".into(), "m".into());
        st.tool_shape = tool_shape(&custom_tool_request());
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_patch",
                "type": "function",
                "function": { "name": "apply_patch", "arguments": "" }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(finish_stream(&mut st, &None));
        let joined = lines.join("\n");
        assert!(joined.contains("event: response.failed"), "{joined}");
        assert!(joined.contains(CODE_MALFORMED_TOOL_CALL), "{joined}");
    }

    /// 非流式：自由格式工具同样回译成 `custom_tool_call`。
    #[test]
    fn chat_to_responses_emits_custom_tool_call() {
        let chat = json!({
            "id": "chatcmpl-2",
            "choices": [{ "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_patch",
                    "type": "function",
                    "function": { "name": "apply_patch", "arguments": PATCH }
                }]
            } }]
        });
        let out = chat_to_responses(&chat, "m", &tool_shape(&custom_tool_request())).unwrap();
        let call = out["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "custom_tool_call")
            .expect("应输出 custom_tool_call");
        assert_eq!(call["name"], "apply_patch");
        assert_eq!(call["input"], PATCH);
        assert_eq!(call["call_id"], "call_patch");
    }

    /// 历史回放：`custom_tool_call` / `custom_tool_call_output` 不再丢失。
    #[test]
    fn custom_tool_history_is_replayed() {
        let req = json!({
            "model": "m",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "改个文件" }]
                },
                {
                    "type": "custom_tool_call",
                    "name": "apply_patch",
                    "call_id": "call_patch",
                    "input": PATCH,
                    "status": "completed"
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_patch",
                    "output": "Success. Updated the following files:\nA a.txt\n"
                }
            ]
        });
        let (chat, _) = responses_to_chat(&req, true, false).unwrap();
        let call = &chat["messages"][1]["tool_calls"][0];
        assert_eq!(call["id"], "call_patch");
        assert_eq!(call["function"]["name"], "apply_patch");
        let args: Value =
            serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["input"], PATCH);
        assert_eq!(chat["messages"][2]["role"], "tool");
        assert_eq!(chat["messages"][2]["tool_call_id"], "call_patch");
        assert!(chat["messages"][2]["content"]
            .as_str()
            .unwrap()
            .contains("Updated the following files"));
    }

    #[test]
    fn sse_line_parsing_handles_done_and_data() {
        let lines = sse_data_lines(&sse_event("x", &json!({"a": 1})));
        assert_eq!(lines, vec!["{\"a\":1}"]);
    }

    #[test]
    fn opencode_id_matches_opencode_format() {
        // 前缀 + 26 位：前 12 位是时间戳低 6 字节的小写十六进制（opencode 规则），
        // 后 14 位是 0-9A-Za-z；Zen 服务端就是这么要求的，不合形状会被判成非 opencode 客户端
        for (id, prefix) in [
            (opencode_id("msg", false), "msg_"),
            (opencode_id("ses", true), "ses_"),
        ] {
            let body = id.strip_prefix(prefix).expect("前缀应完整保留");
            assert_eq!(body.len(), 26, "{id}");
            assert!(
                body[..12]
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "前 12 位应是小写十六进制：{id}"
            );
            assert!(
                body.chars().all(|c| c.is_ascii_alphanumeric()),
                "后段只允许 0-9A-Za-z：{id}"
            );
        }
        // 同一毫秒内连续铸号也必须不同（毫秒内计数器参与取值）
        assert_ne!(opencode_id("msg", false), opencode_id("msg", false));
        // 升序与降序是同一时间戳的互补取值：首字节必然不同（会话用降序、消息用升序）
        let ascending = opencode_id("x", false);
        let descending = opencode_id("x", true);
        assert_ne!(&ascending[2..14], &descending[2..14]);
    }

    #[test]
    fn opencode_session_maps_client_header_to_stable_id() {
        let state = test_proxy_state("ses_fixed123", "http://127.0.0.1:1", None, TraceSink::disabled());
        let mut headers = HeaderMap::new();
        headers.insert(
            "session-id",
            HeaderValue::from_static("3f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8"),
        );
        // UUID 形状的 codex 线程 id 不再直接贴进头里，而是映射成合法 opencode 会话标识
        let session = opencode_session(&state, &headers);
        assert_is_opencode_id(&session, "ses");
        // 同一 codex 线程恒定
        assert_eq!(opencode_session(&state, &headers), session);
        // 反查能找回原始 codex 线程 id
        assert_eq!(
            state.session_map.codex_of(&session).as_deref(),
            Some("3f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8")
        );

        // 另一个线程 → 另一个会话 id
        let mut other = HeaderMap::new();
        other.insert("session-id", HeaderValue::from_static("3f1a2b3c"));
        assert_ne!(opencode_session(&state, &other), session);

        // 前后空白与 `ses_` 前缀归一化到同一条映射
        let mut prefixed = HeaderMap::new();
        prefixed.insert(
            "session-id",
            HeaderValue::from_static(" ses_3f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8 "),
        );
        assert_eq!(opencode_session(&state, &prefixed), session);

        // 缺失 / 纯空白 / 非可见 ASCII：回落到代理级稳定会话，且查不到对应线程
        assert_eq!(opencode_session(&state, &HeaderMap::new()), "ses_fixed123");
        let mut blank = HeaderMap::new();
        blank.insert("session-id", HeaderValue::from_static("   "));
        assert_eq!(opencode_session(&state, &blank), "ses_fixed123");
        let mut invalid = HeaderMap::new();
        invalid.insert(
            "session-id",
            HeaderValue::from_bytes(b"caf\xe9").unwrap(),
        );
        assert_eq!(opencode_session(&state, &invalid), "ses_fixed123");
        assert_eq!(state.session_map.codex_of("ses_fixed123"), None);
    }

    #[test]
    fn session_map_is_deterministic_and_reversible() {
        let map = SessionMap::default();
        let a = map.resolve("thread-a").expect("应铸出会话 id");
        let b = map.resolve("thread-b").expect("应铸出会话 id");
        assert_is_opencode_id(&a, "ses");
        assert_is_opencode_id(&b, "ses");
        assert_ne!(a, b);
        // 同 key 恒定、双向可查
        assert_eq!(map.resolve("thread-a").as_deref(), Some(a.as_str()));
        assert_eq!(map.codex_of(&a).as_deref(), Some("thread-a"));
        assert_eq!(map.codex_of(&b).as_deref(), Some("thread-b"));
        assert_eq!(map.codex_of("ses_unknown"), None);
    }

    #[test]
    fn session_map_normalizes_key_and_ignores_blank() {
        let map = SessionMap::default();
        let a = map.resolve("thread-a").expect("应铸出会话 id");
        // trim + 剥 `ses_` 前缀后落到同一条映射
        assert_eq!(map.resolve("  ses_thread-a  ").as_deref(), Some(a.as_str()));
        assert!(map.resolve("   ").is_none());
        assert!(map.resolve("ses_").is_none());
    }

    #[test]
    fn session_map_clears_when_full() {
        let map = SessionMap::default();
        let keep = map.resolve("thread-0").expect("应铸出会话 id");
        for index in 1..SESSION_MAP_LIMIT {
            map.resolve(&format!("thread-{index}")).expect("应铸出会话 id");
        }
        assert_eq!(map.codex_of(&keep).as_deref(), Some("thread-0"));
        // 超出上限：整体清空后插入（与 ThreadModeRegistry 同一取舍）
        let overflow = map.resolve("thread-overflow").expect("应铸出会话 id");
        assert_eq!(map.codex_of(&overflow).as_deref(), Some("thread-overflow"));
        assert_eq!(map.codex_of(&keep), None);
    }

    #[test]
    fn session_map_remints_when_id_collides() {
        let map = SessionMap::default();
        let taken = map.resolve("thread-a").expect("应铸出会话 id");
        let mut calls = 0usize;
        let other = map
            .resolve_with("thread-b", || {
                calls += 1;
                if calls <= 2 {
                    taken.clone()
                } else {
                    "ses_fresh".to_string()
                }
            })
            .expect("应铸出会话 id");
        assert_eq!(other, "ses_fresh");
        assert_eq!(calls, 3, "前两次撞号应重铸");
        // 撞号重铸不影响原会话的映射
        assert_eq!(map.codex_of(&taken).as_deref(), Some("thread-a"));
        assert_eq!(map.resolve("thread-a").as_deref(), Some(taken.as_str()));
    }

    #[test]
    fn log_at_writes_safe_events_to_session_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        log_at(
            &log,
            "info",
            "zen_proxy.request",
            &[
                ("model", "zen/gpt-5-mini".to_string()),
                ("stream", "true".to_string()),
            ],
        );
        log_at(
            &log,
            "warn",
            "zen_proxy.forward_error",
            &[("error", "上游请求失败：TLS".to_string())],
        );

        let files = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>();
        let joined = files.join("\n");
        assert!(joined.contains("event=zen_proxy.request"));
        assert!(joined.contains("event=zen_proxy.forward_error"));
        assert!(joined.contains("model=zen/gpt-5-mini"));
        // 安全字段：不应包含 Authorization / 敏感内容字样
        assert!(!joined.contains("Authorization"));
        assert!(!joined.contains("Bearer"));
    }

    #[test]
    fn log_at_none_or_bad_dir_is_silent() {
        // 无句柄：不 panic
        log_at(&None, "info", "zen_proxy.request", &[]);
        // 日志目录不可写（路径指向一个普通文件）：静默不 panic
        let dir = tempfile::TempDir::new().unwrap();
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, b"x").unwrap();
        let log = Some(Arc::new(SessionLog::new(blocker)));
        log_at(&log, "info", "zen_proxy.request", &[]);
    }

    /// 从 SSE 块里取事件名序列。
    fn event_names(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .filter_map(|l| l.lines().next())
            .map(|l| l.trim_start_matches("event: ").to_string())
            .collect()
    }

    /// 取指定事件的 data 载荷。
    fn event_payload(lines: &[String], name: &str) -> Option<Value> {
        lines.iter().find_map(|l| {
            let mut it = l.lines();
            let head = it.next()?;
            if head.trim_start_matches("event: ") != name {
                return None;
            }
            let data = it.next()?.trim_start_matches("data:").trim();
            serde_json::from_str::<Value>(data).ok()
        })
    }

    #[test]
    fn malformed_tool_arguments_fail_the_stream() {
        let mut st = StreamState::new("resp_bad".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "content": "我直接改文件" }, "finish_reason": null }]
        });
        // 真实故障形态：参数被截断
        let c2 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_bad",
                "type": "function",
                "function": { "name": "apply_patch", "arguments": "{\"old_string\": " }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let names = event_names(&lines);
        assert!(names.contains(&"response.failed".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
        // 坏调用不得以完成态进入会话：`output_item.added` 是增量协议里先发出去的，
        // 但参数完成事件与 function_call 的 `output_item.done` 都不得发出。
        assert!(!names.contains(&"response.function_call_arguments.done".to_string()));
        assert!(!lines
            .iter()
            .any(|l| l.contains("output_item.done") && l.contains("\"type\":\"function_call\"")));
        // 文本照常收尾：用户仍能看到模型已产出的话
        assert!(lines
            .iter()
            .any(|l| l.contains("output_item.done") && l.contains("\"type\":\"message\"")));

        let failed = event_payload(&lines, "response.failed").expect("应发出 response.failed");
        assert_eq!(failed["response"]["status"], "failed");
        assert_eq!(failed["response"]["error"]["code"], CODE_MALFORMED_TOOL_CALL);
        assert!(failed["response"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("apply_patch"));
    }

    #[test]
    fn empty_tool_arguments_still_complete() {
        let mut st = StreamState::new("resp_noargs".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call_a",
                "type": "function",
                "function": { "name": "get_time", "arguments": "" }
            }] }, "finish_reason": "tool_calls" }]
        });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(finish_stream(&mut st, &None));
        let names = event_names(&lines);
        assert!(names.contains(&"response.completed".to_string()));
        assert!(!names.contains(&"response.failed".to_string()));
        assert!(names.contains(&"response.function_call_arguments.done".to_string()));
    }

    #[test]
    fn stream_error_payload_fails_the_stream() {
        let mut st = StreamState::new("resp_err".into(), "m".into());
        let chunk = json!({ "error": { "message": "Internal server error", "type": "error" } });
        assert!(process_chunk(&chunk, &mut st).is_empty());
        assert!(st.failure.is_some(), "流内 error 应被记为失败");

        let lines = finish_stream(&mut st, &None);
        let names = event_names(&lines);
        assert!(names.contains(&"response.failed".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
        let failed = event_payload(&lines, "response.failed").unwrap();
        assert_eq!(failed["response"]["error"]["code"], CODE_UPSTREAM_STREAM_ERROR);
        assert!(failed["response"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Internal server error"));
    }

    #[test]
    fn stream_usage_is_mapped_into_completed() {
        let mut st = StreamState::new("resp_usage".into(), "m".into());
        let c1 = json!({
            "choices": [{ "delta": { "content": "好" }, "finish_reason": null }]
        });
        let c2 = json!({
            "choices": [],
            "usage": { "prompt_tokens": 120000, "completion_tokens": 30, "total_tokens": 120030 }
        });
        let c3 = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let done = event_payload(&lines, "response.completed").expect("应正常完成");
        assert_eq!(done["response"]["usage"]["input_tokens"], 120000);
        assert_eq!(done["response"]["usage"]["output_tokens"], 30);
        assert_eq!(done["response"]["usage"]["total_tokens"], 120030);
    }

    #[test]
    fn responses_to_chat_repairs_invalid_arguments() {
        let req = json!({
            "model": "m",
            "input": [
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "apply_patch",
                    "arguments": "{\"old_string\": "
                }
            ]
        });
        let (chat, repairs) = responses_to_chat(&req, true, false).unwrap();
        assert_eq!(repairs.invalid_arguments, 1);
        assert_eq!(
            chat["messages"][0]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
    }

    #[test]
    fn responses_to_chat_synthesizes_missing_tool_output() {
        let req = json!({
            "model": "m",
            "input": [
                {
                    "type": "function_call",
                    "call_id": "call_9",
                    "name": "apply_patch",
                    "arguments": "{\"a\":1}"
                }
            ]
        });
        let (chat, repairs) = responses_to_chat(&req, true, false).unwrap();
        assert_eq!(repairs.missing_tool_outputs, 1);
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_9");
        assert_eq!(messages[1]["content"], MISSING_TOOL_OUTPUT_TEXT);
    }

    #[test]
    fn chat_to_responses_rejects_invalid_arguments() {
        let chat = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_bad",
                        "type": "function",
                        "function": { "name": "apply_patch", "arguments": "{\"old_string\": " }
                    }]
                }
            }]
        });
        let err = chat_to_responses(&chat, "m", &ToolShape::default()).unwrap_err();
        assert!(err.contains("apply_patch"), "错误信息应含工具名：{err}");
    }

    #[test]
    fn optional_field_rejection_is_detected_by_parameter_name() {
        let stream_options = ["stream_options", "include_usage"];
        assert!(mentions_field(
            "{\"error\":{\"message\":\"Unrecognized request argument supplied: stream_options\"}}",
            &stream_options
        ));
        assert!(mentions_field(
            "include_usage is not supported by this model",
            &stream_options
        ));
        // 普通 4xx 不应触发降级重试
        assert!(!mentions_field(
            "{\"error\":{\"message\":\"Assistant tool call function.arguments must be valid JSON.\"}}",
            &stream_options
        ));
        // 推理强度被指名时的关键词匹配
        assert!(mentions_field(
            "Unknown parameter: 'reasoning_effort'.",
            &["reasoning_effort", "reasoning effort"]
        ));
    }

    #[test]
    fn request_log_records_reasoning_effort() {
        let fields = request_log_fields(
            &json!({
                "model": "m",
                "stream": true,
                "instructions": "提示词",
                "input": [{ "type": "message" }],
                "tools": [],
                "reasoning": { "effort": "high", "summary": "auto" }
            }),
            true,
            "req_test",
            "default",
            MODE_SRC_HEURISTIC,
            "default",
        );
        let value_of = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(value_of("reasoning_effort"), Some("high".to_string()));
        assert_eq!(value_of("model"), Some("m".to_string()));
        assert_eq!(value_of("stream"), Some("true".to_string()));
        assert_eq!(value_of("input_msg_count"), Some("1".to_string()));
        assert_eq!(value_of("tool_count"), Some("0".to_string()));
        assert_eq!(value_of("instructions_chars"), Some("3".to_string()));
        assert_eq!(value_of("patch_failures"), Some("0".to_string()));
        assert!(value_of("input_chars").is_some());
        assert_eq!(value_of("call_id"), Some("req_test".to_string()));
        // 协作模式与来源由调用方（`resolve_nudge_mode`）解析后传入，这里只记录
        assert_eq!(value_of("模式"), Some("default".to_string()));
        assert_eq!(value_of("模式来源"), Some(MODE_SRC_HEURISTIC.to_string()));
        // 首轮教学的注入结论同样进这一行日志
        assert_eq!(value_of("首轮教学"), Some("default".to_string()));
        let plan_fields = request_log_fields(
            &json!({
                "model": "m",
                "input": [
                    { "type": "message", "role": "developer", "content": [{
                        "type": "input_text", "text": REAL_PLAN_MODE_BLOCK
                    }] }
                ]
            }),
            true,
            "req_test",
            "plan",
            MODE_SRC_REGISTRY,
            "off",
        );
        assert_eq!(
            plan_fields
                .iter()
                .find(|(name, _)| *name == "模式")
                .map(|(_, value)| value.as_str()),
            Some("plan")
        );
        assert_eq!(
            plan_fields
                .iter()
                .find(|(name, _)| *name == "模式来源")
                .map(|(_, value)| value.as_str()),
            Some(MODE_SRC_REGISTRY)
        );
        assert_eq!(
            plan_fields
                .iter()
                .find(|(name, _)| *name == "首轮教学")
                .map(|(_, value)| value.as_str()),
            Some("off")
        );

        // 没请求推理时记 `-`
        let fields = request_log_fields(
            &json!({ "model": "m" }),
            false,
            "req_test",
            "default",
            MODE_SRC_HEURISTIC,
            "off",
        );
        assert_eq!(
            fields
                .iter()
                .find(|(name, _)| *name == "reasoning_effort")
                .map(|(_, value)| value.as_str()),
            Some("-")
        );

        // 历史里有失败的补丁调用：记条数（模型正在补丁格式上打转）
        let fields = request_log_fields(
            &json!({
                "model": "m",
                "input": [
                    { "type": "custom_tool_call", "call_id": "c1", "name": "apply_patch",
                      "input": "*** Begin Patch\n*** End Patch\n" },
                    { "type": "custom_tool_call_output", "call_id": "c1",
                      "output": "apply_patch verification failed: Failed to find context 'x @@'" }
                ]
            }),
            true,
            "req_test",
            "default",
            MODE_SRC_HEURISTIC,
            "off",
        );
        assert_eq!(
            fields
                .iter()
                .find(|(name, _)| *name == "patch_failures")
                .map(|(_, value)| value.as_str()),
            Some("1")
        );
    }

    #[test]
    fn usage_details_are_mapped_into_responses() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 40,
            "total_tokens": 140,
            "prompt_tokens_details": { "cached_tokens": 80, "cache_write_tokens": 20 },
            "completion_tokens_details": { "reasoning_tokens": 12 }
        });
        let out = usage_to_responses(Some(&usage));
        assert_eq!(out["input_tokens"], 100);
        assert_eq!(out["output_tokens"], 40);
        assert_eq!(out["total_tokens"], 140);
        assert_eq!(out["input_tokens_details"]["cached_tokens"], 80);
        assert_eq!(out["input_tokens_details"]["cache_write_tokens"], 20);
        assert_eq!(out["output_tokens_details"]["reasoning_tokens"], 12);
    }

    #[test]
    fn usage_details_fall_back_to_top_level_fields() {
        let usage = json!({
            "prompt_tokens": 10,
            "completion_tokens": 2,
            "cache_read_input_tokens": 6,
            "cache_creation_input_tokens": 4,
            "reasoning_tokens": 1
        });
        let out = usage_to_responses(Some(&usage));
        assert_eq!(out["input_tokens_details"]["cached_tokens"], 6);
        assert_eq!(out["input_tokens_details"]["cache_write_tokens"], 4);
        assert_eq!(out["output_tokens_details"]["reasoning_tokens"], 1);
    }

    #[test]
    fn usage_without_details_omits_detail_objects() {
        let out = usage_to_responses(Some(&json!({
            "prompt_tokens": 5,
            "completion_tokens": 1,
            "total_tokens": 6
        })));
        assert!(out.get("input_tokens_details").is_none());
        assert!(out.get("output_tokens_details").is_none());
        // null / 非数值都不算命中
        let out = usage_to_responses(Some(&json!({
            "prompt_tokens_details": { "cached_tokens": null },
            "completion_tokens_details": { "reasoning_tokens": "7" },
            "cache_write_tokens": null
        })));
        assert!(out.get("input_tokens_details").is_none());
        assert!(out.get("output_tokens_details").is_none());
    }

    #[test]
    fn optional_fields_map_reasoning_and_passthrough_fields() {
        let req = json!({
            "model": "m",
            "reasoning": { "effort": "high", "summary": "auto" },
            "parallel_tool_calls": false,
            "prompt_cache_key": "cache-key",
            "service_tier": "flex"
        });
        let fields = optional_fields(&req, true);
        let value_of = |key: &str| {
            fields
                .iter()
                .find(|field| field.key == key)
                .map(|field| field.value.clone())
        };
        assert_eq!(value_of("stream_options"), Some(json!({ "include_usage": true })));
        assert_eq!(value_of("reasoning_effort"), Some(json!("high")));
        assert_eq!(value_of("parallel_tool_calls"), Some(json!(false)));
        assert_eq!(value_of("prompt_cache_key"), Some(json!("cache-key")));
        assert_eq!(value_of("service_tier"), Some(json!("flex")));
        // 非流式不带 stream_options；未给可选字段时列表为空
        assert!(optional_fields(&req, false)
            .iter()
            .all(|field| field.key != "stream_options"));
        assert!(optional_fields(&json!({ "model": "m" }), false).is_empty());
    }

    #[test]
    fn tool_choice_converts_only_known_shapes() {
        assert_eq!(
            tool_choice_to_chat(Some(&json!({ "type": "function", "name": "search" }))),
            Some(json!({ "type": "function", "function": { "name": "search" } }))
        );
        assert_eq!(
            tool_choice_to_chat(Some(&json!("required"))),
            Some(json!("required"))
        );
        // 无 chat 等价物的形态与空值一律丢弃
        assert_eq!(tool_choice_to_chat(Some(&json!({ "type": "allowed_tools" }))), None);
        assert_eq!(tool_choice_to_chat(Some(&Value::Null)), None);
        assert_eq!(tool_choice_to_chat(None), None);
    }

    // ---------- Zen 免费层请求体门禁：形状补丁 ----------

    #[test]
    fn is_zen_upstream_matches_only_opencode_hosts() {
        // 默认上游与 Zen 的其它路径都命中（host 判定，与路径无关）
        assert!(is_zen_upstream("https://opencode.ai/zen/v1"));
        assert!(is_zen_upstream("https://opencode.ai"));
        assert!(is_zen_upstream("https://api.opencode.ai/zen/v2/"));
        assert!(is_zen_upstream("HTTPS://OpenCode.AI/zen/v1"));
        // 本地 mock / 第三方上游 / 伪装域名 / 非法地址一律不补形状
        assert!(!is_zen_upstream("http://127.0.0.1:18080/zen/v1"));
        assert!(!is_zen_upstream("https://opencode.ai.evil.com/v1"));
        assert!(!is_zen_upstream("https://api.deepseek.com/v1"));
        assert!(!is_zen_upstream("https://10.0.0.20:9080/v1"));
        assert!(!is_zen_upstream(""));
        assert!(!is_zen_upstream("open"));
        assert!(!is_zen_upstream("opencode.ai/zen/v1"));
    }

    #[test]
    fn patch_zen_request_body_appends_missing_required_tools() {
        // 原本没有 tools：数组只含 6 个假工具，形状逐字可核对
        let mut body = json!({ "model": "m", "messages": [], "stream": true });
        patch_zen_request_body(&mut body);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), ZEN_REQUIRED_TOOL_NAMES.len());
        for (tool, name) in tools.iter().zip(ZEN_REQUIRED_TOOL_NAMES) {
            assert_eq!(tool["type"], "function");
            assert_eq!(tool["function"]["name"], name);
            assert_eq!(tool["function"]["description"], ZEN_FAKE_TOOL_DESCRIPTION);
            assert_eq!(
                tool["function"]["parameters"],
                json!({ "type": "object", "properties": {} })
            );
        }
        // 幂等：再调用一次不叠加
        patch_zen_request_body(&mut body);
        assert_eq!(body["tools"].as_array().unwrap().len(), ZEN_REQUIRED_TOOL_NAMES.len());

        // 已有工具：真实工具保留且顺序不变、同名时以客户端声明为准，只补缺失的名字
        let mut body = json!({
            "messages": [],
            "tools": [
                { "type": "function", "function": {
                    "name": "shell", "description": "真实工具" } },
                { "type": "function", "function": {
                    "name": "codexui_glob", "description": "命名空间扁平名" } },
                { "type": "function", "function": {
                    "name": "bash", "description": "客户端自己的 bash" } }
            ]
        });
        patch_zen_request_body(&mut body);
        let tools = body["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["shell", "codexui_glob", "bash", "edit", "glob", "grep", "read", "write"],
            "真实工具在前、缺失的假工具按固定顺序追加在后"
        );
        assert_eq!(tools[2]["function"]["description"], "客户端自己的 bash");
    }

    #[test]
    fn zen_max_tokens_field_follows_client_budget() {
        // 入站没有自己的输出预算 → 补 ZEN_MAX_TOKENS，且错误体点名 max_tokens 时会被摘掉重试
        let field = zen_max_tokens_field(&json!({ "model": "m", "input": "hi" }))
            .expect("无 max_output_tokens 时应补 max_tokens");
        assert_eq!(field.key, "max_tokens");
        assert_eq!(field.value, json!(ZEN_MAX_TOKENS));
        assert!(mentions_field(
            r#"{"error":{"message":"Invalid max_tokens: too large"}}"#,
            field.needles
        ));
        // null 与缺失同口径
        assert!(zen_max_tokens_field(&json!({ "max_output_tokens": null })).is_some());
        // 客户端显式给了预算：尊重它，不补 ZEN_MAX_TOKENS
        assert!(zen_max_tokens_field(&json!({ "max_output_tokens": 100 })).is_none());
        let req = json!({
            "model": "m",
            "input": "hi",
            "max_output_tokens": 100
        });
        let (chat, _) = responses_to_chat(&req, false, false).unwrap();
        assert_eq!(chat["max_tokens"], 100, "客户端预算仍按原样映射");
    }

    #[test]
    fn message_images_become_chat_content_parts() {
        let req = json!({
            "model": "m",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "看这张图" },
                    {
                        "type": "input_image",
                        "image_url": "data:image/png;base64,AAA",
                        "detail": "high"
                    }
                ]
            }]
        });
        let (chat, _) = responses_to_chat(&req, false, false).unwrap();
        let content = &chat["messages"][0]["content"];
        assert_eq!(content[0], json!({ "type": "text", "text": "看这张图" }));
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAA");
        assert_eq!(content[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn unsupported_image_detail_or_missing_url_is_skipped() {
        let req = json!({
            "model": "m",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_image", "image_url": "https://x/y.png", "detail": "original" },
                    { "type": "input_image" }
                ]
            }]
        });
        let (chat, _) = responses_to_chat(&req, false, false).unwrap();
        let content = chat["messages"][0]["content"].as_array().unwrap().clone();
        assert_eq!(content.len(), 1, "无 url 的图片应被丢弃");
        assert_eq!(content[0]["image_url"]["url"], "https://x/y.png");
        assert!(
            content[0]["image_url"].get("detail").is_none(),
            "未知 detail 不写入"
        );
    }

    #[test]
    fn text_only_messages_keep_string_content() {
        let req = json!({
            "model": "m",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "hi" }]
            }]
        });
        let (chat, _) = responses_to_chat(&req, false, false).unwrap();
        assert_eq!(chat["messages"][0]["content"], json!("hi"));
    }

    #[test]
    fn reasoning_deltas_become_reasoning_summary_events() {
        let mut st = StreamState::new("resp_r".into(), "m".into());
        let c1 = json!({ "choices": [{ "delta": { "reasoning_content": "先看" }, "finish_reason": null }] });
        let c2 = json!({ "choices": [{ "delta": { "reasoning_content": "日志" }, "finish_reason": null }] });
        let c3 = json!({ "choices": [{ "delta": { "content": "结论" }, "finish_reason": "stop" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(process_chunk(&c3, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let names = event_names(&lines);
        assert_eq!(names[0], "response.output_item.added", "推理 item 先开");
        assert!(names.contains(&"response.reasoning_summary_part.added".to_string()));
        assert!(names.contains(&"response.reasoning_summary_text.delta".to_string()));
        assert!(names.contains(&"response.reasoning_summary_text.done".to_string()));
        assert!(names.contains(&"response.reasoning_summary_part.done".to_string()));
        assert!(names.contains(&"response.completed".to_string()));

        let delta = event_payload(&lines, "response.reasoning_summary_text.delta").unwrap();
        assert_eq!(delta["summary_index"], 0);
        assert_eq!(delta["delta"], "先看");
        let done = event_payload(&lines, "response.reasoning_summary_text.done").unwrap();
        assert_eq!(done["text"], "先看日志");

        // output_item.done 顺序：推理 item 在前，且带摘要全文
        let done_items: Vec<Value> = lines
            .iter()
            .filter(|line| line.contains("event: response.output_item.done"))
            .filter_map(|line| {
                serde_json::from_str::<Value>(
                    line.lines().nth(1)?.trim_start_matches("data:").trim(),
                )
                .ok()
            })
            .collect();
        assert_eq!(done_items[0]["item"]["type"], "reasoning");
        assert_eq!(done_items[0]["item"]["summary"][0]["text"], "先看日志");
        assert_eq!(done_items[1]["item"]["type"], "message");
    }

    #[test]
    fn reasoning_object_form_is_understood() {
        let mut st = StreamState::new("resp_ro".into(), "m".into());
        let chunk = json!({
            "choices": [{ "delta": { "reasoning": { "content": "想想" } }, "finish_reason": null }]
        });
        let lines = process_chunk(&chunk, &mut st);
        assert!(lines
            .iter()
            .any(|line| line.contains("response.reasoning_summary_text.delta")
                && line.contains("想想")));
    }

    #[test]
    fn finish_reason_length_yields_incomplete() {
        let mut st = StreamState::new("resp_len".into(), "m".into());
        let c1 = json!({ "choices": [{ "delta": { "content": "半截" }, "finish_reason": null }] });
        let c2 = json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));

        let names = event_names(&lines);
        assert!(names.contains(&"response.incomplete".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
        let event = event_payload(&lines, "response.incomplete").unwrap();
        assert_eq!(event["response"]["status"], "incomplete");
        assert_eq!(
            event["response"]["incomplete_details"]["reason"],
            "max_output_tokens"
        );
    }

    #[test]
    fn stream_usage_details_are_forwarded() {
        let mut st = StreamState::new("resp_ud".into(), "m".into());
        let c1 = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 5,
                "total_tokens": 105,
                "prompt_tokens_details": { "cached_tokens": 60 },
                "completion_tokens_details": { "reasoning_tokens": 3 }
            }
        });
        let c2 = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
        let mut lines = process_chunk(&c1, &mut st);
        lines.extend(process_chunk(&c2, &mut st));
        lines.extend(finish_stream(&mut st, &None));
        let done = event_payload(&lines, "response.completed").unwrap();
        assert_eq!(
            done["response"]["usage"]["input_tokens_details"]["cached_tokens"],
            60
        );
        assert_eq!(
            done["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
            3
        );
    }

    #[test]
    fn chat_to_responses_carries_usage_details() {
        let chat = json!({
            "choices": [{ "message": { "role": "assistant", "content": "好" } }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 3,
                "total_tokens": 12,
                "prompt_tokens_details": { "cached_tokens": 4 }
            }
        });
        let resp = chat_to_responses(&chat, "m", &ToolShape::default()).unwrap();
        assert_eq!(resp["usage"]["input_tokens_details"]["cached_tokens"], 4);
        assert_eq!(resp["usage"]["input_tokens"], 9);
    }

    #[test]
    fn failure_takes_precedence_over_incomplete() {
        let mut st = StreamState::new("resp_fi".into(), "m".into());
        st.failure = Some(StreamFailure {
            code: CODE_UPSTREAM_STREAM_ERROR.to_string(),
            message: "boom".to_string(),
            detail: "读取出错".to_string(),
        });
        st.finish_reason = Some("length".to_string());
        let lines = finish_stream(&mut st, &None);
        let names = event_names(&lines);
        assert!(names.contains(&"response.failed".to_string()));
        assert!(!names.contains(&"response.incomplete".to_string()));
        assert!(!names.contains(&"response.completed".to_string()));
    }

    #[test]
    fn finish_reason_alone_does_not_complete_the_stream() {
        let mut st = StreamState::new("resp_ord".into(), "m".into());
        let finish = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
        assert!(
            process_chunk(&finish, &mut st).is_empty(),
            "finish_reason 分片本身不产出事件"
        );
        assert!(
            !st.closed,
            "不应在 finish_reason 处关流，否则会丢掉随后的 usage 分片"
        );

        // OpenAI `include_usage` 的真实形态：finish_reason 之后再发一个只带 usage 的分片
        let usage = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 20,
                "total_tokens": 1020,
                "prompt_tokens_details": { "cached_tokens": 800 },
                "completion_tokens_details": { "reasoning_tokens": 7 }
            }
        });
        assert!(process_chunk(&usage, &mut st).is_empty());

        let lines = finish_stream(&mut st, &None);
        let done = event_payload(&lines, "response.completed").expect("应正常完成");
        assert_eq!(done["response"]["usage"]["input_tokens"], 1000);
        assert_eq!(
            done["response"]["usage"]["input_tokens_details"]["cached_tokens"],
            800
        );
        assert_eq!(
            done["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
            7
        );
    }

    #[test]
    fn stream_summary_reports_usage_presence_and_delta_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let mut st = StreamState::new("resp_diag".into(), "m".into());
        st.usage = Some(json!({ "prompt_tokens": 1 }));
        st.delta_keys.insert("content".to_string());
        st.delta_keys.insert("reasoning_content".to_string());
        let _ = finish_stream(&mut st, &log);

        let joined = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("usage=present"), "{joined}");
        assert!(
            joined.contains("delta_keys=content,reasoning_content"),
            "{joined}"
        );
    }

    /// 收尾摘要与可疑标记都要带上补丁失败条数（用于判断模型是否在补丁格式上打转）。
    #[test]
    fn stream_summary_reports_patch_failures() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let mut st = StreamState::new("resp_patch".into(), "m".into());
        st.usage = Some(json!({ "prompt_tokens": 1 }));
        st.patch_failures = 2;
        let _ = finish_stream(&mut st, &log);

        let joined = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("patch_failures=2"), "{joined}");
        assert!(joined.contains("patch_retry"), "{joined}");
    }

    #[test]
    fn reasoning_details_array_is_understood() {
        let mut st = StreamState::new("resp_rd".into(), "m".into());
        let chunk = json!({
            "choices": [{ "delta": { "reasoning_details": [
                { "type": "reasoning.encrypted", "data": "xxx" },
                { "type": "reasoning.text", "text": "明细推理" }
            ] }, "finish_reason": null }]
        });
        let lines = process_chunk(&chunk, &mut st);
        assert!(lines.iter().any(|line| {
            line.contains("response.reasoning_summary_text.delta") && line.contains("明细推理")
        }));
    }

    #[test]
    fn reasoning_text_and_summary_fields_are_understood() {
        let mut st = StreamState::new("resp_rs".into(), "m".into());
        let text = json!({
            "choices": [{ "delta": { "reasoning": { "text": "来自 text" } }, "finish_reason": null }]
        });
        let summary = json!({
            "choices": [{ "delta": { "reasoning": { "summary": "来自 summary" } }, "finish_reason": null }]
        });
        let mut lines = process_chunk(&text, &mut st);
        lines.extend(process_chunk(&summary, &mut st));
        let joined = lines.join("\n");
        assert!(joined.contains("来自 text"));
        assert!(joined.contains("来自 summary"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::sync::Arc;
    use std::collections::VecDeque;
    use tokio::sync::Mutex as AsyncMutex;

    /// 记录一次上游收到的请求（聊身体 + 识别头）。
    #[derive(Debug, Default, Clone)]
    struct Received {
        body: Value,
        authorization: Option<String>,
        user_agent: Option<String>,
        opencode_client: Option<String>,
        opencode_project: Option<String>,
        opencode_request: Option<String>,
        opencode_session: Option<String>,
    }

    #[derive(Debug, Default, Clone)]
    struct PassthroughReceived {
        method: String,
        path: String,
        query: Option<String>,
        body: Value,
        authorization: Option<String>,
        user_agent: Option<String>,
        opencode_client: Option<String>,
        opencode_project: Option<String>,
        opencode_request: Option<String>,
        opencode_session: Option<String>,
    }

    /// 起一个本地 mock Zen 服务，返回监听地址与接收记录。
    async fn spawn_mock_zen(rec: Arc<AsyncMutex<Option<Received>>>) -> String {
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |headers: HeaderMap, Json(body): Json<Value>| {
                async move {
                    let mut g = rec.lock().await;
                    *g = Some(Received {
                        body,
                        authorization: headers
                            .get(header::AUTHORIZATION)
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        user_agent: headers
                            .get(header::USER_AGENT)
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_client: headers
                            .get("x-opencode-client")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_project: headers
                            .get("x-opencode-project")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_request: headers
                            .get("x-opencode-request")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                        opencode_session: headers
                            .get("x-opencode-session")
                            .and_then(|h| h.to_str().ok())
                            .map(|s| s.to_string()),
                    });
                    Json(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "m",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "mock 回复" },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3 }
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    /// 起一个本地 mock Zen 透传端点，返回带 `/v1` 前缀的 base URL。
    async fn spawn_mock_passthrough(
        rec: Arc<AsyncMutex<Option<PassthroughReceived>>>,
    ) -> String {
        let rec_get = rec.clone();
        let app = Router::new()
            .route(
                "/v1/models",
                axum::routing::get(move |headers: HeaderMap, uri: Uri| {
                    let rec = rec_get.clone();
                    async move {
                        *rec.lock().await = Some(PassthroughReceived {
                            method: "GET".into(),
                            path: uri.path().to_string(),
                            query: uri.query().map(str::to_string),
                            body: Value::Null,
                            authorization: headers
                                .get(header::AUTHORIZATION)
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            user_agent: headers
                                .get(header::USER_AGENT)
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_client: headers
                                .get("x-opencode-client")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_project: headers
                                .get("x-opencode-project")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_request: headers
                                .get("x-opencode-request")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                            opencode_session: headers
                                .get("x-opencode-session")
                                .and_then(|h| h.to_str().ok())
                                .map(str::to_string),
                        });
                        Json(json!({
                            "object": "list",
                            "data": [{ "id": "zen-model" }]
                        }))
                    }
                }),
            )
            .route(
                "/v1/echo",
                axum::routing::post(
                    move |headers: HeaderMap, uri: Uri, Json(body): Json<Value>| {
                        let rec = rec.clone();
                        async move {
                            *rec.lock().await = Some(PassthroughReceived {
                                method: "POST".into(),
                                path: uri.path().to_string(),
                                query: uri.query().map(str::to_string),
                                body: body.clone(),
                                authorization: headers
                                    .get(header::AUTHORIZATION)
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                user_agent: headers
                                    .get(header::USER_AGENT)
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_client: headers
                                    .get("x-opencode-client")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_project: headers
                                    .get("x-opencode-project")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_request: headers
                                    .get("x-opencode-request")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                                opencode_session: headers
                                    .get("x-opencode-session")
                                    .and_then(|h| h.to_str().ok())
                                    .map(str::to_string),
                            });
                            Json(body)
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}/v1")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forwards_chat_with_auth_and_user_agent() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({
            "model": "zen/gpt-5-mini",
            "input": "你好",
            "instructions": "你是助手",
            "stream": false
        });
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer public"),
        );
        let state = proxy_state(base_url, None);
        let resp = forward(&req, &headers, false, &state, "req_test")
            .await
            .expect("forward 应成功");
        assert!(resp.status.is_success());

        // 验证上游收到的内容
        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(got.authorization.as_deref(), Some("Bearer public"));
        assert_eq!(got.user_agent.as_deref(), Some(ZEN_USER_AGENT));
        assert_eq!(got.body["model"], "zen/gpt-5-mini");
        assert_eq!(got.body["messages"][0]["role"], "system");
        assert_eq!(got.body["messages"][1]["role"], "user");
        assert_eq!(got.body["messages"][1]["content"], "你好");
        assert_eq!(got.body["stream"], false);
        // 模拟 opencode 客户端的识别头
        assert_eq!(got.opencode_client.as_deref(), Some(OPENCODE_CLIENT));
        assert_eq!(got.opencode_project.as_deref(), Some(OPENCODE_PROJECT));
        let req_id = got.opencode_request.as_deref().expect("应有请求 id");
        assert_is_opencode_id(req_id, "msg");
        // 客户端没给 session-id：用代理级稳定会话（形状同样合法）
        assert_eq!(got.opencode_session.as_deref(), Some("ses_fixed123"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forwards_chat_prefers_client_session_header() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({ "model": "m", "input": "hi", "stream": false });
        let mut headers = HeaderMap::new();
        headers.insert("session-id", HeaderValue::from_static("3f1a2b3c"));
        let state = proxy_state(base_url, None);
        let resp = forward(&req, &headers, false, &state, "req_test")
            .await
            .expect("forward 应成功");
        assert!(resp.status.is_success());

        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        // 按 codex 会话 id 派生的 opencode 会话标识：ses_ + 26 位 [0-9A-Za-z]
        let session = got.opencode_session.clone().expect("应有会话 id");
        assert_is_opencode_id(&session, "ses");
        // 反查能把实发值映射回 codex 线程 id（诊断用）
        assert_eq!(
            state.session_map.codex_of(&session).as_deref(),
            Some("3f1a2b3c")
        );
        assert_eq!(got.opencode_client.as_deref(), Some(OPENCODE_CLIENT));
        assert_eq!(got.opencode_project.as_deref(), Some(OPENCODE_PROJECT));
        assert_is_opencode_id(
            got.opencode_request.as_deref().expect("应有请求 id"),
            "msg",
        );

        // 同一 codex 会话的后续请求发同一个 x-opencode-session，不同会话则不同
        let resp = forward(&req, &headers, false, &state, "req_test")
            .await
            .expect("forward 应成功");
        assert!(resp.status.is_success());
        let again = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(again.opencode_session.as_deref(), Some(session.as_str()));
        assert_ne!(
            again.opencode_request.as_deref(),
            got.opencode_request.as_deref(),
            "每条上游请求都应换新的 x-opencode-request"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forwards_without_auth_when_client_sends_none() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;
        let req = json!({ "model": "m", "input": "hi", "stream": false });
        let headers = HeaderMap::new();
        let state = proxy_state(base_url, None);
        let resp = forward(&req, &headers, false, &state, "req_test")
            .await
            .expect("forward 应成功");
        assert!(resp.status.is_success());
        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(got.authorization, None);
        assert_eq!(got.user_agent.as_deref(), Some(ZEN_USER_AGENT));
        assert_is_opencode_id(
            got.opencode_request.as_deref().expect("应有请求 id"),
            "msg",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
   async fn passthrough_forwards_models_and_post_body() {
       let rec: Arc<AsyncMutex<Option<PassthroughReceived>>> =
           Arc::new(AsyncMutex::new(None));
       let base_url = spawn_mock_passthrough(rec.clone()).await;
      let state = ProxyState {
          session: "ses_fixed123".into(),
          zen_body_patch: is_zen_upstream(&base_url),
          base_url,
          log: None,
          trace: TraceSink::disabled(),
          requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
          session_map: Arc::new(SessionMap::default()),
           modes: Arc::new(ThreadModeRegistry::default()),
      };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer public"),
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        let uri: Uri = "/v1/models?foo=bar".parse().unwrap();
        let resp =
            forward_passthrough(&state, Method::GET, &uri, &headers, Body::empty()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("zen-model"));
        let got = rec.lock().await.clone().expect("mock 应已收到 GET 请求");
        assert_eq!(got.method, "GET");
        assert_eq!(got.path, "/v1/models");
        assert_eq!(got.query.as_deref(), Some("foo=bar"));
        assert_eq!(got.authorization.as_deref(), Some("Bearer public"));
        assert_eq!(got.user_agent.as_deref(), Some(ZEN_USER_AGENT));
        assert_eq!(got.opencode_client.as_deref(), Some(OPENCODE_CLIENT));
        assert_eq!(got.opencode_project.as_deref(), Some(OPENCODE_PROJECT));
        // 透传路径同样只需要合法形状：没带 session-id 时用代理级稳定会话
        assert_eq!(got.opencode_session.as_deref(), Some("ses_fixed123"));
        assert_is_opencode_id(
            got.opencode_request.as_deref().expect("应有请求 id"),
            "msg",
        );

        let uri: Uri = "/v1/echo".parse().unwrap();
        let resp = forward_passthrough(
            &state,
            Method::POST,
            &uri,
            &headers,
            Body::from(r#"{"a":1}"#),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let got = rec.lock().await.clone().expect("mock 应已收到 POST 请求");
        assert_eq!(got.method, "POST");
        assert_eq!(got.path, "/v1/echo");
        assert_eq!(got.body["a"], 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
   async fn passthrough_prefers_client_session_header() {
       let rec: Arc<AsyncMutex<Option<PassthroughReceived>>> =
           Arc::new(AsyncMutex::new(None));
       let base_url = spawn_mock_passthrough(rec.clone()).await;
      let state = ProxyState {
          session: "ses_fixed123".into(),
          zen_body_patch: is_zen_upstream(&base_url),
          base_url,
          log: None,
          trace: TraceSink::disabled(),
          requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
          session_map: Arc::new(SessionMap::default()),
           modes: Arc::new(ThreadModeRegistry::default()),
      };
        let mut headers = HeaderMap::new();
        headers.insert("session-id", HeaderValue::from_static("client-session-42"));

        let uri: Uri = "/v1/models".parse().unwrap();
        let resp =
            forward_passthrough(&state, Method::GET, &uri, &headers, Body::empty()).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let got = rec.lock().await.clone().expect("mock 应已收到 GET 请求");
        // 入站 session-id 不合法（不是 opencode 形状）也不影响：代理按它派生一个合法会话 id
        let session = got.opencode_session.clone().expect("应有会话 id");
        assert_is_opencode_id(&session, "ses");
        assert_eq!(
            state.session_map.codex_of(&session).as_deref(),
            Some("client-session-42")
        );
    }

    /// 起一个"任何方法/路径都记录一行并回 chat.completion JSON"的上游，
    /// 用于分辨请求走的是翻译分支还是透传分支。
    async fn spawn_mock_record_all(rec: Arc<AsyncMutex<Vec<String>>>) -> String {
        let app = Router::new()
            .fallback(
                move |method: Method,
                      uri: Uri,
                      headers: HeaderMap,
                      _body: axum::body::Bytes| {
                let rec = rec.clone();
                async move {
                    let session = headers
                        .get("x-opencode-session")
                        .and_then(|h| h.to_str().ok())
                        .unwrap_or("-")
                        .to_string();
                    rec.lock().await.push(format!(
                        "{} {} {}",
                        method.as_str(),
                        uri.path(),
                        session
                    ));
                    Json(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "m",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "mock 回复" },
                            "finish_reason": "stop"
                        }]
                    }))
                }
            },
            )
            // 大请求体用例要能读完整 body（axum 默认上限 2MB）；layer 只作用于此前注册的路由。
            .layer(axum::extract::DefaultBodyLimit::max(RESPONSES_BODY_LIMIT));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    async fn body_text(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// 记录行的最后一条：`METHOD path session`。
    async fn last_row(rec: &Arc<AsyncMutex<Vec<String>>>) -> Option<String> {
        rec.lock().await.last().cloned()
    }

    /// 取一个当前空闲的回环端口（测试用；绑定后立即释放）。
    fn free_loopback_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    // ---------- 口嗨自动续跑：脚本化上游与端到端用例 ----------

    /// 脚本化上游响应：SSE 分片用于流式（正常回复与续跑轮都是流式），
    /// `SseDelayed` 模拟「首包很慢」的上游。
    #[derive(Clone)]
    enum ScriptedReply {
        Sse(Vec<String>),
        /// 等待 `delay` 后一次性下发全部 `lines`：模拟「首包很慢」的上游（续跑轮会用到）。
        SseDelayed { lines: Vec<String>, delay: Duration },
    }

    /// 起一个按调用次序返回脚本化响应的 mock 上游，并记录**全部**收到的请求体；
    /// 脚本用完后回一条内容为「已完成」的普通 JSON（避免未覆盖的调用把用例带偏）。
    async fn spawn_mock_zen_scripted(
        script: Vec<ScriptedReply>,
    ) -> (String, Arc<AsyncMutex<Vec<Value>>>) {
        let queue = Arc::new(AsyncMutex::new(VecDeque::from(script)));
        let rec: Arc<AsyncMutex<Vec<Value>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let rec_for_route = rec.clone();
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |Json(body): Json<Value>| {
                let queue = queue.clone();
                let rec = rec_for_route.clone();
                async move {
                    rec.lock().await.push(body);
                    let reply = queue.lock().await.pop_front();
                    match reply {
                        Some(ScriptedReply::Sse(lines)) => {
                            let chunks: Vec<Result<axum::body::Bytes, std::io::Error>> = lines
                                .into_iter()
                                .map(|l| Ok(axum::body::Bytes::from(l)))
                                .collect();
                            axum::response::Response::builder()
                                .header(header::CONTENT_TYPE, "text/event-stream")
                                .body(Body::from_stream(stream::iter(chunks)))
                                .unwrap()
                        }
                        Some(ScriptedReply::SseDelayed { lines, delay }) => {
                            let body = stream::once(async move {
                                tokio::time::sleep(delay).await;
                                let body: Vec<u8> =
                                    lines.into_iter().flat_map(|l| l.into_bytes()).collect();
                                Ok::<_, std::io::Error>(axum::body::Bytes::from(body))
                            });
                            axum::response::Response::builder()
                                .header(header::CONTENT_TYPE, "text/event-stream")
                                .body(Body::from_stream(body))
                                .unwrap()
                        }
                        None => axum::response::Response::builder()
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(
                                json!({
                                    "choices": [{
                                        "message": { "role": "assistant", "content": "已完成" },
                                        "finish_reason": "stop"
                                    }]
                                })
                                .to_string(),
                            ))
                            .unwrap(),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), rec)
    }

    /// 一段「纯文本回复」的流式分片。
    fn sse_text_reply(text: &str) -> Vec<String> {
        vec![
            format!(
                "data: {}\n\n",
                json!({ "choices": [{ "index": 0, "delta": { "content": text }, "finish_reason": null }] })
            ),
            format!(
                "data: {}\n\n",
                json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] })
            ),
            "data: [DONE]\n\n".to_string(),
        ]
    }

    /// 一段「工具调用」的流式分片。
    fn sse_tool_call_reply(name: &str, arguments: &str) -> Vec<String> {
        vec![
            format!(
                "data: {}\n\n",
                json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{
                    "index": 0, "id": "call_1",
                    "function": { "name": name, "arguments": arguments }
                }] }, "finish_reason": null }] })
            ),
            format!(
                "data: {}\n\n",
                json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }] })
            ),
            "data: [DONE]\n\n".to_string(),
        ]
    }

    /// 默认模式按约定收尾的终局文本（`<zen_task_completed>` 成对标签）。
    fn completion_reply(reason: &str) -> ScriptedReply {
        ScriptedReply::Sse(sse_text_reply(&format!(
            "<zen_task_completed>{reason}</zen_task_completed>"
        )))
    }

    /// 某个上游请求体里最后一条 assistant 消息的文本（首轮教学注入在 system 里，不影响它）。
    fn last_assistant_text(call: &Value) -> String {
        call["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|m| m["role"].as_str() == Some("assistant"))
            })
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// 某个上游请求体里的 system 文本（= 入站 `instructions`，首轮教学的契约挂在这里）。
    fn system_text(call: &Value) -> String {
        call["messages"]
            .as_array()
            .and_then(|messages| messages.first())
            .filter(|m| m["role"].as_str() == Some("system"))
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// 某个上游请求体里最后一条消息的文本（续跑轮即注入的续跑提醒）。
    fn last_message_text(call: &Value) -> String {
        call["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// 入站 Responses 请求体（JSON）：指定末条 user 文本，可选带一个工具声明。
    fn nudge_probe_json_for(user_text: &str, with_tools: bool) -> Value {
        let mut req = json!({
            "model": "mimo-v2.5-free",
            "stream": true,
            "input": [{ "type": "message", "role": "user",
                        "content": [{ "type": "input_text", "text": user_text }] }]
        });
        if with_tools {
            req["tools"] = json!([{
                "type": "function",
                "name": "shell",
                "description": "run shell",
                "parameters": { "type": "object", "properties": {} }
            }]);
        }
        req
    }

    /// 入站 Responses 请求体（JSON）：可选带一个工具声明（无工具时不应催办）。
    fn nudge_probe_json(with_tools: bool) -> Value {
        nudge_probe_json_for("把 base_url 的测试补上", with_tools)
    }

    fn nudge_probe(with_tools: bool) -> Body {
        Body::from(nudge_probe_json(with_tools).to_string())
    }

    /// 应用 `autoTitleThread` 发起的会话标题生成请求（后台临时线程）。
    fn nudge_probe_title_task(with_tools: bool) -> Body {
        let text = format!(
            "{TITLE_TASK_PREFIX}，只输出标题本身，不要任何解释、引号或 Markdown。\n\n用户消息：\n把 zen 代理的看门狗改一下"
        );
        Body::from(nudge_probe_json_for(&text, with_tools).to_string())
    }

    /// 在探测请求前面插入一条协作模式开发者消息（模拟 codex 下发的模式文本）。
    fn nudge_probe_with_mode(with_tools: bool, mode_text: &str) -> Body {
        nudge_probe_with_modes(with_tools, &[mode_text])
    }

    /// 在探测请求前面按顺序插入多条协作模式开发者消息：codex 会把历次模式块都留在历史里，
    /// 切换模式后的请求正是「旧块在前、当前块在后」的形状（线上误判的复现形态）。
    fn nudge_probe_with_modes(with_tools: bool, mode_texts: &[&str]) -> Body {
        let mut req = nudge_probe_json(with_tools);
        let mut input: Vec<Value> = mode_texts
            .iter()
            .map(|text| {
                json!({
                    "type": "message",
                    "role": "developer",
                    "content": [{ "type": "input_text", "text": text }]
                })
            })
            .collect();
        input.extend(req["input"].as_array().unwrap().clone());
        req["input"] = Value::Array(input);
        Body::from(req.to_string())
    }

    /// 带 `session-id` 请求头（codex 上报的线程 id，代理给上游时才补 `ses_` 前缀）。
    fn session_headers(thread_id: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("session-id", HeaderValue::from_str(thread_id).unwrap());
        headers
    }

    /// 预置一条登记的协议模式登记表（模拟 app-server 侧已经登记过该线程的模式）。
    fn modes_with(thread_id: &str, mode: &str) -> Arc<ThreadModeRegistry> {
        let registry = ThreadModeRegistry::default();
        assert!(registry.record(thread_id, mode), "登记应成功");
        Arc::new(registry)
    }

    /// 探测用代理状态（可复用：`nudge` 计数表在同一个 state 内跨请求保留，
    /// 测「跨请求不复用计数」这类行为时必须复用同一个 state）。
    fn nudge_probe_state(upstream: &str, log: ZenLog) -> ProxyState {
        ProxyState {
            session: "ses_fixed123".into(),
            base_url: upstream.to_string(),
            log,
            trace: TraceSink::disabled(),
            requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
            zen_body_patch: is_zen_upstream(upstream),
            session_map: Arc::new(SessionMap::default()),
            modes: Arc::new(ThreadModeRegistry::default()),
        }
    }

    /// 用一个已构造好的代理状态走一次流式翻译入口，返回发给 codex 的完整 SSE 文本。
    async fn run_nudge_probe_state(
        state: ProxyState,
        headers: HeaderMap,
        body: Body,
    ) -> String {
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(state),
            Method::POST,
            OriginalUri(uri),
            headers,
            body,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        body_text(resp).await
    }

    /// 走一次代理的流式翻译入口（指定请求头与模式登记表），返回发给 codex 的完整 SSE 文本。
    async fn run_nudge_probe_full(
        upstream: &str,
        log: ZenLog,
        headers: HeaderMap,
        body: Body,
        modes: Arc<ThreadModeRegistry>,
    ) -> String {
        let mut state = nudge_probe_state(upstream, log);
        state.modes = modes;
        run_nudge_probe_state(state, headers, body).await
    }

    /// 走一次代理的流式翻译入口（无请求头、空登记表），返回发给 codex 的完整 SSE 文本。
    async fn run_nudge_probe(upstream: &str, log: ZenLog, body: Body) -> String {
        run_nudge_probe_full(
            upstream,
            log,
            HeaderMap::new(),
            body,
            Arc::new(ThreadModeRegistry::default()),
        )
        .await
    }

    /// 读取会话日志目录下全部内容（断言 nudge_* 事件用）。
    fn read_session_log(dir: &tempfile::TempDir) -> String {
        std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_injects_continuation_and_relays_tool_call() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "Now update the test - specifically the base_url path test.",
            )),
            ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
        ])
        .await;

        let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

        // 对 codex 而言只是「一条带文本的响应，随后调用了工具」，只收尾一次
        assert!(body.contains("response.output_text.delta"), "{body}");
        assert!(body.contains("Now update the test"), "{body}");
        assert!(body.contains("function_call"), "{body}");
        assert_eq!(body.matches("event: response.completed").count(), 1);

        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 2, "应为：首轮 + 续跑（不再有独立判定请求）");
        // 首轮与续跑轮都带首轮教学注入的契约（instructions → system 消息）
        assert!(
            system_text(&calls[0]).contains(DEFAULT_MODE_CONTRACT_TEXT),
            "首轮应带默认模式契约：{}",
            calls[0]
        );
        // 续跑调用：历史尾部是「助手原样文本 + 注入提醒」
        let messages = calls[1]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "契约的 system 消息 + user + assistant + 提醒：{messages:?}");
        assert_eq!(messages.last().unwrap()["role"], "user");
        assert!(last_assistant_text(&calls[1]).contains("Now update the test"));
        let injected = messages.last().unwrap()["content"].as_str().unwrap();
        assert!(injected.contains("自动续跑"), "{injected}");
        assert!(
            injected.contains(&format!("{TASK_COMPLETED_MARKER}>")),
            "默认模式提醒必须教出成对标签：{injected}"
        );
        assert!(
            injected.contains("必须实际调用工具"),
            "默认模式提醒必须带执行口径：{injected}"
        );
        // 会话日志：只留注入一条（判定事件已随看门狗删除）
        let joined = read_session_log(&dir);
        assert!(joined.contains("event=zen_proxy.nudge_injected"), "{joined}");
        assert!(!joined.contains("event=zen_proxy.nudge_judged"), "{joined}");
    }

    /// 终局自带 `<zen_task_completed>`：模型自己宣告任务结束，一次调用就收尾（不注入、不续跑）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_skipped_when_completion_tag_present() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "这段代码无需改动，原因是 ……\n<zen_task_completed>无需改动</zen_task_completed>",
            )),
            // 第 2 轮不该发生：真发生时脚本会回空话，下面的调用数断言会失败
            ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
        ])
        .await;

        let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert!(!body.contains("function_call"), "{body}");
        assert_eq!(rec.lock().await.len(), 1, "带标签时不应续跑");
        // 正文保留、标签（含载荷）不进 codex：delta 与三个 done 事件里都不该有它
        assert!(body.contains("这段代码无需改动"), "{body}");
        assert!(
            !body.contains(TASK_COMPLETED_MARKER),
            "标签不得下发到 codex：{body}"
        );
        assert!(
            !body.contains("无需改动</zen_task_completed>"),
            "标签载荷也不得下发：{body}"
        );
        // 下发的 done 文本 = 剥掉标签后的正文（delta 与 done 一致）
        assert!(
            body.contains(r#""text":"这段代码无需改动，原因是 ……\n"#),
            "{body}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_skipped")
                && joined.contains("原因=task_completed")
                && joined.contains("模式=default"),
            "{joined}"
        );
        assert!(
            joined.contains("标签内容=无需改动"),
            "被删载荷要进日志：{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_injected"),
            "带标签时不应注入：{joined}"
        );
    }

    /// 续跑轮与首轮走同一个 `forward`：门禁补丁在每一轮上游请求体里都要在。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn zen_body_patch_applies_to_nudge_continuation() {
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "Now update the test - specifically the base_url path test.",
            )),
            completion_reply("已完成"),
        ])
        .await;

        let mut state = nudge_probe_state(&upstream, None);
        state.zen_body_patch = true;
        let body = run_nudge_probe_state(state, HeaderMap::new(), nudge_probe(true)).await;
        assert_eq!(body.matches("event: response.completed").count(), 1);

        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 2, "应为：首轮 + 续跑");
        for call in &calls {
            assert_eq!(
                call["max_tokens"], ZEN_MAX_TOKENS,
                "每一轮都要补 max_tokens：{call}"
            );
            let names: Vec<&str> = call["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["function"]["name"].as_str().unwrap())
                .collect();
            assert_eq!(
                names,
                vec!["shell", "bash", "edit", "glob", "grep", "read", "write"],
                "每一轮都要补假工具：{call}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_stops_after_max_injections() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("第一句空话")),
            ScriptedReply::Sse(sse_text_reply("第二句空话")),
            ScriptedReply::Sse(sse_text_reply("第三句空话")),
            ScriptedReply::Sse(sse_text_reply("第四句空话")),
            ScriptedReply::Sse(sse_text_reply("第五句空话")),
        ])
        .await;

        let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(
            rec.lock().await.len(),
            5,
            "首轮 + 续跑 × 4；第五次口嗨不再注入（单请求注入上限）"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_limited")
                && joined.contains("原因=max_injections")
                && joined.contains("轮次=5")
                && joined.contains("说明=本请求已催办 4 次仍未收尾，停止催办"),
            "{joined}"
        );
        assert_eq!(
            joined.matches("event=zen_proxy.nudge_injected").count(),
            4,
            "恰好注入四次：{joined}"
        );
        assert!(
            joined.contains("本请求催办次数=4/4") && joined.contains("说明="),
            "注入日志应带中文次数与说明：{joined}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_skipped_when_request_has_no_tools() {
        let (upstream, rec) =
            spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply("纯聊天回答"))]).await;

        let body = run_nudge_probe(&upstream, None, nudge_probe(false)).await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(rec.lock().await.len(), 1, "无工具可用时不催办");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_second_pass_survives_slow_upstream() {
        // 续跑轮的首包晚于 USAGE_GRACE：它不应继承上一轮的 finish_reason 而套用
        // 「尾包宽限」把自己整轮丢掉（那样注入就白做了、工具调用也拿不到）。
        let tool_chunks = sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}");
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("我先说明一下接下来要做的事。")),
            ScriptedReply::SseDelayed {
                lines: tool_chunks,
                delay: USAGE_GRACE + Duration::from_millis(500),
            },
        ])
        .await;

        let body = run_nudge_probe(&upstream, None, nudge_probe(true)).await;

        assert!(body.contains("function_call"), "{body}");
        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(rec.lock().await.len(), 2);
    }

    /// 计划模式下的协作模式文本段（codex 0.154 实测文案，见 [`REAL_PLAN_MODE_BLOCK`]；
    /// 前缀补一段 skills 文本以贴近真实请求：模式块拼在同一个 developer 条目末尾）。
    fn plan_mode_text() -> String {
        format!("<skills_instructions>## Skills …</skills_instructions>{REAL_PLAN_MODE_BLOCK}")
    }

    /// 计划模式下终局没有计划标签：按「计划未交付」注入计划专用提醒续跑，
    /// 全程纯代码判据（计划模式不看默认模式的收尾标签，也不发任何独立判定请求）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_plan_mode_asks_for_plan_when_no_marker() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            // 故意不带 <proposed_plan> 包裹：弱模型常直接把计划写成普通 Markdown，
            // 这条用例要保证它被代码判据判为「未交付」并催出计划。
            ScriptedReply::Sse(sse_text_reply(
                "## 计划\n1. 后端加 git_version 命令\n2. 关于页面加一行显示\n3. 跑测试",
            )),
            // 续跑轮：这次给了带标签的计划（对 codex 只是一条普通文本响应）
            ScriptedReply::Sse(sse_text_reply(
                "<proposed_plan>\n1. 后端加 git_version 命令\n2. 关于页面加一行显示\n</proposed_plan>",
            )),
        ])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(
            calls.len(),
            2,
            "计划模式下应「首轮 + 续跑」，无独立判定请求：{calls:?}"
        );
        // 续跑调用：历史尾部是「助手原样文本 + 计划专用提醒」
        let messages = calls[1]["messages"].as_array().unwrap();
        let last = messages.last().unwrap();
        assert_eq!(last["role"], "user");
        let injected = last["content"].as_str().unwrap();
        assert!(injected.contains("没有交付计划"), "{injected}");
        assert!(injected.contains("<proposed_plan>"), "{injected}");
        assert!(
            !injected.contains("必须实际调用工具"),
            "计划模式不得注入执行口径：{injected}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_injected") && joined.contains("模式=plan"),
            "{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_judged"),
            "已随看门狗删除的判定事件不应出现：{joined}"
        );
    }

    /// 用户中途放弃计划（实测例子：「行，那不处理了」）：首轮模型只写普通正文 → 催办一轮，
    /// 续跑轮按约定用 `<zen_plan_cancelled>` 标签收尾 → 代码判据直接收尾，不再注入第 3 轮、
    /// 不会把一份已被放弃的完整方案重新逼出来。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_plan_mode_stops_when_plan_cancelled() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("好，那就保持现状，这一项先不改了。")),
            ScriptedReply::Sse(sse_text_reply(
                "明白，那就不处理了。\n<zen_plan_cancelled>用户说不用改了</zen_plan_cancelled>",
            )),
            // 第 3 轮不该发生：真发生时脚本会回空话，下面的调用数断言会失败
            ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
        ])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(
            calls.len(),
            2,
            "「已取消计划」应当一轮催办后就收尾，不再注入：{calls:?}"
        );
        // 催办轮注入的提醒里带上了约定标签
        let messages = calls[1]["messages"].as_array().unwrap();
        let injected = messages.last().unwrap()["content"].as_str().unwrap();
        assert!(injected.contains("<zen_plan_cancelled>"), "{injected}");
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_skipped")
                && joined.contains("原因=plan_cancelled")
                && joined.contains("模式=plan"),
            "{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_limited"),
            "已取消分支不该走到催办次数上限：{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_judged"),
            "已随看门狗删除的判定事件不应出现：{joined}"
        );
    }

    /// 模型首轮就已按约定给出取消标签（早前回合被催过）：一次调用就收尾，连催办都不发。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_plan_mode_skips_injection_when_already_cancelled() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "那就不做改动了。\n<zen_plan_cancelled>用户说不用改了</zen_plan_cancelled>",
            )),
            ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
        ])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(
            rec.lock().await.len(),
            1,
            "首轮即带取消标签时不应再打上游"
        );
        // 标签不下发：聊天里只剩模型那句结论
        assert!(
            body.contains("那就不做改动了。") && !body.contains("<zen_plan_cancelled"),
            "计划模式的出口标签必须剥离：{body}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_skipped")
                && joined.contains("原因=plan_cancelled")
                && joined.contains("轮次=1")
                && joined.contains("标签内容=用户说不用改了"),
            "{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_injected"),
            "已取消的计划不应再注入催办：{joined}"
        );
    }

    /// 问题本身无法或无需产出实现计划（如「1+1=？」这类事实问题）：首轮模型只写普通正文 →
    /// 催办一轮，续跑轮按约定用 `<zen_plan_unachievable>` 标签收尾 → 代码判据直接收尾，
    /// 不会把一份本就不产出计划的方案重新逼出来。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_plan_mode_stops_when_plan_unachievable() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("1 + 1 = 2。")),
            ScriptedReply::Sse(sse_text_reply(
                "1 + 1 = 2。\n<zen_plan_unachievable>事实问题，不产出实现计划</zen_plan_unachievable>",
            )),
            // 第 3 轮不该发生：真发生时脚本会回空话，下面的调用数断言会失败
            ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
        ])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(
            calls.len(),
            2,
            "「无法/无需计划」应当一轮催办后就收尾，不再注入：{calls:?}"
        );
        // 催办轮注入的提醒里带上了约定标签
        let messages = calls[1]["messages"].as_array().unwrap();
        let injected = messages.last().unwrap()["content"].as_str().unwrap();
        assert!(injected.contains("<zen_plan_unachievable>"), "{injected}");
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_skipped")
                && joined.contains("原因=plan_unachievable")
                && joined.contains("模式=plan"),
            "{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_limited"),
            "无法/无需计划分支不该走到催办次数上限：{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_judged"),
            "已随看门狗删除的判定事件不应出现：{joined}"
        );
    }

    /// 模型首轮就已按约定给出「无法/无需计划」标签（早前回合被催过）：一次调用就收尾，
    /// 连催办都不发。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_plan_mode_skips_injection_when_already_unachievable() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "这就是个纯查询。\n<zen_plan_unachievable>无实现计划可给</zen_plan_unachievable>",
            )),
            ScriptedReply::Sse(sse_text_reply("这一轮不应该被调用")),
        ])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(
            rec.lock().await.len(),
            1,
            "首轮即带无法/无需计划标签时不应再打上游"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_skipped")
                && joined.contains("原因=plan_unachievable")
                && joined.contains("轮次=1"),
            "{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_injected"),
            "无法/无需计划不应再注入催办：{joined}"
        );
    }

    /// 关键回归：历史里残留旧的计划模式块（codex 把历次模式块都留在历史里），但协议登记表
    /// 说这个线程现在是默认模式——必须走默认模式的续跑（注入带 `<zen_task_completed>` 契约的执行
    /// 口径），不能按计划模式注入「请给出计划」（这正是线上 `collaboration_mode_kind=default`
    /// 却按 plan 分流的那次误判）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_mode_registry_beats_stale_plan_block_in_history() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "Now update the test - specifically the base_url path test.",
            )),
            ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
        ])
        .await;

        let body = run_nudge_probe_full(
            &upstream,
            log,
            session_headers("01a0aaf8-abc"),
            nudge_probe_with_modes(true, &[&plan_mode_text(), REAL_DEFAULT_MODE_BLOCK]),
            modes_with("01a0aaf8-abc", "default"),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(
            calls.len(),
            2,
            "登记表说默认模式：应「首轮 + 续跑」（无独立判定请求）：{calls:?}"
        );
        let injected = calls[1]["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(
            injected.contains("必须实际调用工具"),
            "默认模式应注入执行口径：{injected}"
        );
        assert!(
            injected.contains(&format!("{TASK_COMPLETED_MARKER}>")),
            "默认模式应注入标签契约：{injected}"
        );
        assert!(
            !injected.contains("没有交付计划"),
            "不得按计划模式注入计划提醒：{injected}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_injected"),
            "默认模式应注入续跑提醒：{joined}"
        );
        assert!(
            joined.contains("模式=default") && joined.contains("模式来源=registry"),
            "模式与来源应记进日志：{joined}"
        );
        assert!(
            !joined.contains("模式=plan"),
            "历史里的旧计划块不应把模式拉回 plan：{joined}"
        );
    }

    /// 反向：登记表说计划模式、而历史里最后一块是默认模式块时，以登记表为准走计划分支
    /// （注入计划专用提醒，不看默认模式标签）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_mode_registry_plan_wins_over_default_block_in_history() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "## 计划\n1. 后端加 git_version 命令\n2. 关于页面加一行显示",
            )),
            ScriptedReply::Sse(sse_text_reply(
                "<proposed_plan>\n1. 后端加 git_version 命令\n</proposed_plan>",
            )),
        ])
        .await;

        let body = run_nudge_probe_full(
            &upstream,
            log,
            session_headers("ses_thread-plan"),
            nudge_probe_with_modes(true, &[REAL_DEFAULT_MODE_BLOCK]),
            modes_with("thread-plan", "plan"),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 2, "计划模式只应「首轮 + 续跑」：{calls:?}");
        let injected = calls[1]["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(injected.contains("没有交付计划"), "{injected}");
        assert!(injected.contains("<proposed_plan>"), "{injected}");
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("模式=plan") && joined.contains("模式来源=registry"),
            "{joined}"
        );
        assert!(
            !joined.contains("event=zen_proxy.nudge_judged"),
            "已随看门狗删除的判定事件不应出现：{joined}"
        );
    }

    /// 没有 `session-id` 请求头（其它客户端/连不上登记表的线程）时退回关键词兜底判据，
    /// 且以**最后一个**模式块为准：默认块在最后 → 默认分支；计划块在最后 → 计划分支。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_mode_falls_back_to_last_mode_block_without_registry() {
        // ① 旧计划块在前、默认块在后 → 默认模式（续跑 + 执行口径）
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("我先说明一下接下来要做的事。")),
            ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
        ])
        .await;
        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_modes(true, &[&plan_mode_text(), REAL_DEFAULT_MODE_BLOCK]),
        )
        .await;
        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(rec.lock().await.len(), 2, "默认分支应续跑一轮");
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("模式=default") && joined.contains("模式来源=heuristic"),
            "无登记表时应记关键词兜底来源：{joined}"
        );
        assert!(
            joined.contains("event=zen_proxy.nudge_injected"),
            "默认分支应注入续跑提醒：{joined}"
        );

        // ② 默认块在前、计划块在后 → 计划模式（注入计划提醒）
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("## 计划\n1. 做 A")),
            ScriptedReply::Sse(sse_text_reply(
                "<proposed_plan>\n1. 做 A\n</proposed_plan>",
            )),
        ])
        .await;
        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_modes(true, &[REAL_DEFAULT_MODE_BLOCK, &plan_mode_text()]),
        )
        .await;
        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 2, "计划分支只续跑一轮");
        let injected = calls[1]["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(
            injected.contains("没有交付计划") && injected.contains("<proposed_plan>"),
            "计划分支必须注入计划专用提醒：{injected}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("模式=plan") && joined.contains("模式来源=heuristic"),
            "{joined}"
        );
    }

    /// 计划模式下终局带了计划标签：代码判据直接判「计划已交付」，一次上游调用就收尾。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_skipped_in_plan_mode_when_plan_delivered() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
            "<proposed_plan>\n1. 做 A\n2. 做 B\n</proposed_plan>",
        ))])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(rec.lock().await.len(), 1, "已交付的计划不应再打上游");
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_skipped")
                && joined.contains("原因=plan_output")
                && joined.contains("模式=plan"),
            "{joined}"
        );
    }

    /// 计划模式也受「同会话连续注入 2 次」上限约束：第 3 次没给出计划时不再注入。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_plan_mode_stops_after_max_injections() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("第一句空话")),
            ScriptedReply::Sse(sse_text_reply("第二句空话")),
            ScriptedReply::Sse(sse_text_reply("第三句空话")),
            ScriptedReply::Sse(sse_text_reply("第四句空话")),
            ScriptedReply::Sse(sse_text_reply("第五句空话")),
        ])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(
            rec.lock().await.len(),
            5,
            "首轮 + 续跑 × 4；第五次没给计划也不再注入"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_limited")
                && joined.contains("原因=max_injections")
                && joined.contains("模式=plan")
                && joined.contains("轮次=5"),
            "{joined}"
        );
    }

    /// 默认模式的协作块正文里带「(e.g. Plan mode)」：不能被误判成计划模式而注入计划提醒
    /// （否则默认模式的正常编码回合会被注入「请给出计划」）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_default_mode_block_injects_execution_nudge() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("先给个说明，接下来我再动手。")),
            // 续跑轮真的调用了工具：回合就此收尾，不再产生第三轮
            ScriptedReply::Sse(sse_tool_call_reply("shell", "{\"cmd\":\"ls\"}")),
        ])
        .await;

        let body = run_nudge_probe(
            &upstream,
            log,
            nudge_probe_with_mode(true, REAL_DEFAULT_MODE_BLOCK),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert!(body.contains("function_call"), "{body}");
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 2, "默认模式应「首轮 + 续跑」：{calls:?}");
        let injected = calls[1]["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(
            injected.contains("必须实际调用工具"),
            "默认模式注入执行口径：{injected}"
        );
        assert!(
            injected.contains(&format!("{TASK_COMPLETED_MARKER}>")),
            "默认模式注入标签契约：{injected}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_injected") && joined.contains("模式=default"),
            "{joined}"
        );
    }

    /// 默认模式下终局文本本身是计划产物（`<proposed_plan>`）：那不是默认模式的收尾标签，
    /// 因此照常注入续跑提醒；模型在续跑轮按约定用 `<zen_task_completed>` 收尾即结束
    /// （「已给方案、等你确认」也要写进标签，不再由 AI 判定替它放行）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_default_mode_plan_output_is_not_a_completion_tag() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply(
                "先给方案：\n<proposed_plan>\n1. 做 A\n2. 做 B\n</proposed_plan>",
            )),
            completion_reply("已给方案，等你确认后再动手"),
        ])
        .await;

        let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 2, "计划产物不是默认模式收尾标签，应续跑一轮：{calls:?}");
        // 续跑请求：历史尾部是「助手原样文本（含计划包裹）+ 注入提醒」
        assert!(
            last_assistant_text(&calls[1]).contains("1. 做 A"),
            "续跑请求应原样带上上一轮助手文本：{}",
            calls[1]
        );
        assert!(
            last_message_text(&calls[1]).contains(&format!("{TASK_COMPLETED_MARKER}>")),
            "续跑请求应教出标签契约：{}",
            calls[1]
        );
        // 第二轮带标签收尾：下发给 codex 的文本里只剩方案，标签被结构剥离
        assert!(body.contains("1. 做 A"), "{body}");
        assert!(
            !body.contains(TASK_COMPLETED_MARKER),
            "标签不得下发到 codex：{body}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("原因=task_completed") && joined.contains("模式=default"),
            "{joined}"
        );
        assert!(
            joined.contains("标签内容=已给方案"),
            "剥离后的载荷要进日志：{joined}"
        );
    }

    /// 模型不守约定、标签里写了一整句话：照样按结构剥离（剥离不看载荷），日志记全。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_strips_tag_even_when_payload_is_free_text() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
            "我把 base_url 的测试补上了。\n<zen_task_completed>我判断已经全部完成：新增了 3 个用例，并跑通了 cargo test</zen_task_completed>",
        ))])
        .await;

        let body = run_nudge_probe(&upstream, log, nudge_probe(true)).await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert_eq!(rec.lock().await.len(), 1, "首轮带标签即收尾");
        assert!(
            body.contains("我把 base_url 的测试补上了。") && !body.to_lowercase().contains("<zen_"),
            "自由文本载荷也必须整段剥离：{body}"
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("标签内容=我判断已经全部完成"),
            "被删载荷要进日志：{joined}"
        );
    }

    /// 会话标题生成请求（应用后台临时线程）：整轮放行，不催办、不注入。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_skipped_for_title_task() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
            "zen 看门狗改进",
        ))])
        .await;

        let body = run_nudge_probe(&upstream, log, nudge_probe_title_task(true)).await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        assert!(body.contains("zen 看门狗改进"), "{body}");
        assert_eq!(
            rec.lock().await.len(),
            1,
            "标题任务不应注入续跑提醒：{:?}",
            rec.lock().await
        );
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("event=zen_proxy.nudge_skipped") && joined.contains("原因=title_task"),
            "{joined}"
        );
    }

    /// 首轮教学（默认模式）：契约挂在 `instructions` 尾部（→ system 消息），首轮请求就带；
    /// 续跑提醒仍然只出现在续跑轮，两者不是同一段文本。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_default_mode_first_pass_request_carries_contract() {
        let (upstream, rec) = spawn_mock_zen_scripted(vec![
            ScriptedReply::Sse(sse_text_reply("我先说明一下接下来要做的事。")),
            completion_reply("已完成：只是说明"),
        ])
        .await;

        let body = run_nudge_probe(&upstream, None, nudge_probe(true)).await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 2, "应「首轮 + 续跑」：{calls:?}");
        // 首轮：system 里就是默认模式契约；不含续跑提醒，也不含计划模式的出口标签
        let first_system = system_text(&calls[0]);
        assert!(
            first_system.contains(DEFAULT_MODE_CONTRACT_TEXT),
            "首轮应带默认模式契约：{}",
            calls[0]
        );
        assert!(!first_system.contains("自动续跑"), "首轮不该有续跑提醒：{first_system}");
        assert!(!first_system.contains(PLAN_CANCEL_MARKER), "{first_system}");
        assert!(!first_system.contains(PLAN_UNACHIEVABLE_MARKER), "{first_system}");
        // 续跑轮：契约仍在，历史尾部追加了续跑提醒
        assert!(
            system_text(&calls[1]).contains(DEFAULT_MODE_CONTRACT_TEXT),
            "续跑轮应继续带契约：{}",
            calls[1]
        );
        let injected = last_message_text(&calls[1]);
        assert!(
            injected.contains(TASK_COMPLETED_MARKER) && injected.contains("自动续跑"),
            "续跑轮尾部应是带标签的提醒：{injected}"
        );
    }

    /// 首轮教学（计划模式）：只前置两个出口标签，不前置「必须给完整方案」；两种模式的契约互不串味。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_plan_mode_first_pass_request_carries_exit_contract() {
        let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
            "<proposed_plan>\n1. 做 A\n</proposed_plan>",
        ))])
        .await;

        let body = run_nudge_probe(
            &upstream,
            None,
            nudge_probe_with_mode(true, &plan_mode_text()),
        )
        .await;

        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 1, "已交付的计划不该再有第二轮：{calls:?}");
        let system = system_text(&calls[0]);
        assert!(
            system.contains(PLAN_MODE_CONTRACT_TEXT),
            "计划模式首轮应带出口契约：{}",
            calls[0]
        );
        assert!(system.contains(PLAN_CANCEL_MARKER), "{system}");
        assert!(system.contains(PLAN_UNACHIEVABLE_MARKER), "{system}");
        // 模式不串味：计划模式请求不带默认模式收尾标签，默认模式请求不带计划出口标签
        assert!(!system.contains(TASK_COMPLETED_MARKER), "{system}");
        assert!(
            !system.contains("都必须给出完整方案"),
            "首轮教学不前置强制口径：{system}"
        );
    }

    /// 首轮教学的门槛：标题线程与「没有工具声明」的请求都不注入契约
    /// （非流式由 `contract_injection_eligible` 单测覆盖）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn contract_not_injected_for_title_task_or_tool_less_request() {
        // ① 标题线程：整轮放行，system 里没有契约
        let (upstream, rec) = spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply(
            "zen 首轮教学",
        ))])
        .await;
        let body = run_nudge_probe(&upstream, None, nudge_probe_title_task(true)).await;
        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(
            !system_text(&calls[0]).contains(DEFAULT_MODE_CONTRACT_TEXT),
            "标题线程不该注入契约：{}",
            calls[0]
        );

        // ② 没有工具声明：与催办同一门槛，不注入
        let (upstream, rec) =
            spawn_mock_zen_scripted(vec![ScriptedReply::Sse(sse_text_reply("纯聊天回答"))]).await;
        let body = run_nudge_probe(&upstream, None, nudge_probe(false)).await;
        assert_eq!(body.matches("event: response.completed").count(), 1);
        let calls = rec.lock().await.clone();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(
            !system_text(&calls[0]).contains(DEFAULT_MODE_CONTRACT_TEXT),
            "无工具请求不该注入契约：{}",
            calls[0]
        );
    }

    /// 注入上限只按**单次请求**计：同一个 `ProxyState`（同一个会话）连发两次请求，第二次
    /// 仍然是「首轮 + 续跑」各打一次上游——不再有跨请求的连续计数与静默窗口。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nudge_injections_are_per_request() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let headers = session_headers("thread-per-request");
        // 一个 state 贯穿两次请求：跨请求不再有任何计数状态
        let shared = nudge_probe_state("http://127.0.0.1:1", log.clone());

        for round in 1..=2 {
            let (upstream, rec) = spawn_mock_zen_scripted(vec![
                ScriptedReply::Sse(sse_text_reply("接下来我会把测试补上。")),
                completion_reply("已完成：补了测试"),
            ])
            .await;
            let mut state = shared.clone();
            state.base_url = upstream;
            let body = run_nudge_probe_state(state, headers.clone(), nudge_probe(true)).await;
            assert_eq!(body.matches("event: response.completed").count(), 1);
            assert_eq!(
                rec.lock().await.len(),
                2,
                "第 {round} 次请求都应是「首轮 + 续跑」：{:?}",
                rec.lock().await
            );
        }
        let joined = read_session_log(&dir);
        assert!(
            joined.contains("本请求催办次数=1/4")
                && !joined.contains("event=zen_proxy.nudge_limited"),
            "两次请求应各注入一次、都不触发上限：{joined}"
        );
    }

    fn responses_probe() -> Body {
        Body::from(
            json!({ "model": "m", "input": "hi", "stream": false }).to_string(),
        )
    }

    fn proxy_state(base_url: String, log: ZenLog) -> ProxyState {
        proxy_state_traced(base_url, log, TraceSink::disabled())
    }

   fn proxy_state_traced(base_url: String, log: ZenLog, trace: TraceSink) -> ProxyState {
       ProxyState {
           session: "ses_fixed123".into(),
           zen_body_patch: is_zen_upstream(&base_url),
           base_url,
           log,
           trace,
           requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
           session_map: Arc::new(SessionMap::default()),
            modes: Arc::new(ThreadModeRegistry::default()),
       }
   }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispatches_translation_on_base_url_path_plus_responses() {
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;

        // 上游路径 /zen/v2 → 只把 /zen/v2/responses 当翻译入口（带不带尾斜杠都一样）。
        for base in [format!("{upstream}/zen/v2"), format!("{upstream}/zen/v2/")] {
            rec.lock().await.clear();
            let state = proxy_state(base.clone(), None);
            let uri: Uri = "/zen/v2/responses".parse().unwrap();
            let resp = handle_any(
                State(state),
                Method::POST,
                OriginalUri(uri),
                HeaderMap::new(),
                responses_probe(),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_text(resp).await;
            assert!(body.contains("\"object\":\"response\""), "{body}");
            assert_eq!(
                rec.lock().await.clone(),
                vec!["POST /zen/v2/chat/completions ses_fixed123".to_string()]
            );
        }

        // 上游无路径（DeepSeek 官方 base_url 的现实情况）→ /responses 走翻译。
        rec.lock().await.clear();
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(proxy_state(upstream.clone(), None)),
            Method::POST,
            OriginalUri(uri),
            HeaderMap::new(),
            responses_probe(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("\"object\":\"response\""), "{body}");
        assert_eq!(
            rec.lock().await.clone(),
            vec!["POST /chat/completions ses_fixed123".to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unmatched_responses_path_passes_through_and_warns() {
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let state = proxy_state(format!("{upstream}/zen/v1"), log);

        // 本地 provider 少写了 /zen/v1：原样透传到上游，不再静默收尾。
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(state),
            Method::POST,
            OriginalUri(uri),
            HeaderMap::new(),
            responses_probe(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        // 透传即上游原文（chat.completion），而非翻译后的 response 对象。
        assert!(body_text(resp).await.contains("chat.completion"));
        assert_eq!(
            rec.lock().await.clone(),
            vec!["POST /responses ses_fixed123".to_string()]
        );

        let logged = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            logged.contains("zen_proxy.path_unmatched")
                && logged.contains("path=/responses")
                && logged.contains("expected=/zen/v1/responses"),
            "{logged}"
        );
        assert!(logged.contains("zen_proxy.passthrough"), "{logged}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_post_on_responses_path_passes_through() {
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(proxy_state(upstream, None)),
            Method::GET,
            OriginalUri(uri),
            HeaderMap::new(),
            Body::empty(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            rec.lock().await.clone(),
            vec!["GET /responses ses_fixed123".to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn started_proxy_translates_path_derived_from_upstream() {
        // 端到端覆盖 start() 的路由装配：这正是"翻译分支从来没被走到"的根因所在。
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let port = free_loopback_port();
        let mut handle = None;
        let status = apply(
            &mut handle,
            true,
            port,
            format!("{upstream}/zen/v1"),
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(status.running, "{:?}", status.error);

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/zen/v1/responses"))
            .header("content-type", "application/json")
            .body(json!({ "model": "m", "input": "hi", "stream": false }).to_string())
            .send()
            .await
            .expect("代理应可访问");
        assert_eq!(resp.status(), StatusCode::OK);
        let text = resp.text().await.unwrap();
        assert!(text.contains("\"object\":\"response\""), "{text}");
        let rows = rec.lock().await.clone();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].starts_with("POST /zen/v1/chat/completions ses_"),
            "{rows:?}"
        );

        let status = apply(
            &mut handle,
            false,
            port,
            upstream,
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(!status.running);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oversized_body_still_reaches_translation() {
        // 2MB 曾是 axum 默认上限，长会话请求体已近 1MB；这里用 >2MB 体确认不再 413。
        let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream = spawn_mock_record_all(rec.clone()).await;
        let filler = "x".repeat(3 * 1024 * 1024);
        let body = json!({ "model": "m", "stream": false, "input": filler }).to_string();
        let uri: Uri = "/responses".parse().unwrap();
        let resp = handle_any(
            State(proxy_state(upstream, None)),
            Method::POST,
            OriginalUri(uri),
            HeaderMap::new(),
            Body::from(body),
        )
        .await;
        let status = resp.status();
        let text = body_text(resp).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(text.contains("\"object\":\"response\""), "{text}");
        assert_eq!(rec.lock().await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn apply_restarts_only_when_port_or_upstream_changes() {
        let rec_a: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let rec_b: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
        let upstream_a = spawn_mock_record_all(rec_a.clone()).await;
        let upstream_b = spawn_mock_record_all(rec_b.clone()).await;
        let port = free_loopback_port();
        let mut handle = None;

        let status = apply(
            &mut handle,
            true,
            port,
            upstream_a.clone(),
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(status.running, "{:?}", status.error);
        let probe = |port: u16| async move {
            // 每次用新 client：代理重启会关闭旧连接，复用连接池会读到半关闭的连接。
            let resp = reqwest::Client::new()
                .get(format!("http://127.0.0.1:{port}/models"))
                .send()
                .await
                .expect("代理应可访问");
            assert_eq!(resp.status(), StatusCode::OK);
        };

        probe(port).await;
        let first = last_row(&rec_a).await.expect("上游 A 应已收到请求");
        rec_a.lock().await.clear();

        // 端口与上游都没变（含只差尾斜杠的等价写法）→ 复用实例，会话标识不变。
        let status = apply(
            &mut handle,
            true,
            port,
            upstream_a.clone(),
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(status.running, "{:?}", status.error);
        let status = apply(
            &mut handle,
            true,
            port,
            format!("{upstream_a}/"),
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(status.running, "{:?}", status.error);
        probe(port).await;
        assert_eq!(
            last_row(&rec_a).await.as_deref(),
            Some(first.as_str()),
            "同端口同上游不应重启（{first}）"
        );

        // 换上游地址 → 立即重启：B 收到请求、A 不再收到。
        let before_a = rec_a.lock().await.len();
        let status = apply(
            &mut handle,
            true,
            port,
            upstream_b.clone(),
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(status.running, "{:?}", status.error);
        probe(port).await;
        let second = last_row(&rec_b).await.expect("上游 B 应已收到请求");
        assert_ne!(first.split(' ').nth(2), second.split(' ').nth(2));
        assert_eq!(rec_a.lock().await.len(), before_a);

        // 端口变化 → 也重启。
        let other_port = free_loopback_port();
        let status = apply(
            &mut handle,
            true,
            other_port,
            upstream_b,
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(status.running, "{:?}", status.error);
        assert_eq!(status.port, other_port);
        probe(other_port).await;

        // 关闭 → 停止。
        let status = apply(
            &mut handle,
            false,
            other_port,
            upstream_a,
            None,
            TraceSink::disabled(),
            Arc::new(ThreadModeRegistry::default()),
        )
        .await;
        assert!(!status.running);
        assert!(handle.is_none());
    }

    /// 起一个"模仿 DeepSeek 思考模式 + 声明 tools"的 mock：**任一** assistant 消息
    /// （带 content 或带 tool_calls）缺 `reasoning_content` 就回 400（原文照抄），全带上则 200。
    /// 每次请求记录一行 `rc=on|off`。
    async fn spawn_mock_requiring_reasoning_rc(
        rec: Arc<AsyncMutex<Vec<String>>>,
    ) -> String {
        use axum::response::IntoResponse;
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |Json(body): Json<Value>| {
                let rec = rec.clone();
                async move {
                    let messages = body
                        .get("messages")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let assistants = messages
                        .iter()
                        .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
                        .collect::<Vec<_>>();
                    let all_have_rc = !assistants.is_empty()
                        && assistants
                            .iter()
                            .all(|m| m.get("reasoning_content").is_some());
                    rec.lock()
                        .await
                        .push(format!("rc={}", if all_have_rc { "on" } else { "off" }));
                    if !assistants.is_empty() && !all_have_rc {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({ "error": {
                                "message": "The `reasoning_content` in the thinking mode must be passed back to the API.",
                                "type": "invalid_request_error"
                            } })),
                        )
                            .into_response();
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "id": "chatcmpl-mock",
                            "object": "chat.completion",
                            "created": 0,
                            "model": "m",
                            "choices": [{
                                "index": 0,
                                "message": { "role": "assistant", "content": "ok" },
                                "finish_reason": "stop"
                            }]
                        })),
                    )
                        .into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
   async fn learns_and_retries_reasoning_content_requirement() {
       let rec: Arc<AsyncMutex<Vec<String>>> = Arc::new(AsyncMutex::new(Vec::new()));
       let upstream = spawn_mock_requiring_reasoning_rc(rec.clone()).await;
       let dir = tempfile::TempDir::new().unwrap();
       let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
       let state = ProxyState {
           session: "ses_fixed123".into(),
           zen_body_patch: is_zen_upstream(&upstream),
           base_url: upstream,
           log,
           trace: TraceSink::disabled(),
           requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
           session_map: Arc::new(SessionMap::default()),
           modes: Arc::new(ThreadModeRegistry::default()),
       };
        let probe = |state: ProxyState| async move {
            let uri: Uri = "/responses".parse().unwrap();
            let body = Body::from(
                json!({
                    "model": "m",
                    "stream": false,
                    "input": [
                        { "type": "message", "role": "user", "content": "hi" },
                        { "type": "reasoning", "id": "rs_1", "summary": [
                            { "type": "summary_text", "text": "先看代码" }
                        ] },
                        { "type": "message", "role": "assistant", "content": "我先看一下" },
                        { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}" },
                        { "type": "function_call_output", "call_id": "call_1", "output": "ok" }
                    ]
                })
                .to_string(),
            );
            handle_any(
                State(state),
                Method::POST,
                OriginalUri(uri),
                HeaderMap::new(),
                body,
            )
            .await
        };

        // 第一次：上游因缺少 reasoning_content 回 400 → 代理打开开关内部重试 → 客户端只看到 200。
        let resp = probe(state.clone()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            rec.lock().await.clone(),
            vec!["rc=off".to_string(), "rc=on".to_string()]
        );
        assert!(state.requires_reasoning_rc.load(Ordering::Relaxed));

        // 第二次：开关已粘在代理实例上 → 只发一次请求且直接带字段。
        rec.lock().await.clear();
        let resp = probe(state.clone()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(rec.lock().await.clone(), vec!["rc=on".to_string()]);

        let logged = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            logged.contains("zen_proxy.reasoning_content_enabled")
                && logged.contains("zen_proxy.forward_attempt")
                && logged.contains("assistant_with_content=1")
                && logged.contains("assistant_with_content_rc=0")
                && logged.contains("zen_proxy.forward")
                && logged.contains("reasoning_rc=on"),
            "{logged}"
        );
    }

    /// 起一个"带指定字段就用 4xx 指名拒绝、否则放行"的 mock Zen
    /// （记录请求次数与末次 body）。
    async fn spawn_mock_zen_rejecting_field(
        count: Arc<AsyncMutex<usize>>,
        last: Arc<AsyncMutex<Option<Value>>>,
        field: &'static str,
    ) -> String {
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |Json(body): Json<Value>| {
                let count = count.clone();
                let last = last.clone();
                async move {
                    let rejected = body.get(field).is_some();
                    *count.lock().await += 1;
                    *last.lock().await = Some(body);
                    if rejected {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": {
                                    "message": format!("Unrecognized request argument supplied: {field}"),
                                    "type": "invalid_request_error"
                                }
                            })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "id": "chatcmpl-mock",
                            "object": "chat.completion",
                            "model": "m",
                            "choices": [{
                                "index": 0,
                                "message": { "role": "assistant", "content": "ok" },
                                "finish_reason": "stop"
                            }]
                        })),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    /// 起一个恒定 4xx（错误体不指认任何可选字段）的 mock Zen。
    async fn spawn_mock_zen_bad_request(count: Arc<AsyncMutex<usize>>) -> String {
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |Json(_body): Json<Value>| {
                let count = count.clone();
                async move {
                    *count.lock().await += 1;
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "error": {
                                "message": "Assistant tool call function.arguments must be valid JSON.",
                                "type": "invalid_request_error"
                            }
                        })),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn streaming_request_asks_for_token_usage() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let state = proxy_state(base_url.clone(), None);
        let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
            .await
        .expect("forward 应成功");
        assert!(resp.status.is_success());
        assert!(resp.dropped_fields.is_empty(), "上游未拒绝时不应摘字段");

        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(got.body["stream_options"]["include_usage"], true);
        assert_eq!(got.body["stream"], true);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn include_usage_is_dropped_and_retried_when_upstream_rejects_it() {
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
        let base_url =
            spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "stream_options").await;

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let state = proxy_state(base_url.clone(), None);
        let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
            .await
        .expect("forward 应成功");
        assert!(resp.status.is_success());
        assert_eq!(
            resp.dropped_fields,
            vec!["stream_options"],
            "应标记被摘掉的字段"
        );
        assert_eq!(*count.lock().await, 2, "应恰好重试一次");

        let got = last.lock().await.clone().expect("mock 应已收到请求");
        assert!(got.get("stream_options").is_none(), "重试不应再带 stream_options");
        assert_eq!(got["stream"], true);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reasoning_effort_is_dropped_and_retried_when_rejected() {
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
        let base_url =
            spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "reasoning_effort").await;

        let req = json!({
            "model": "m",
            "input": "hi",
            "stream": true,
            "reasoning": { "effort": "xhigh" }
        });
        let state = proxy_state(base_url.clone(), None);
        let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
            .await
        .expect("forward 应成功");
        assert!(resp.status.is_success());
        assert_eq!(resp.dropped_fields, vec!["reasoning_effort"]);
        assert_eq!(*count.lock().await, 2, "应恰好重试一次");

        let got = last.lock().await.clone().expect("mock 应已收到请求");
        assert!(got.get("reasoning_effort").is_none(), "重试不应再带被拒字段");
        assert_eq!(
            got["stream_options"]["include_usage"], true,
            "未被拒的可选字段应保留"
        );
    }

    /// Zen 上游（补丁开启）：上游实收的 chat 请求体同时带 `max_tokens` 与 6 个假工具，
    /// 真实工具保留在前、顺序不变。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zen_body_patch_reaches_upstream_request_body() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({
            "model": "m",
            "input": "hi",
            "stream": false,
            "tools": [{
                "type": "function",
                "name": "shell",
                "description": "run shell",
                "parameters": { "type": "object", "properties": {} }
            }]
        });
        let mut state = proxy_state(base_url, None);
        state.zen_body_patch = true;
        let resp = forward(&req, &HeaderMap::new(), false, &state, "req_test")
            .await
            .expect("forward 应成功");
        assert!(resp.status.is_success());

        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert_eq!(got.body["max_tokens"], ZEN_MAX_TOKENS, "应补门禁要求的 max_tokens");
        let names: Vec<&str> = got.body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["shell", "bash", "edit", "glob", "grep", "read", "write"],
            "真实工具保留在前、缺的假工具按固定顺序补在后：{}",
            got.body
        );
        assert_eq!(
            got.body["tools"][1]["function"]["description"],
            ZEN_FAKE_TOOL_DESCRIPTION
        );
    }

    /// 非 Zen 上游（自建 / 第三方兼容端点）：同一个入站请求一个字段都不补。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zen_body_patch_is_off_for_non_zen_upstream() {
        let rec: Arc<AsyncMutex<Option<Received>>> = Arc::new(AsyncMutex::new(None));
        let base_url = spawn_mock_zen(rec.clone()).await;

        let req = json!({
            "model": "m",
            "input": "hi",
            "stream": false,
            "tools": [{
                "type": "function",
                "name": "shell",
                "description": "run shell",
                "parameters": { "type": "object", "properties": {} }
            }]
        });
        // 补丁开关由 base_url 派生：回环地址不是 Zen 上游，自然关闭
        let state = proxy_state(base_url, None);
        assert!(!state.zen_body_patch, "回环地址不应被当成 Zen 上游");
        let resp = forward(&req, &HeaderMap::new(), false, &state, "req_test")
            .await
            .expect("forward 应成功");
        assert!(resp.status.is_success());

        let got = rec.lock().await.clone().expect("mock 应已收到请求");
        assert!(
            got.body.get("max_tokens").is_none(),
            "非 Zen 上游不补 max_tokens：{}",
            got.body
        );
        let names: Vec<&str> = got.body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["shell"], "非 Zen 上游不补假工具：{}", got.body);
    }

    /// 上游在 4xx 错误体里指名 `max_tokens`：摘掉后重试一次（复用既有可选字段降级路径），
    /// 回环重试仍带工具补丁。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zen_max_tokens_is_dropped_and_retried_when_rejected() {
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
        let base_url =
            spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "max_tokens").await;

        let req = json!({ "model": "m", "input": "hi", "stream": false });
        let mut state = proxy_state(base_url, None);
        state.zen_body_patch = true;
        let resp = forward(&req, &HeaderMap::new(), false, &state, "req_test")
            .await
            .expect("forward 应成功");
        assert!(resp.status.is_success());
        assert_eq!(resp.dropped_fields, vec!["max_tokens"], "应标记被摘掉的字段");
        assert_eq!(*count.lock().await, 2, "应恰好重试一次");

        let got = last.lock().await.clone().expect("mock 应已收到请求");
        assert!(got.get("max_tokens").is_none(), "重试不应再带 max_tokens：{got}");
        assert_eq!(
            got["tools"].as_array().unwrap().len(),
            ZEN_REQUIRED_TOOL_NAMES.len(),
            "工具补丁不受可选字段摘除影响：{got}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unrelated_4xx_is_not_retried() {
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let base_url = spawn_mock_zen_bad_request(count.clone()).await;

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let state = proxy_state(base_url.clone(), None);
        let resp = forward(&req, &HeaderMap::new(), true, &state, "req_test")
            .await
        .expect("forward 应成功");
        assert!(!resp.status.is_success());
        assert!(resp.dropped_fields.is_empty(), "未指名可选字段时不应摘字段");
        assert_eq!(*count.lock().await, 1, "不应重试");
        assert!(
            resp.reasoning_rc_change.is_none(),
            "未提及 reasoning_content 时不应翻转回传开关"
        );
        assert!(!resp.reasoning_rc);
        match resp.payload {
            ForwardPayload::Text(text) => {
                assert!(text.contains("valid JSON"), "错误体应原样透传：{text}")
            }
            ForwardPayload::Live(_) => panic!("4xx 应携带已读出的错误体"),
        }
    }

    /// 起一个"发一片 finish_reason 后就保持连接不关"的 mock 上游。
    async fn spawn_mock_upstream_holding_open() -> String {
        let app = Router::new().route(
            "/stream",
            axum::routing::get(|| async {
                let first = futures_util::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                        b"data: {\"choices\":[{\"delta\":{\"content\":\"half\"},\"finish_reason\":\"stop\"}]}\n\n",
                    ))
                });
                let rest = futures_util::stream::pending::<
                    Result<axum::body::Bytes, std::io::Error>,
                >();
                axum::response::Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(first.chain(rest)))
                    .unwrap()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finish_without_more_chunks_times_out_and_completes() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = Some(Arc::new(SessionLog::new(dir.path().to_path_buf())));
        let base_url = spawn_mock_upstream_holding_open().await;
        let resp = http_client()
            .get(format!("{base_url}/stream"))
            .send()
            .await
            .expect("mock 上游应可连接");

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let started = std::time::Instant::now();
        let state = proxy_state(base_url.clone(), log.clone());
        let out = proxy_stream_response(
            state,
            HeaderMap::new(),
            "req_test".to_string(),
            req,
            resp,
            TraceCall::disabled(),
        )
        .await;
        let body = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            axum::body::to_bytes(out.into_body(), usize::MAX),
        )
        .await
        .expect("宽限超时后应已收尾，不能挂住回合")
        .unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("event: response.completed"),
            "应正常收尾：{text}"
        );
        assert!(
            started.elapsed() >= USAGE_GRACE,
            "应先等满宽限期再收尾，实际 {:?}",
            started.elapsed()
        );

        let joined = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("zen_proxy.usage_timeout"),
            "应记录宽限超时：{joined}"
        );
        assert!(joined.contains("usage=none"), "{joined}");
    }

    // -----------------------------------------------------------------------
    // 内容诊断日志（logs/zen）：请求体 + 上游响应原文 + 收尾事件 + 可疑标记
    // -----------------------------------------------------------------------

    /// trace 目录里的全部文件（按文件名排序）。
    fn trace_files(dir: &std::path::Path) -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(entry.path()).unwrap_or_default(),
                )
            })
            .collect();
        files.sort();
        files
    }

    fn trace_file_with(dir: &std::path::Path, suffix: &str) -> (String, String) {
        trace_files(dir)
            .into_iter()
            .find(|(name, _)| name.ends_with(suffix))
            .unwrap_or_else(|| panic!("应存在 {suffix} 文件"))
    }

    fn traced_sink(dir: &std::path::Path) -> TraceSink {
        TraceSink::new(Arc::new(crate::codex::zen_trace::ZenTrace::new(
            dir.to_path_buf(),
        )))
    }

    /// 起一个返回固定 SSE 原文的 mock Zen（POST /chat/completions）。
    async fn spawn_mock_zen_sse(chunks: Vec<&'static str>) -> String {
        let app = Router::new().route(
            "/chat/completions",
            axum::routing::post(move |Json(_body): Json<Value>| {
                let chunks = chunks.clone();
                async move {
                    let stream = futures_util::stream::iter(
                        chunks
                            .into_iter()
                            .map(|chunk| {
                                Ok::<_, std::io::Error>(axum::body::Bytes::from(chunk.to_string()))
                            })
                            .collect::<Vec<_>>(),
                    );
                    axum::response::Response::builder()
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .body(Body::from_stream(stream))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[test]
    fn suspicious_flags_cover_each_branch() {
        let flags = |facts: SuspiciousFacts<'_>| flags_field(&suspicious_flags(&facts));
        // 正常收尾：带工具调用 + 有 usage → 无标记
        assert_eq!(
            flags(SuspiciousFacts {
                finish_reason: Some("tool_calls"),
                call_count: 2,
                usage_present: true,
                ..Default::default()
            }),
            "-"
        );
        // 模型什么都没输出就 stop（最符合「任务未完成就结束」）
        assert_eq!(
            flags(SuspiciousFacts {
                finish_reason: Some("stop"),
                usage_present: true,
                ..Default::default()
            }),
            "stop_without_output"
        );
        // 只输出文本、没有工具调用
        assert_eq!(
            flags(SuspiciousFacts {
                finish_reason: Some("stop"),
                text_chars: 12,
                usage_present: true,
                ..Default::default()
            }),
            "stop_without_tool_call"
        );
        // 上游截断
        assert_eq!(
            flags(SuspiciousFacts {
                finish_reason: Some("length"),
                usage_present: true,
                ..Default::default()
            }),
            "truncated"
        );
        // 本次历史里已有失败的补丁调用（模型在补丁格式上打转）
        assert_eq!(
            flags(SuspiciousFacts {
                finish_reason: Some("tool_calls"),
                call_count: 1,
                usage_present: true,
                patch_retry: true,
                ..Default::default()
            }),
            "patch_retry"
        );
        // 失败收尾（无 finish_reason）
        assert_eq!(
            flags(SuspiciousFacts {
                failed: true,
                usage_present: true,
                ..Default::default()
            }),
            "failed,finish_reason_missing"
        );
        // 缺 usage / 宽限超时 / 上游非 2xx
        assert_eq!(
            flags(SuspiciousFacts {
                finish_reason: Some("tool_calls"),
                call_count: 1,
                ..Default::default()
            }),
            "usage_missing"
        );
        assert_eq!(
            flags(SuspiciousFacts {
                finish_reason: Some("tool_calls"),
                call_count: 1,
                usage_present: true,
                usage_timeout: true,
                ..Default::default()
            }),
            "usage_timeout"
        );
        assert_eq!(
            flags(SuspiciousFacts {
                upstream_http_error: true,
                usage_present: true,
                ..Default::default()
            }),
            "upstream_http_error,finish_reason_missing"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trace_records_stream_request_response_and_summary() {
        let dir = tempfile::TempDir::new().unwrap();
        let upstream = spawn_mock_zen_sse(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        ])
        .await;
        let state = proxy_state_traced(upstream, None, traced_sink(dir.path()));
        let resp = handle_any(
            State(state),
            Method::POST,
            OriginalUri("/responses".parse().unwrap()),
            HeaderMap::new(),
            Body::from(
                json!({
                    "model": "mimo-v2.5-flash",
                    "stream": true,
                    "input": [{
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": "hi" }]
                    }]
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let text = body_text(resp).await;
        assert!(text.contains("event: response.completed"), "{text}");

        let (_, request) = trace_file_with(dir.path(), ".request.json");
        assert!(
            request.contains("\"model\": \"mimo-v2.5-flash\""),
            "{request}"
        );
        assert!(request.contains("\"role\": \"user\""), "{request}");
        assert!(request.contains("\"content\": \"hi\""), "{request}");
        assert!(request.contains("\"include_usage\": true"), "{request}");

        let (_, response) = trace_file_with(dir.path(), ".response.sse");
        assert!(
            response.contains("data: [DONE]"),
            "上游行应原样落盘：{response}"
        );
        assert!(response.contains("\"finish_reason\":\"stop\""), "{response}");

        let (_, summary) = trace_file_with(dir.path(), ".summary.txt");
        assert!(summary.contains("call_id=req_"), "{summary}");
        assert!(summary.contains("attempt=1"), "{summary}");
        assert!(summary.contains("upstream_url="), "{summary}");
        assert!(summary.contains("authorization=absent"), "{summary}");
        assert!(summary.contains("finish_reason=stop"), "{summary}");
        assert!(summary.contains("usage=present"), "{summary}");
        // 剥离口径进摘要：普通回复没有被剥离的字符
        assert!(summary.contains("stripped_chars=0"), "{summary}");
        assert!(
            summary.contains("suspicious=stop_without_tool_call"),
            "{summary}"
        );
        assert!(
            summary.contains("translated_terminal_event=event: response.completed"),
            "{summary}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trace_marks_stop_without_output_as_suspicious() {
        let dir = tempfile::TempDir::new().unwrap();
        let upstream = spawn_mock_zen_sse(vec![
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ])
        .await;
        let state = proxy_state_traced(upstream, None, traced_sink(dir.path()));
        let resp = handle_any(
            State(state),
            Method::POST,
            OriginalUri("/responses".parse().unwrap()),
            HeaderMap::new(),
            Body::from(json!({ "model": "m", "stream": true, "input": "hi" }).to_string()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let text = body_text(resp).await;
        assert!(text.contains("event: response.completed"), "{text}");

        let (_, summary) = trace_file_with(dir.path(), ".summary.txt");
        assert!(
            summary.contains("suspicious=stop_without_output,usage_missing"),
            "{summary}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trace_records_each_retry_attempt() {
        let dir = tempfile::TempDir::new().unwrap();
        let trace = traced_sink(dir.path());
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let last: Arc<AsyncMutex<Option<Value>>> = Arc::new(AsyncMutex::new(None));
        let base_url =
            spawn_mock_zen_rejecting_field(count.clone(), last.clone(), "stream_options").await;

        let req = json!({ "model": "m", "input": "hi", "stream": true });
        let state = proxy_state_traced(base_url.clone(), None, trace);
        let resp = forward(&req, &HeaderMap::new(), true, &state, "req_retry")
            .await
        .expect("forward 应成功");
        assert!(resp.status.is_success());
        drop(resp);

        let files = trace_files(dir.path());
        let a1_request = files
            .iter()
            .find(|(name, _)| name.contains("req_retry-a1.request.json"))
            .expect("第一次尝试应有请求体")
            .clone();
        let a2_request = files
            .iter()
            .find(|(name, _)| name.contains("req_retry-a2.request.json"))
            .expect("重试应有独立请求体")
            .clone();
        assert!(a1_request.1.contains("stream_options"), "{}", a1_request.1);
        assert!(
            !a2_request.1.contains("stream_options"),
            "重试不应再带被拒字段：{}",
            a2_request.1
        );
        let a1_summary = files
            .iter()
            .find(|(name, _)| name.contains("req_retry-a1.summary.txt"))
            .expect("第一次尝试应有摘要")
            .clone();
        assert!(a1_summary.1.contains("retried=true"), "{}", a1_summary.1);
        assert!(
            a1_summary.1.contains("suspicious=upstream_http_error"),
            "{}",
            a1_summary.1
        );
        let (_, a1_error) = trace_file_with(dir.path(), "req_retry-a1.response.txt");
        assert!(a1_error.contains("stream_options"), "{a1_error}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trace_records_upstream_error_body() {
        let dir = tempfile::TempDir::new().unwrap();
        let count: Arc<AsyncMutex<usize>> = Arc::new(AsyncMutex::new(0));
        let upstream = spawn_mock_zen_bad_request(count.clone()).await;
        let state = proxy_state_traced(upstream, None, traced_sink(dir.path()));
        let resp = handle_any(
            State(state),
            Method::POST,
            OriginalUri("/responses".parse().unwrap()),
            HeaderMap::new(),
            responses_probe(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let (_, error_body) = trace_file_with(dir.path(), ".response.txt");
        assert!(error_body.contains("must be valid JSON"), "{error_body}");
        assert_eq!(
            error_body.matches("must be valid JSON").count(),
            1,
            "不可重试的上游错误只应落盘一次：{error_body}"
        );
        let (_, summary) = trace_file_with(dir.path(), ".summary.txt");
        assert!(summary.contains("upstream_status=400"), "{summary}");
        assert!(
            summary.contains("suspicious=upstream_http_error"),
            "{summary}"
        );
    }
}
