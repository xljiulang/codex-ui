/** 交互/审批协议字段访问辅助（协议数据为透明 JSON，尽力类型化取值） */

export function str(v: unknown): string {
  return v == null ? "" : String(v);
}

export function list(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}

export function obj(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" ? (v as Record<string, unknown>) : {};
}

/** 把权限对象/数组转成易读文本（对象渲染为 `键：值`，数组用“；”连接） */
export function permText(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (Array.isArray(v)) {
    return v
      .map((x) => permText(x))
      .filter(Boolean)
      .join("；");
  }
  if (typeof v === "object") {
    return Object.entries(v as Record<string, unknown>)
      .map(([k, val]) => `${k}：${permText(val)}`)
      .join("；");
  }
  return String(v);
}
