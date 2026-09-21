//! 知识库提示注入：把"本机有 `codexui-kb` 知识库 CLI"这件事告诉 agent。
//!
//! 注入位置是 `turn/start` 的 `additionalContext`——协议定义为"客户端提供的上下文片段，
//! 按不透明来源标识分组"，形状 `{ [来源]: { value: string, kind: "untrusted" | "application" } }`。
//!
//! 为什么不用会话级 `developerInstructions`：该字段是**替换**语义。docs/协议盘点.md 记录
// `thread/start|resume` 的 `baseInstructions`（用户指令）与 `developerInstructions`（开发者指令），
// 且 `turn/start` 的 `collaborationMode.settings.developer_instructions` 每轮都能覆盖它；
// codex-ui 每轮固定发 `developer_instructions: null`（计划模式还需保持 null，避免覆盖 codex
// 自带的 plan 开发者指令），所以会话级注入会被逐轮冲掉。逐回合注入 `additionalContext` 是
// 追加语义，不影响既有指令。

use serde_json::{json, Value};

/// `additionalContext` 里的来源标识（同一会话内固定，便于服务端按来源归并）
pub const CONTEXT_KEY: &str = "codexui-kb";

/// 注入文本：固定不变，只讲"有什么工具、怎么用、什么时候用"。
/// 刻意不写"用会话工作目录匹配库的来源目录"这类规则——来源目录只表明覆盖的资料范围，
/// 实际常与使用者工作目录不同；选哪个库交给 agent（用户点名就用点名的，没点名先 list）。
pub const KB_HINT: &str = concat!(
    "【本机知识库】这台机器上有离线知识库命令行工具 codexui-kb（已在 PATH 上，直接调用命令名；",
    "数据在 %APPDATA%\\com.codexui.app\\kbs，向量模型与程序同目录）：",
    "`codexui-kb list` 列出全部库（库名、来源目录、文档数/切块数、更新时间）；",
    "`codexui-kb search <库名> --query <检索词> [--top-k 8]` 混合检索（向量 + 关键词）并返回带出处的片段；",
    "`codexui-kb index <库名> [--full]` 按已登记来源建库/增量更新；",
    "`codexui-kb create <库名> <目录>` 登记库与资料目录；`codexui-kb delete <库名>` 删除索引。",
    "输出为 NDJSON（若干 progress 行 + 最后一条 result 或 error），退出码 0 成功、2 参数错误、3 模型未就绪、4 库被占用。",
    "用法约定：回答故障码、型号、操作步骤这类需要事实依据的问题前先查知识库；",
    "用户点名了库就用该库，没点名就先 list 看有哪些库（来源目录只表明覆盖的资料范围，不一定与会话工作目录相同），仍不确定就向用户确认；",
    "回答里标注命中片段的 doc_path 与 title_path；库里查不到可用内容时，提示用户可用 `create <库名> <目录>` 建库。"
);

/// 给 `turn/start` 的参数补上知识库提示，返回是否改动了参数。
///
/// - 非对象参数（`null`/数组/标量）原样返回 `false`；
/// - 已带 `additionalContext.<CONTEXT_KEY>` 时不覆盖（幂等，尊重调用方）；
/// - `additionalContext` 存在但不是对象时不擅自改写（保持调用方给的形状）；
/// - 其余情况写入 `{ value: KB_HINT, kind: "application" }`，并保留其它来源的片段。
pub fn apply_turn_params(params: &mut Value) -> bool {
    let Some(obj) = params.as_object_mut() else {
        return false;
    };
    let entry = obj
        .entry("additionalContext")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(map) = entry.as_object_mut() else {
        return false;
    };
    if map.contains_key(CONTEXT_KEY) {
        return false;
    }
    map.insert(
        CONTEXT_KEY.to_string(),
        json!({ "value": KB_HINT, "kind": "application" }),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_context_when_missing() {
        let mut params = json!({ "threadId": "t1", "input": [] });
        assert!(apply_turn_params(&mut params));
        let entry = params
            .pointer("/additionalContext/codexui-kb")
            .expect("应写入 codexui-kb 片段");
        assert_eq!(entry.get("kind").and_then(Value::as_str), Some("application"));
        assert_eq!(entry.get("value").and_then(Value::as_str), Some(KB_HINT));
        // 原有字段保持不变
        assert_eq!(params.get("threadId").and_then(Value::as_str), Some("t1"));
    }

    #[test]
    fn is_idempotent_and_keeps_other_sources() {
        let mut params = json!({
            "additionalContext": { "other-app": { "value": "别的上下文", "kind": "application" } }
        });
        assert!(apply_turn_params(&mut params));
        assert!(
            !apply_turn_params(&mut params),
            "第二次调用不应重复写入"
        );
        assert!(params.pointer("/additionalContext/other-app").is_some());
        let ours = params
            .pointer("/additionalContext/codexui-kb/value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(ours.matches("codexui-kb list").count(), 1);
    }

    #[test]
    fn skips_when_caller_already_set_our_key() {
        let mut params = json!({
            "additionalContext": { "codexui-kb": { "value": "调用方自备", "kind": "untrusted" } }
        });
        assert!(!apply_turn_params(&mut params));
        assert_eq!(
            params
                .pointer("/additionalContext/codexui-kb/value")
                .and_then(Value::as_str),
            Some("调用方自备")
        );
    }

    #[test]
    fn leaves_non_object_params_and_odd_shapes_untouched() {
        let mut null_params = Value::Null;
        assert!(!apply_turn_params(&mut null_params));
        assert_eq!(null_params, Value::Null);

        let mut array_params = json!([1, 2]);
        assert!(!apply_turn_params(&mut array_params));
        assert_eq!(array_params, json!([1, 2]));

        let mut odd = json!({ "additionalContext": "不是对象" });
        assert!(!apply_turn_params(&mut odd));
        assert_eq!(odd, json!({ "additionalContext": "不是对象" }));
    }

    #[test]
    fn hint_text_covers_cli_and_avoids_directory_binding_rule() {
        for needle in [
            "codexui-kb list",
            "codexui-kb search",
            "codexui-kb index",
            "codexui-kb create",
            "doc_path",
            "title_path",
        ] {
            assert!(KB_HINT.contains(needle), "提示应包含 {needle}");
        }
        // 明确不引入"按工作目录/来源目录绑定选择库"的规则
        for forbidden in ["工作目录匹配", "匹配 source", "按工作目录", "cwd"] {
            assert!(
                !KB_HINT.contains(forbidden),
                "提示不应包含目录绑定规则：{forbidden}"
            );
        }
        assert!(KB_HINT.chars().count() < 700, "提示应保持简短");
    }
}
