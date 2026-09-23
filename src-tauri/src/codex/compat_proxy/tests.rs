//! `compat_proxy` 的单元测试（原 `compat_proxy.rs` 内联测试模块拆出）。

use super::test_support::*;
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
    assert_eq!(
        last_mode_block_declares_plan(REAL_PLAN_MODE_BLOCK),
        Some(true)
    );

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
    assert_eq!(
        last_mode_block_declares_plan("普通文本，没有模式标签"),
        None
    );

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
        last_user_text(
            &json!({ "input": [{ "type": "message", "role": "assistant", "content": "x" }] })
        ),
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

    // 默认模式提醒：两条路径（继续干 / 用标签宣告结束），并教出成对标签
    assert!(
        NUDGE_TEXT.contains("必须实际调用工具"),
        "必须保留执行口径，否则弱模型继续只写承诺：{NUDGE_TEXT}"
    );
    assert!(
        NUDGE_TEXT.contains("不得用文字描述、计划或承诺代替工具调用")
            && NUDGE_TEXT.contains("就不得直接输出完成标签来逃避执行"),
        "必须堵住「只写承诺」与「提前甩完成标签」两条逃避路径：{NUDGE_TEXT}"
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
        NUDGE_TEXT.contains("仅输出一行闭合标签"),
        "收尾轮必须是「只回这一行标签」：{NUDGE_TEXT}"
    );
    assert!(
        is_task_completed(NUDGE_TEXT),
        "提醒文本自身就带标记，前缀判据必须命中：{NUDGE_TEXT}"
    );
    // 结构不变量：未来的代理层过滤/分桶只依赖这些（与载荷词汇无关）
    for needle in [
        "成对闭合",
        "独占一行",
        "不换行",
        "本轮最多一个",
        "不得放入代码块",
        "不得改写标签",
        "标签外不得有任何其他字符",
    ] {
        assert!(
            NUDGE_TEXT.contains(needle),
            "提醒必须写明标签结构约定「{needle}」：{NUDGE_TEXT}"
        );
    }
    // 默认模式提醒不教计划标签（模式教学不串味）
    assert!(!NUDGE_TEXT.contains(PLAN_OUTPUT_MARKER), "{NUDGE_TEXT}");
    assert!(!NUDGE_TEXT.contains(PLAN_CANCEL_MARKER), "{NUDGE_TEXT}");
    assert!(
        !NUDGE_TEXT.contains(PLAN_UNACHIEVABLE_MARKER),
        "{NUDGE_TEXT}"
    );

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
    // 结构不变量（计划模式提醒沿用附件原文措辞，与默认提醒同义）
    for needle in [
        "成对闭合",
        "各自独占一行",
        "不换行",
        "本轮最多只写一个",
        "不要改写标签",
    ] {
        assert!(
            PLAN_NUDGE_TEXT.contains(needle),
            "计划提醒必须写明标签结构约定「{needle}」：{PLAN_NUDGE_TEXT}"
        );
    }
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
        PLAN_NUDGE_TEXT.contains("都必须给出完整方案")
            || PLAN_NUDGE_TEXT.contains("必须想办法把完整方案"),
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
            assert!(
                !text.contains(legacy),
                "残留旧标签字面量「{legacy}」：{text}"
            );
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
        "本轮最多一个",
        "不得放入代码块",
        "不得改写标签",
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
    // 契约必须让模型照常输出结论正文：只有催办提醒（上一轮正文已展示）才要求
    // 「标签外不得有任何其他字符」，契约只要求收尾标签独占一行、正文写在标签之前
    assert!(
        DEFAULT_MODE_CONTRACT_TEXT.contains("结论正文写在标签之前"),
        "契约不能让模型为了回标签而丢掉结论正文：{DEFAULT_MODE_CONTRACT_TEXT}"
    );
    for text in [DEFAULT_MODE_CONTRACT_TEXT, PLAN_MODE_CONTRACT_TEXT] {
        assert!(
            !text.contains("标签外不得有任何其他字符"),
            "契约不得照搬催办轮的「标签外不得有任何其他字符」（会把正文一起禁掉）：{text}"
        );
        assert!(
            text.contains("标签那一行不得再有任何其他字符"),
            "契约要写清「标签独占一行、该行不再有别的字符」：{text}"
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
    for (case, label) in cases
        .into_iter()
        .zip(["固定短词", "一整句话", "英文", "空载荷", "乱码"])
    {
        let (visible, payload) = TagStripper::strip_once(case, true);
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
        true,
    );
    assert_eq!(visible, "已补好测试。\n", "整行标签连换行一起删");
    assert_eq!(payload, "我判断已经完成了", "日志里记实际载荷");

    // 行内标签：只删标签本身，同行其余文本保留
    let (visible, _) = TagStripper::strip_once(
        "已完成改动<zen_task_completed>已完成</zen_task_completed>，请查看",
        true,
    );
    assert_eq!(visible, "已完成改动，请查看");

    // 大小写不敏感（判据同口径）
    let (visible, _) =
        TagStripper::strip_once("收尾\n<ZEN_TASK_COMPLETED>DONE</ZEN_TASK_COMPLETED>", true);
    assert!(
        !visible.to_lowercase().contains("zen_task_completed"),
        "{visible}"
    );

    // 一行里多个标签逐个处理
    let (visible, payload) = TagStripper::strip_once(
        "前言\n<zen_plan_cancelled>已放弃</zen_plan_cancelled><zen_task_completed>已完成</zen_task_completed>\n",
        true,
    );
    assert!(!visible.to_lowercase().contains("<zen_"), "{visible}");
    assert!(
        payload.contains("已放弃") && payload.contains("已完成"),
        "{payload}"
    );
}

/// 跨分片、写错闭标签、未闭合这几种弱模型常见形态。
#[test]
fn tag_stripper_handles_split_chunks_and_odd_closers() {
    // 逐字符喂：开闭标签都被切碎也不能漏出去
    let raw = "正文\n<zen_task_completed>已完成</zen_task_completed>";
    let mut stripper = TagStripper::new(true);
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
        true,
    );
    assert_eq!(visible, "正文\n尾部", "容错闭合后保留后续正文：{visible}");
    assert_eq!(payload, "已完成");

    // 未闭合（流结束仍在标签里）：丢掉标签开头到结尾，载荷照样进日志
    let mut stripper = TagStripper::new(true);
    let mut visible = stripper.feed("正文\n<zen_task_completed>没有闭合");
    visible.push_str(&stripper.finish());
    assert_eq!(visible, "正文\n");
    assert_eq!(stripper.stripped_note(), "没有闭合");

    // 只是「看起来像标签开头」的普通文本：原样下发（不能被吞掉）
    let mut stripper = TagStripper::new(true);
    let mut visible = stripper.feed("这里有 <zen_ 但不是标签");
    visible.push_str(&stripper.finish());
    assert_eq!(visible, "这里有 <zen_ 但不是标签");
    assert!(stripper.stripped_note().is_empty());

    // 剥掉 `proposed_plan` 之外的正文不受影响（代理不碰 codex 自己的标签）
    let (visible, _) = TagStripper::strip_once(
        "<proposed_plan>\n1. 做 A\n</proposed_plan>\n<zen_task_completed>已完成</zen_task_completed>",
        true,
    );
    assert!(visible.contains("<proposed_plan>"), "{visible}");
    assert!(
        !visible.to_lowercase().contains("zen_task_completed"),
        "{visible}"
    );
}

/// 「回合收尾约束和助推」关闭时剥离器直通：标签原样下发、不记载荷，正文一字不改。
#[test]
fn tag_stripper_passes_through_when_nudge_disabled() {
    let raw = "结论。\n<zen_task_completed>已完成</zen_task_completed>\n";
    let (visible, payload) = TagStripper::strip_once(raw, false);
    assert_eq!(visible, raw, "关闭时可见文本必须与上游原文逐字一致");
    assert!(payload.is_empty(), "关闭时不记标签载荷：{payload}");

    // 逐分片喂（直通模式下不存在 holdback，每片立即返回）
    let mut stripper = TagStripper::new(false);
    let mut out = String::new();
    for ch in raw.chars() {
        out.push_str(&stripper.feed(&ch.to_string()));
    }
    out.push_str(&stripper.finish());
    assert_eq!(out, raw);
    assert!(stripper.stripped_note().is_empty());
}

/// 非流式翻译：同一段带标签的响应，开关开时剥离、关时原样保留。
#[test]
fn chat_to_responses_honors_strip_switch() {
    let chat = json!({
        "id": "chatcmpl-switch",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "结论：无需改动。\n<zen_task_completed>无需改动</zen_task_completed>"
            },
            "finish_reason": "stop"
        }]
    });
    let text_of = |strip: bool| {
        chat_to_responses(&chat, "m", &ToolShape::default(), strip).unwrap()["output"][0]["content"]
            [0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(text_of(true), "结论：无需改动。\n", "开启时剥离标签");
    assert_eq!(
        text_of(false),
        "结论：无需改动。\n<zen_task_completed>无需改动</zen_task_completed>",
        "关闭时原样保留标签"
    );
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
    }
    // 默认模式的**催办提醒**改用「只输出一行标签 + 固定词替换表」的写法，
    // 「不要写别的说明」只在首轮教学契约里（提醒靠「标签外不得有任何其他字符」兜住）
    assert!(
        NUDGE_TEXT.contains("仅输出一行闭合标签") && NUDGE_TEXT.contains("必须替换为以下四者之一"),
        "默认提醒必须教出「只回一行标签 + 四个固定词」：{NUDGE_TEXT}"
    );
    assert!(
        DEFAULT_MODE_CONTRACT_TEXT.contains("不要写别的说明"),
        "契约必须禁止标签里写长说明（省 token）：{DEFAULT_MODE_CONTRACT_TEXT}"
    );
    for text in [PLAN_NUDGE_TEXT, PLAN_MODE_CONTRACT_TEXT] {
        assert!(
            text.contains("<zen_plan_cancelled>已放弃</zen_plan_cancelled>"),
            "计划模式教学应给出固定短词：{text}"
        );
        assert!(
            text.contains("<zen_plan_unachievable>无法计划</zen_plan_unachievable>"),
            "计划模式教学应给出固定短词：{text}"
        );
        assert!(
            text.contains("不要写别的说明"),
            "计划模式教学必须禁止标签里写长说明（省 token）：{text}"
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
    assert_eq!(normalize_base_url(""), DEFAULT_COMPAT_BASE_URL);
    assert_eq!(normalize_base_url("   "), DEFAULT_COMPAT_BASE_URL);
    assert_eq!(normalize_base_url("/"), DEFAULT_COMPAT_BASE_URL);
}

#[test]
fn responses_path_follows_base_url_path() {
    assert_eq!(responses_path("https://api.deepseek.com"), "/responses");
    assert_eq!(responses_path("https://api.deepseek.com/"), "/responses");
    assert_eq!(
        responses_path("https://opencode.ai/zen/v1"),
        "/zen/v1/responses"
    );
    assert_eq!(
        responses_path("https://opencode.ai/zen/v2/"),
        "/zen/v2/responses"
    );
    assert_eq!(responses_path("https://a/v1"), "/v1/responses");
    // 空/非法地址回退默认上游，派生出默认路径。
    assert_eq!(responses_path(""), responses_path(DEFAULT_COMPAT_BASE_URL));
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
    assert_eq!(
        repairs,
        RepairReport::default(),
        "配对完整的工具调用不应触发净化"
    );
    assert_eq!(chat["stream"], true);
    assert_eq!(chat["messages"].as_array().unwrap().len(), 4);
    assert_eq!(chat["messages"][0]["role"], "system");
    assert_eq!(chat["messages"][0]["content"], "你是一个助手");
    assert_eq!(chat["messages"][1]["content"], "请调用工具");
    assert_eq!(chat["messages"][2]["role"], "assistant");
    assert_eq!(chat["messages"][2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(
        chat["messages"][2]["tool_calls"][0]["function"]["name"],
        "search"
    );
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
    let resp = chat_to_responses(&chat, "zen/gpt-5-mini", &ToolShape::default(), true).unwrap();
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
    let resp = chat_to_responses(&chat, "zen/gpt-5-mini", &ToolShape::default(), true).unwrap();
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
    let resp = chat_to_responses(&chat, "m", &ToolShape::default(), true).unwrap();
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
    let events: Vec<&str> = lines.iter().filter_map(|b| b.lines().next()).collect();
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
            Some(
                serde_json::from_str::<Value>(data.trim()).ok()?["delta"]
                    .as_str()?
                    .to_string(),
            )
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
        vec![
            "exec_command",
            "codexui_add_scheduled_task",
            "codexui_get_usage"
        ],
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
        .map(|l| serde_json::from_str::<Value>(l.split("data:").nth(1).unwrap().trim()).unwrap())
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
    let out = chat_to_responses(&chat_resp, "m", &shape, true).unwrap();
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
    assert!(
        desc.contains("Never write `@@ some description @@`"),
        "{desc}"
    );
    assert!(
        desc.contains("Edit files (apply_patch patch language)."),
        "{desc}"
    );
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
    assert!(
        desc.contains("Edit files (other_tool patch language)."),
        "{desc}"
    );
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
    assert!(
        patch_text.contains("Never write `@@ some description @@`"),
        "{patch_text}"
    );
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
    assert_eq!(
        chat["messages"][1]["content"],
        json!("apply_patch verification failed: Failed to find context 'x @@'")
    );
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
    assert!(
        joined.contains("event: response.output_item.added"),
        "{joined}"
    );
    assert!(joined.contains(r#""type":"custom_tool_call""#), "{joined}");
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
    let v: Value = serde_json::from_str(done.split("data:").nth(1).unwrap().trim()).unwrap();
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
    let out = chat_to_responses(&chat, "m", &tool_shape(&custom_tool_request()), true).unwrap();
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
    let state = test_proxy_state(
        "ses_fixed123",
        "http://127.0.0.1:1",
        None,
        TraceSink::disabled(),
    );
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
    invalid.insert("session-id", HeaderValue::from_bytes(b"caf\xe9").unwrap());
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
        map.resolve(&format!("thread-{index}"))
            .expect("应铸出会话 id");
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
        "compat_proxy.request",
        &[
            ("model", "zen/gpt-5-mini".to_string()),
            ("stream", "true".to_string()),
        ],
    );
    log_at(
        &log,
        "warn",
        "compat_proxy.forward_error",
        &[("error", "上游请求失败：TLS".to_string())],
    );

    let files = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect::<Vec<_>>();
    let joined = files.join("\n");
    assert!(joined.contains("event=compat_proxy.request"));
    assert!(joined.contains("event=compat_proxy.forward_error"));
    assert!(joined.contains("model=zen/gpt-5-mini"));
    // 安全字段：不应包含 Authorization / 敏感内容字样
    assert!(!joined.contains("Authorization"));
    assert!(!joined.contains("Bearer"));
}

#[test]
fn log_at_none_or_bad_dir_is_silent() {
    // 无句柄：不 panic
    log_at(&None, "info", "compat_proxy.request", &[]);
    // 日志目录不可写（路径指向一个普通文件）：静默不 panic
    let dir = tempfile::TempDir::new().unwrap();
    let blocker = dir.path().join("blocked");
    std::fs::write(&blocker, b"x").unwrap();
    let log = Some(Arc::new(SessionLog::new(blocker)));
    log_at(&log, "info", "compat_proxy.request", &[]);
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
    assert_eq!(
        failed["response"]["error"]["code"],
        CODE_MALFORMED_TOOL_CALL
    );
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
    assert_eq!(
        failed["response"]["error"]["code"],
        CODE_UPSTREAM_STREAM_ERROR
    );
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
    let err = chat_to_responses(&chat, "m", &ToolShape::default(), true).unwrap_err();
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
    assert_eq!(
        value_of("stream_options"),
        Some(json!({ "include_usage": true }))
    );
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
    assert_eq!(
        tool_choice_to_chat(Some(&json!({ "type": "allowed_tools" }))),
        None
    );
    assert_eq!(tool_choice_to_chat(Some(&Value::Null)), None);
    assert_eq!(tool_choice_to_chat(None), None);
}

// ---------- Zen 免费层请求体门禁：形状补丁 ----------

/// 请求体门禁补丁的开关**只看**「OpenCode 客户端身份」这一项：开即补（无论上游
/// host 是 opencode、DeepSeek 还是回环地址），关即不补；收尾开关不影响它。
#[test]
fn zen_body_patch_follows_identity_switch_only() {
    // 上游地址（含 Zen / 第三方 / 回环 / 非法）不参与判据，只有身份开关决定是否补形状
    for base_url in [
        "https://opencode.ai/zen/v1",
        "https://api.deepseek.com/v1",
        "http://127.0.0.1:18080/zen/v1",
        "not-a-url",
        "",
    ] {
        assert!(
            CompatProxyConfig::new(18080, base_url, true, true).opencode_identity,
            "身份开启即补形状（不看上游）：{base_url}"
        );
        assert!(
            !CompatProxyConfig::new(18080, base_url, false, true).opencode_identity,
            "身份关闭即不补形状：{base_url}"
        );
    }
    // 收尾开关是另一个独立维度：它不参与「补不补形状」
    assert!(
        CompatProxyConfig::new(18080, "https://opencode.ai/zen/v1", true, false).opencode_identity
    );
}

/// 身份开关落地到请求头：开时发四个识别头 + opencode UA；关时一个识别头都不发，
/// 并把入站自带的同名头剔除、UA 透传入站值。
#[test]
fn opencode_identity_headers_follow_the_switch() {
    let mut inbound = HeaderMap::new();
    inbound.insert(header::USER_AGENT, HeaderValue::from_static("codex/1.2.3"));
    // 恶意/异常客户端自带识别头：关闭时必须被剔除
    inbound.insert(
        "x-opencode-session",
        HeaderValue::from_static("ses_attacker_supplied"),
    );

    let on = test_proxy_state(
        "ses_fixed123",
        "https://opencode.ai/zen/v1",
        None,
        TraceSink::disabled(),
    );
    let mut out = HeaderMap::new();
    apply_opencode_identity(&mut out, &inbound, &on, "ses_fixed123", "msg_fixed");
    assert_eq!(out.get("x-opencode-client").unwrap(), OPENCODE_CLIENT);
    assert_eq!(out.get("x-opencode-project").unwrap(), OPENCODE_PROJECT);
    assert_eq!(out.get("x-opencode-request").unwrap(), "msg_fixed");
    assert_eq!(out.get("x-opencode-session").unwrap(), "ses_fixed123");
    assert_eq!(out.get(header::USER_AGENT).unwrap(), ZEN_USER_AGENT);

    let mut off = test_proxy_state(
        "ses_fixed123",
        "https://opencode.ai/zen/v1",
        None,
        TraceSink::disabled(),
    );
    off.opencode_identity = false;
    let mut out = HeaderMap::new();
    apply_opencode_identity(&mut out, &inbound, &off, "ses_fixed123", "msg_fixed");
    for name in OPENCODE_IDENTITY_HEADERS {
        assert!(out.get(name).is_none(), "关闭时不应发 {name}");
    }
    assert_eq!(
        out.get(header::USER_AGENT).unwrap(),
        "codex/1.2.3",
        "关闭时 UA 应透传入站值"
    );

    // 入站没有 UA 时关闭态不带 UA（客户端本身也不再设默认 UA）
    let mut out = HeaderMap::new();
    apply_opencode_identity(
        &mut out,
        &HeaderMap::new(),
        &off,
        "ses_fixed123",
        "msg_fixed",
    );
    assert!(out.get(header::USER_AGENT).is_none());
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
    // 6 个工具全部追加 → 教学并入 messages：原本没有 system 消息，新建一条放在最前
    assert_eq!(body["messages"][0]["role"], "system");
    let inst = body["messages"][0]["content"].as_str().unwrap().to_string();
    assert!(
        inst.contains("bash、edit、glob、grep、read、write"),
        "教学应列出全部 6 个工具：{inst}"
    );
    assert!(
        body.get("instructions").is_none(),
        "chat 体不应出现顶层 instructions：{body}"
    );
    // 幂等：再调用一次不叠加
    patch_zen_request_body(&mut body);
    assert_eq!(
        body["tools"].as_array().unwrap().len(),
        ZEN_REQUIRED_TOOL_NAMES.len()
    );
    // 幂等：不新增消息、教学不重复追加
    assert_eq!(
        body["messages"].as_array().unwrap().len(),
        1,
        "幂等调用不应新增消息：{body}"
    );
    assert_eq!(body["messages"][0]["content"].as_str().unwrap(), &inst);

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
        vec![
            "shell",
            "codexui_glob",
            "bash",
            "edit",
            "glob",
            "grep",
            "read",
            "write"
        ],
        "真实工具在前、缺失的假工具按固定顺序追加在后"
    );
    assert_eq!(tools[2]["function"]["description"], "客户端自己的 bash");
    // bash 已存在不追加 → 教学只列 edit/glob/grep/read/write
    let inst = body["messages"][0]["content"].as_str().unwrap();
    assert!(
        inst.contains("edit、glob、grep、read、write"),
        "教学应列出 5 个缺失工具：{inst}"
    );
    assert!(
        !inst.contains("bash、edit"),
        "教学不应重复列出 bash：{inst}"
    );
}

#[test]
fn fake_tools_instruction_returns_none_for_empty_list() {
    assert!(fake_tools_instruction(&[]).is_none());
}

#[test]
fn fake_tools_instruction_lists_only_added_tools() {
    let text = fake_tools_instruction(&["edit", "glob"]).unwrap();
    assert!(text.contains("edit、glob"), "应列出指定工具：{text}");
    assert!(!text.contains("bash"), "不应含未追加工具：{text}");
    assert!(!text.contains("grep"), "不应含未追加工具：{text}");
}

#[test]
fn patch_zen_request_body_appends_teaching_to_existing_system_message() {
    // 真实形状：入站 instructions 已被 responses_to_chat 翻成首条 system 消息
    let mut body = json!({
        "model": "m",
        "messages": [{ "role": "system", "content": "原有教学内容" }],
        "stream": true
    });
    patch_zen_request_body(&mut body);
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        1,
        "应并入已有 system 消息、不新增消息：{body}"
    );
    let inst = messages[0]["content"].as_str().unwrap();
    assert!(
        inst.starts_with("原有教学内容"),
        "应保留原有 system 文本前缀：{inst}"
    );
    assert!(inst.contains("弃用工具声明"), "应追加弃用教学：{inst}");
    assert!(
        inst.contains("原有教学内容\n\n"),
        "原文本与教学之间应有一个空行分隔：{inst}"
    );
    assert!(
        body.get("instructions").is_none(),
        "chat 体不应出现顶层 instructions：{body}"
    );
}

#[test]
fn patch_zen_request_body_idempotent_instructions() {
    let mut body = json!({ "model": "m", "messages": [], "stream": true });
    patch_zen_request_body(&mut body);
    let first = body["messages"][0]["content"].as_str().unwrap().to_string();
    patch_zen_request_body(&mut body);
    assert_eq!(
        body["messages"].as_array().unwrap().len(),
        1,
        "幂等调用不应新增消息：{body}"
    );
    let second = body["messages"][0]["content"].as_str().unwrap();
    assert_eq!(first, second, "幂等调用不应重复追加教学");
}

#[test]
fn patch_zen_request_body_inserts_system_message_before_user_message() {
    // 没有 system 消息（如非流式、无 instructions 的请求）：教学另起一条放在最前
    let mut body = json!({
        "model": "m",
        "messages": [{ "role": "user", "content": "hi" }],
        "stream": true
    });
    patch_zen_request_body(&mut body);
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        2,
        "应插一条 system 消息、不动原有消息：{body}"
    );
    assert_eq!(messages[0]["role"], "system");
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains("弃用工具声明"));
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "hi");
}

#[test]
fn patch_zen_request_body_inserts_when_leading_system_content_is_not_text() {
    // 首条 system 的 content 不是字符串（分片形态）：不合并，另插一条 system 消息
    let mut body = json!({
        "model": "m",
        "messages": [{ "role": "system", "content": [{ "type": "text", "text": "分段" }] }],
    });
    patch_zen_request_body(&mut body);
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        2,
        "分片 system 消息无法追加，应另插一条：{body}"
    );
    assert_eq!(messages[0]["role"], "system");
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains("弃用工具声明"));
    assert!(messages[1]["content"].is_array(), "原有分片消息应逐字保留");
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
    let c1 =
        json!({ "choices": [{ "delta": { "reasoning_content": "先看" }, "finish_reason": null }] });
    let c2 =
        json!({ "choices": [{ "delta": { "reasoning_content": "日志" }, "finish_reason": null }] });
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
            serde_json::from_str::<Value>(line.lines().nth(1)?.trim_start_matches("data:").trim())
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
    assert!(lines.iter().any(
        |line| line.contains("response.reasoning_summary_text.delta") && line.contains("想想")
    ));
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
    let resp = chat_to_responses(&chat, "m", &ToolShape::default(), true).unwrap();
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
