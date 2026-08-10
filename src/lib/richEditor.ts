import { mergeAttributes, Node, type JSONContent } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import type { UserInput } from "./types";
import { toProtocolPath } from "./mention";

/** 编辑器内的有序内容单元：文本 run 或引用 chip run */
export type EditorRun =
  | { kind: "text"; text: string }
  | {
      kind: "ref";
      refId: string;
      refKind: "file" | "plugin" | "skill";
      label: string;
    };

/**
 * 内联引用 chip 节点（contenteditable=false 的 span）。
 * 不参与文本序列化（leafText 为空），光标可跨越，Backspace 由 ProseMirror 默认处理删除。
 */
export const Reference = Node.create({
  name: "reference",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  draggable: false,

  addAttributes() {
    return {
      refId: { default: null },
      kind: { default: "file" },
      label: { default: "" },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-ref-id]" }];
  },

  renderHTML({ node, HTMLAttributes }) {
    const kind = String(node.attrs.kind ?? "file");
    const prefix = kind === "skill" ? "$" : "@";
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        class: `ref-chip ref-chip--${kind}`,
        "data-ref-id": String(node.attrs.refId ?? ""),
        "data-kind": kind,
        "data-label": String(node.attrs.label ?? ""),
      }),
      `${prefix}${node.attrs.label ?? ""}`,
    ];
  },
});

/** 把编辑器 doc JSON 序列化为有序 runs：文本按块以 \n 连接，引用按文档顺序保留 */
export function docToRuns(doc: JSONContent): EditorRun[] {
  const runs: EditorRun[] = [];
  const blocks = Array.isArray(doc.content) ? doc.content : [];
  const pushText = (s: string) => {
    if (!s) return;
    const last = runs[runs.length - 1];
    if (last && last.kind === "text") last.text += s;
    else runs.push({ kind: "text", text: s });
  };

  blocks.forEach((block, bi) => {
    const items = Array.isArray(block.content) ? block.content : [];
    for (const item of items) {
      if (item.type === "text" && typeof item.text === "string" && item.text) {
        pushText(item.text);
      } else if (item.type === "reference" && item.attrs) {
        const kind = item.attrs.kind === "skill" ? "skill" : "plugin";
        runs.push({
          kind: "ref",
          refId: String(item.attrs.refId ?? ""),
          refKind: item.attrs.kind === "file" ? "file" : kind,
          label: String(item.attrs.label ?? ""),
        });
      }
    }
    if (bi < blocks.length - 1) pushText("\n");
  });
  return runs;
}

/** 从 runs 中提取纯文本（引用 chip 不贡献文本） */
export function runsToText(runs: EditorRun[]): string {
  return runs
    .filter((r) => r.kind === "text")
    .map((r) => (r.kind === "text" ? r.text : ""))
    .join("");
}

/** 计算绝对路径相对工作目录的路径（正斜杠，Windows 前缀比较不区分大小写）；不在根下时回退绝对路径 */
export function relativePath(root: string, abs: string): string {
  if (!root) return toProtocolPath(abs);
  const norm = (p: string) =>
    toProtocolPath(p)
      .replace(/^\/+/, "")
      .replace(/\/+$/, "")
      .split("/");
  const rootParts = norm(root);
  const absParts = norm(abs);
  let i = 0;
  while (
    i < rootParts.length &&
    i < absParts.length &&
    rootParts[i].toLowerCase() === absParts[i].toLowerCase()
  ) {
    i++;
  }
  if (i === 0) return toProtocolPath(abs);
  const rel = [
    ...Array(rootParts.length - i).fill(".."),
    ...absParts.slice(i),
  ];
  return rel.length ? rel.join("/") : toProtocolPath(abs);
}

/**
 * 把编辑器 runs 序列化为单条发送文本：引用以内联链接写在原位置
 * 文件 `[名称](相对路径)`、插件 `[@名称](plugin://pluginId)`、技能 `[$名称](路径)`。
 */
export function runsToWireText(
  runs: EditorRun[],
  refsById: ReadonlyMap<string, UserInput>,
  root = "",
): string {
  let out = "";
  for (const r of runs) {
    if (r.kind === "text") {
      out += r.text;
      continue;
    }
    const a = refsById.get(r.refId);
    if (!a) continue;
    if (a.type !== "mention" && a.type !== "skill") continue;
    const name = a.name;
    if (r.refKind === "skill") {
      out += `[$${name}](${toProtocolPath(a.path)}) `;
    } else if (r.refKind === "plugin" && a.type === "skill" && a.pluginId) {
      out += `[@${name}](plugin://${a.pluginId}) `;
    } else if (r.refKind === "plugin") {
      out += `[@${name}](${toProtocolPath(a.path)}) `;
    } else {
      out += `[${name}](${relativePath(root, a.path)}) `;
    }
  }
  return out;
}

/**
 * 把附件映射为编辑器 chip 种类：mention→file；localImage 不进编辑器（由调用方处理）；
 * skill 按 source 区分 plugin/skill。
 */
export function refKindOfAttachment(a: UserInput): "file" | "plugin" | "skill" {
  if (a.type === "skill") return a.source === "plugin" ? "plugin" : "skill";
  return "file";
}

/**
 * 计算「@/$ + token」在文档中的起始位置，供删除触发词使用。
 * 向后遍历光标所在文本块：引用 chip 不贡献文本字符，文本节点按长度扣除。
 */
export function tokenStartPos(
  doc: PMNode,
  caretPos: number,
  tokenLen: number,
): number {
  const clamped = Math.max(0, Math.min(caretPos, doc.content.size));
  const $from = doc.resolve(clamped);
  const parent = $from.parent;
  let need = tokenLen + 1; // 触发符 + token
  if (parent.isTextblock) {
    const start = $from.start();
    let childEnd = $from.parentOffset;
    for (let i = parent.childCount - 1; i >= 0 && need > 0; i--) {
      const child = parent.child(i);
      const childStart = childEnd - child.nodeSize;
      if (child.isText) {
        const avail = Math.min(need, childEnd - Math.max(childStart, 0));
        if (avail >= need) return start + childEnd - need;
        need -= avail;
      }
      childEnd = childStart;
    }
  }
  // 兜底：假设触发词紧跟光标
  return Math.max(0, caretPos - (tokenLen + 1));
}
