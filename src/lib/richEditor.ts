import { mergeAttributes, Node, type JSONContent } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import type { UserInput } from "./types";

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

/**
 * 把编辑器 doc JSON 序列化为有序 runs：文本按块以 \n 连接，引用按文档顺序保留。
 *
 * 递归遍历整棵 doc 树，而不仅是顶层块的直接子节点——否则粘贴/输入的列表、块引用、
 * 代码块等嵌套结构（doc > bulletList > listItem > paragraph > text）会取不到里层文本，
 * 导致输入框可见文本但 runsToText 为空、发送按钮被误禁用。
 */
export function docToRuns(doc: JSONContent): EditorRun[] {
  const runs: EditorRun[] = [];
  const pushText = (s: string) => {
    if (!s) return;
    const last = runs[runs.length - 1];
    if (last && last.kind === "text") last.text += s;
    else runs.push({ kind: "text", text: s });
  };

  // 块级容器：其子块之间以 \n 分隔（doc/有序或无序列表/列表项/块引用/代码块）。
  // 行内文本块（paragraph/heading 等）的子节点间不插入换行，保持现有单段内文本顺序。
  const isBlockContainer = (t: string) =>
    ["doc", "bulletList", "orderedList", "listItem", "blockquote", "codeBlock"].includes(t);

  const walk = (node: JSONContent) => {
    const type = node.type ?? "";
    if (type === "text") {
      if (typeof node.text === "string" && node.text) pushText(node.text);
      return;
    }
    if (type === "reference") {
      if (node.attrs) {
        const kind = node.attrs.kind === "skill" ? "skill" : "plugin";
        runs.push({
          kind: "ref",
          refId: String(node.attrs.refId ?? ""),
          refKind: node.attrs.kind === "file" ? "file" : kind,
          label: String(node.attrs.label ?? ""),
        });
      }
      return;
    }
    const content = Array.isArray(node.content) ? node.content : [];
    content.forEach((child, i) => {
      walk(child);
      if (isBlockContainer(type) && i < content.length - 1) pushText("\n");
    });
  };

  walk(doc);
  return runs;
}

/** 从 runs 中提取纯文本（引用 chip 不贡献文本） */
export function runsToText(runs: EditorRun[]): string {
  return runs
    .filter((r) => r.kind === "text")
    .map((r) => (r.kind === "text" ? r.text : ""))
    .join("");
}

/**
 * 把编辑器 runs 序列化为内联文本：插件 `[@名称](plugin://pluginId)` 与技能 `[$名称](路径)`
 * 内联在文本原位；文件不再内联（由调用方经附件区进入 Files 段）。
 */
export function runsToWireText(
  runs: EditorRun[],
  refsById: ReadonlyMap<string, UserInput>,
): string {
  let inline = "";
  for (const r of runs) {
    if (r.kind === "text") {
      inline += r.text;
      continue;
    }
    const a = refsById.get(r.refId);
    if (!a) continue;
    if (a.type !== "mention" && a.type !== "skill") continue;
    const name = a.name;
    if (r.refKind === "file") continue; // 文件不进内联文本
    if (r.refKind === "skill") {
      inline += `[$${name}](${a.path}) `;
    } else if (r.refKind === "plugin" && a.type === "skill" && a.pluginId) {
      inline += `[@${name}](plugin://${a.pluginId}) `;
    } else if (r.refKind === "plugin") {
      inline += `[@${name}](${a.path}) `;
    }
  }
  return inline;
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
