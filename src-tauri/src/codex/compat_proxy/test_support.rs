//! 两个测试模块共用的测试工具与夹具常量（仅测试期编译）。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::*;

pub(crate) const REAL_PLAN_MODE_BLOCK: &str = "<collaboration_mode># Plan Mode (Conversational)\r\n\r\nYou work in 3 phases, and you should *chat your way* to a great plan before finalizing it.\r\n\r\n## Mode rules (strict)\r\n\r\nYou are in **Plan Mode** until a developer message explicitly ends it.\r\n\r\nEventually issuing a `<proposed_plan>` block.\r\n</collaboration_mode>";
/// codex 0.154 实测的默认模式协作块：正文第一句就含 `(e.g. Plan mode)`，只认标题行才不会误判。
pub(crate) const REAL_DEFAULT_MODE_BLOCK: &str = "<collaboration_mode># Collaboration Mode: Default\r\n\r\nYou are now in Default mode. Any previous instructions for other modes (e.g. Plan mode) are no longer active.\r\n\r\nYour active mode changes only when new developer instructions with a different `<collaboration_mode>...</collaboration_mode>` change it.\r\n</collaboration_mode>";
/// 测试用代理状态：基址可指向本地 mock。
pub(crate) fn test_proxy_state(base_url: &str, log: ZenLog, trace: TraceSink) -> ProxyState {
    ProxyState {
        base_url: base_url.to_string(),
        client: reqwest::Client::new(),
        log,
        trace,
        requires_reasoning_rc: Arc::new(AtomicBool::new(false)),
        nudge_enabled: true,
        protocols: protocol_cache(),
        modes: Arc::new(ThreadModeRegistry::default()),
    }
}
