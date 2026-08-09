export type DiffLineKind = "ctx" | "add" | "del";

export interface DiffHunk {
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  lines: { kind: DiffLineKind; text: string }[];
}

export type InlineRow =
  | { kind: "ctx"; oldNo: number; newNo: number; text: string }
  | { kind: "del"; oldNo: number; text: string }
  | { kind: "add"; newNo: number; text: string }
  | { kind: "sep" };

/** 按行拆分文本（去掉末尾空元素，兼容 \r\n） */
export function splitLines(content: string): string[] {
  if (!content) return [];
  const lines = content.split("\n").map((l) => l.replace(/\r$/, ""));
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/**
 * 解析 unified diff 的 hunks；忽略 diff 头（diff --git / --- / +++）与
 * “\ No newline at end of file” 标记。
 */
export function parseUnifiedDiff(diff: string): DiffHunk[] {
  const hunks: DiffHunk[] = [];
  let current: DiffHunk | null = null;
  for (const raw of diff.split("\n")) {
    const line = raw.replace(/\r$/, "");
    if (line.startsWith("\\")) continue;
    // 跳过空行：unified diff 尾部常带一个换行产生的空元素，不应计入 hunk 内容
    if (line === "") continue;
    const m =
      /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (m) {
      current = {
        oldStart: Number(m[1]),
        oldCount: Number(m[2] ?? 1),
        newStart: Number(m[3]),
        newCount: Number(m[4] ?? 1),
        lines: [],
      };
      hunks.push(current);
      continue;
    }
    if (!current) continue;
    if (line.startsWith("+")) {
      current.lines.push({ kind: "add", text: line.slice(1) });
    } else if (line.startsWith("-")) {
      current.lines.push({ kind: "del", text: line.slice(1) });
    } else {
      current.lines.push({
        kind: "ctx",
        text: line.startsWith(" ") ? line.slice(1) : line,
      });
    }
  }
  return hunks;
}

/** hunk 内的旧区行（ctx+del，保持原顺序）与新区行数（ctx+add） */
function hunkRegions(h: DiffHunk): { oldRegion: string[]; newRegionLen: number } {
  const oldRegion: string[] = [];
  let newRegionLen = 0;
  for (const l of h.lines) {
    if (l.kind === "del") {
      oldRegion.push(l.text);
    } else {
      newRegionLen++; // ctx 与 add 都属于新文件区域
      if (l.kind === "ctx") oldRegion.push(l.text);
    }
  }
  return { oldRegion, newRegionLen };
}

/** 校验 hunk 与内容是否一致，不一致抛错（调用方回退统一 diff） */
function validateHunks(
  hunks: DiffHunk[],
  oldLines: string[] | null,
  newLines: string[],
) {
  for (const h of hunks) {
    const ctx = h.lines.filter((l) => l.kind === "ctx").length;
    const del = h.lines.filter((l) => l.kind === "del").length;
    const add = h.lines.filter((l) => l.kind === "add").length;
    if (ctx + del !== h.oldCount || ctx + add !== h.newCount) {
      throw new Error("diff hunk 计数不一致");
    }
    if (h.newStart - 1 > newLines.length) {
      throw new Error("diff hunk 超出新内容范围");
    }
    if (oldLines !== null && h.oldStart - 1 > oldLines.length) {
      throw new Error("diff hunk 超出旧内容范围");
    }
  }
}

/**
 * 反向应用 unified diff：由新内容重建旧内容。
 * 自底向上逐 hunk 用旧区行替换新区行。
 */
export function applyReverseUnifiedDiff(
  newContent: string,
  diff: string,
): string {
  const hunks = parseUnifiedDiff(diff);
  const newLines = splitLines(newContent);
  validateHunks(hunks, null, newLines);
  for (let i = hunks.length - 1; i >= 0; i--) {
    const h = hunks[i];
    const { oldRegion, newRegionLen } = hunkRegions(h);
    const start = Math.max(0, h.newStart - 1);
    const delCount = Math.max(0, Math.min(newRegionLen, newLines.length - start));
    newLines.splice(start, delCount, ...oldRegion);
  }
  return newLines.join("\n");
}

/**
 * 构建单栏内联行：hunk 外未变行逐行配对（覆盖全部内容）；
 * hunk 内 ctx 行保留原位，旧行（红）在上、新行（绿）在下，中间插入“旧 | 新”分隔。
 */
export function buildInlineRows(
  oldContent: string,
  newContent: string,
  diff: string,
): InlineRow[] {
  const hunks = parseUnifiedDiff(diff);
  const oldLines = splitLines(oldContent);
  const newLines = splitLines(newContent);
  validateHunks(hunks, oldLines, newLines);
  const rows: InlineRow[] = [];
  let oldIdx = 0;
  let newIdx = 0;

  const emitCtxPair = () => {
    const o = oldLines[oldIdx];
    const n = newLines[newIdx];
    rows.push({
      kind: "ctx",
      oldNo: oldIdx + 1,
      newNo: newIdx + 1,
      text: o ?? n ?? "",
    });
    oldIdx++;
    newIdx++;
  };

  for (const h of hunks) {
    const pre = Math.max(
      Math.max(0, h.oldStart - 1 - oldIdx),
      Math.max(0, h.newStart - 1 - newIdx),
    );
    for (let i = 0; i < pre; i++) emitCtxPair();

    let o = h.oldStart;
    let n = h.newStart;
    let sawChange = false;
    const beforeCtx: InlineRow[] = [];
    const afterCtx: InlineRow[] = [];
    const dels: { oldNo: number; text: string }[] = [];
    const adds: { newNo: number; text: string }[] = [];
    for (const l of h.lines) {
      if (l.kind === "ctx") {
        const row: InlineRow = {
          kind: "ctx",
          oldNo: o++,
          newNo: n++,
          text: l.text,
        };
        (sawChange ? afterCtx : beforeCtx).push(row);
      } else if (l.kind === "del") {
        dels.push({ oldNo: o++, text: l.text });
        sawChange = true;
      } else {
        adds.push({ newNo: n++, text: l.text });
        sawChange = true;
      }
    }
    rows.push(...beforeCtx);
    rows.push(
      ...dels.map(
        (d) => ({ kind: "del" as const, oldNo: d.oldNo, text: d.text }),
      ),
    );
    if (dels.length && adds.length) rows.push({ kind: "sep" });
    rows.push(
      ...adds.map(
        (a) => ({ kind: "add" as const, newNo: a.newNo, text: a.text }),
      ),
    );
    rows.push(...afterCtx);

    oldIdx += h.oldCount;
    newIdx += h.newCount;
  }
  while (oldIdx < oldLines.length || newIdx < newLines.length) {
    emitCtxPair();
  }
  return rows;
}
