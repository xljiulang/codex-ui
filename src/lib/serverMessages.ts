/**
 * 服务端（codex app-server）错误/警告消息 → 友好中文提示。
 * 已知消息映射为中文；未匹配的消息保留原文（便于排查服务端问题）。
 */

/** codexErrorInfo 变体 → 中文（结构化匹配，优先于消息文本匹配） */
const CODEX_ERROR_INFO_MAP: Record<string, string> = {
  contextWindowExceeded: "上下文已超出模型窗口",
  sessionBudgetExceeded: "会话预算已耗尽",
  usageLimitExceeded: "已超出用量限制",
  serverOverloaded: "服务过载，请稍后重试",
  internalServerError: "服务内部错误",
  badRequest: "请求参数有误",
  unauthorized: "认证失效，请重新登录",
  sandboxError: "沙箱执行出错",
  httpConnectionFailed: "网络连接失败，请重试",
  responseStreamConnectionFailed: "响应连接失败，请重试",
  responseStreamDisconnected: "响应连接已断开，请重试",
  responseTooManyFailedAttempts: "响应重试次数过多，请稍后重试",
  activeTurnNotSteerable: "当前回合类型不支持该操作",
  threadRollbackFailed: "会话回滚失败",
  cyberPolicy: "内容策略拦截，无法执行",
};

interface MessageRule {
  /** 小写子串匹配；列表按匹配长度从长到短排列，避免短模式误吞 */
  match: string;
  text: string;
}

const MESSAGE_RULES: MessageRule[] = [
  {
    match: "context window exceeded while compacting",
    text: "上下文超出窗口，已自动压缩并移除最早的历史内容",
  },
  {
    match: "cannot resume running thread",
    text: "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试",
  },
  {
    match: "thread is unavailable",
    text: "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试",
  },
  {
    match: "thread is busy",
    text: "该会话正被占用（已有回合在运行或其它进程持有），请稍后再试",
  },
  {
    match: "preflight is already running",
    text: "已有审批/应用操作进行中，请先等待完成",
  },
  {
    match: "apply is already running",
    text: "已有审批/应用操作进行中，请先等待完成",
  },
  { match: "server overloaded", text: "服务过载，请稍后重试" },
  { match: "requires experimentalapi capability", text: "当前 Codex 版本不支持该功能" },
  { match: "not initialized", text: "服务尚未就绪，请稍后重试" },
  { match: "already initialized", text: "服务重复初始化" },
  { match: "thread not found", text: "会话不存在" },
  { match: "context window exceeded", text: "上下文已超出模型窗口" },
  { match: "session budget exceeded", text: "会话预算已耗尽" },
  { match: "quota exceeded", text: "会话预算已耗尽" },
  { match: "usage limit exceeded", text: "已超出用量限制" },
  { match: "usage not included", text: "已超出用量限制" },
  { match: "unauthorized", text: "认证失效，请重新登录" },
  { match: "authentication failed", text: "认证失效，请重新登录" },
  { match: "sandbox", text: "沙箱执行出错" },
  { match: "response stream disconnected", text: "响应连接已断开，请重试" },
  { match: "response stream connection failed", text: "响应连接失败，请重试" },
  { match: "response too many failed attempts", text: "响应重试次数过多，请稍后重试" },
  { match: "http connection failed", text: "网络连接失败，请重试" },
  { match: "active turn not steerable", text: "当前回合类型不支持该操作" },
  { match: "rollback", text: "会话回滚失败" },
  { match: "cyber policy", text: "内容策略拦截，无法执行" },
  { match: "content policy", text: "内容策略拦截，无法执行" },
  { match: "bad request", text: "请求参数有误" },
  { match: "invalid params", text: "请求参数有误" },
  { match: "invalid parameters", text: "请求参数有误" },
  { match: "method not found", text: "当前 Codex 版本不支持该操作" },
  { match: "unknown variant", text: "当前 Codex 版本不支持该操作" },
  { match: "timed out", text: "请求超时，请重试" },
  { match: "timeout", text: "请求超时，请重试" },
  { match: "internal server error", text: "服务内部错误" },
  { match: "internal error", text: "服务内部错误" },
];

/** 提取错误消息：优先 error.error.message / error.message，回退 String(e) */
export function extractErrorMessage(e: unknown): string {
  if (e && typeof e === "object") {
    const err = e as { error?: { message?: unknown }; message?: unknown };
    if (typeof err.error?.message === "string") return err.error.message;
    if (typeof err.message === "string") return err.message;
  }
  return String(e);
}

/** codexErrorInfo：字符串（"contextWindowExceeded"）或对象（{ activeTurnNotSteerable: {...} }）→ 中文；未命中返回 null */
function mapCodexErrorInfo(info: unknown): string | null {
  if (typeof info === "string") return CODEX_ERROR_INFO_MAP[info] ?? null;
  if (info && typeof info === "object") {
    const keys = Object.keys(info as Record<string, unknown>);
    if (keys.length === 1) return CODEX_ERROR_INFO_MAP[keys[0]] ?? null;
  }
  return null;
}

/** 服务端消息 → 友好中文；未匹配保留原文 */
export function friendlyServerMessage(raw: string): string {
  const lower = raw.toLowerCase();
  for (const rule of MESSAGE_RULES) {
    if (lower.includes(rule.match)) return rule.text;
  }
  return raw;
}

/**
 * 是否为「模型 metadata 未找到，回退到 fallback metadata」的警告。
 * 该提示只在 warning 通道以 message 字符串下发，无结构化 codexErrorInfo，
 * 因此按文本匹配；同时命中三个子串才判真，避免误吞其它 metadata/fallback 类消息。
 */
export function isIgnoredWarning(raw: string): boolean {
  const lower = raw.toLowerCase();
  return (
    lower.includes("model metadata for") &&
    lower.includes("not found") &&
    lower.includes("defaulting to fallback metadata")
  );
}

/** 错误对象/值 → 友好中文：优先结构化 codexErrorInfo，其次消息文本匹配，未匹配保留原文 */
export function friendlyServerError(e: unknown): string {
  if (e && typeof e === "object") {
    const err = e as {
      error?: { codexErrorInfo?: unknown };
      codexErrorInfo?: unknown;
    };
    const mapped = mapCodexErrorInfo(
      err.error?.codexErrorInfo ?? err.codexErrorInfo,
    );
    if (mapped) return mapped;
  }
  return friendlyServerMessage(extractErrorMessage(e));
}
