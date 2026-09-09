import { describe, expect, it } from "vitest";
import {
  extractErrorMessage,
  friendlyServerError,
  friendlyServerMessage,
} from "../serverMessages";

describe("friendlyServerMessage 服务端消息本地化", () => {
  const cases: [string, string][] = [
    [
      "cannot resume running thread thr_123 with history while it is already running",
      "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试",
    ],
    ["Thread is unavailable", "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试"],
    ["thread is busy", "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试"],
    [
      "Context window exceeded while compacting; removing oldest history item. Error: x",
      "上下文超出窗口，已自动压缩并移除最早的历史内容",
    ],
    ["Context window exceeded", "上下文已超出模型窗口"],
    ["session budget exceeded", "会话预算已耗尽"],
    ["quota exceeded", "会话预算已耗尽"],
    ["usage limit exceeded", "已超出用量限制"],
    ["usage not included", "已超出用量限制"],
    ["Server overloaded; retry later.", "服务过载，请稍后重试"],
    [
      "thread/start.mockField requires experimentalApi capability",
      "当前 Codex 版本不支持该功能",
    ],
    ["Not initialized", "服务尚未就绪，请稍后重试"],
    ["Already initialized", "服务重复初始化"],
    ["thread not found", "会话不存在"],
    [
      "A preflight is already running; wait for it to finish first.",
      "已有审批/应用操作进行中，请先等待完成",
    ],
    [
      "An apply is already running; wait for it to finish first.",
      "已有审批/应用操作进行中，请先等待完成",
    ],
    ["unauthorized: invalid token", "认证失效，请重新登录"],
    ["sandbox error while executing command", "沙箱执行出错"],
    ["response stream disconnected", "响应连接已断开，请重试"],
    ["response stream connection failed", "响应连接失败，请重试"],
    ["response too many failed attempts", "响应重试次数过多，请稍后重试"],
    ["http connection failed with status 500", "网络连接失败，请重试"],
    ["active turn not steerable", "当前回合类型不支持该操作"],
    ["thread rollback failed", "会话回滚失败"],
    ["cyber policy blocked", "内容策略拦截，无法执行"],
    ["bad request", "请求参数有误"],
    ["Invalid params", "请求参数有误"],
    ["method not found: foo", "当前 Codex 版本不支持该操作"],
    ["unknown variant `X`, expected ...", "当前 Codex 版本不支持该操作"],
    ["request timed out after 60s", "请求超时，请重试"],
    ["internal error", "服务内部错误"],
  ];

  it.each(cases)("映射 %s → %s", (raw, expected) => {
    expect(friendlyServerMessage(raw)).toBe(expected);
  });

  it("长模式优先：窗口压缩完整消息不被短模式截胡", () => {
    const raw =
      "Context window exceeded while compacting; removing oldest history item.";
    expect(friendlyServerMessage(raw)).toBe(
      "上下文超出窗口，已自动压缩并移除最早的历史内容",
    );
  });

  it("未匹配消息保留原文", () => {
    const raw = "Some brand new server message from a future version";
    expect(friendlyServerMessage(raw)).toBe(raw);
  });
});

describe("friendlyServerError 结构化 codexErrorInfo", () => {
  it("字符串形态优先于消息文本", () => {
    expect(
      friendlyServerError({
        message: "Context window exceeded",
        codexErrorInfo: "contextWindowExceeded",
      }),
    ).toBe("上下文已超出模型窗口");
  });

  it("对象形态取首键（带字段的变体）", () => {
    expect(
      friendlyServerError({
        message: "raw",
        codexErrorInfo: { activeTurnNotSteerable: { turnKind: "review" } },
      }),
    ).toBe("当前回合类型不支持该操作");
  });

  it("嵌套 error.codexErrorInfo 同样生效", () => {
    expect(
      friendlyServerError({
        error: { message: "raw", codexErrorInfo: "serverOverloaded" },
      }),
    ).toBe("服务过载，请稍后重试");
  });

  it("未识别 codexErrorInfo 回退消息文本映射", () => {
    expect(
      friendlyServerError({
        message: "Server overloaded; retry later.",
        codexErrorInfo: "other",
      }),
    ).toBe("服务过载，请稍后重试");
  });
});

describe("extractErrorMessage 提取优先级", () => {
  it("error.error.message > error.message > String(e)", () => {
    expect(extractErrorMessage({ error: { message: "A" }, message: "B" })).toBe(
      "A",
    );
    expect(extractErrorMessage({ message: "B" })).toBe("B");
    expect(extractErrorMessage("raw")).toBe("raw");
    expect(extractErrorMessage(42)).toBe("42");
    expect(extractErrorMessage(null)).toBe("null");
  });
});
