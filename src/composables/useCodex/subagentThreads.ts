import type { ThreadRead } from "../../lib/types";

type ThreadIdentity = Pick<ThreadRead["thread"], "id"> &
  Partial<
    Pick<
      ThreadRead["thread"],
      "source" | "parentThreadId" | "ephemeral" | "agentNickname" | "agentRole"
    >
  >;

const MAX_SUBAGENT_THREADS = 1024;
const parentByChild = new Map<string, string>();

/** 登记子代理线程并返回父线程 id；普通线程返回 null。 */
export function registerSubagentThread(thread: ThreadIdentity): string | null {
  const child = thread.id?.trim();
  const incomingParent = thread.parentThreadId?.trim() ?? "";
  const existingParent = child ? parentByChild.get(child) : undefined;
  const isSubagentSource = thread.source?.trim().toLowerCase() === "subagent";
  if (
    !child ||
    (!isSubagentSource && !incomingParent && existingParent === undefined)
  ) {
    return null;
  }
  const parent = incomingParent || existingParent || "";
  if (
    existingParent === undefined &&
    parentByChild.size >= MAX_SUBAGENT_THREADS
  ) {
    parentByChild.clear();
  }
  parentByChild.set(child, parent);
  return parent;
}

export function isSubagentThread(threadId: string | null | undefined): boolean {
  return !!threadId && parentByChild.has(threadId);
}

export function subagentParentThread(
  threadId: string | null | undefined,
): string | null {
  if (!threadId) return null;
  let parent = parentByChild.get(threadId) || "";
  const seen = new Set<string>();
  while (parent && parentByChild.has(parent) && !seen.has(parent)) {
    seen.add(parent);
    parent = parentByChild.get(parent) || "";
  }
  return parent || null;
}

/** 仅测试用：清空跨用例共享的登记表。 */
export function __resetSubagentThreadsForTest() {
  parentByChild.clear();
}
