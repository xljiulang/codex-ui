import type { GitFile } from "./gitChanges";

/** 变更文件树节点：文件行 */
export interface GitFileNode {
  kind: "file";
  name: string;
  relPath: string;
  depth: number;
  file: GitFile;
}

/** 变更文件树节点：目录行（聚合子级的暂存/未暂存/未跟踪标记） */
export interface GitDirNode {
  kind: "dir";
  name: string;
  relPath: string;
  depth: number;
  collapsed: boolean;
  childCount: number;
  hasStaged: boolean;
  hasUnstaged: boolean;
  hasUntracked: boolean;
  children: GitTreeNode[];
}

export type GitTreeNode = GitFileNode | GitDirNode;

interface DirAcc {
  dirs: Map<string, DirAcc>;
  files: GitFile[];
}

/**
 * 把变更文件列表构建为目录树（目录按名称 A-Z、文件按路径 A-Z），
 * 目录的折叠态由调用方传入的 Set 决定（跨刷新保留）。
 */
export function buildGitTree(
  files: GitFile[],
  collapsedDirs: ReadonlySet<string>,
): GitTreeNode[] {
  const root: DirAcc = { dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split("/");
    let acc = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const d = parts[i];
      let next = acc.dirs.get(d);
      if (!next) {
        next = { dirs: new Map(), files: [] };
        acc.dirs.set(d, next);
      }
      acc = next;
    }
    acc.files.push(f);
  }
  return dirNodes(root, "", 0, collapsedDirs);
}

function dirNodes(
  acc: DirAcc,
  relPath: string,
  depth: number,
  collapsedDirs: ReadonlySet<string>,
): GitTreeNode[] {
  const nodes: GitTreeNode[] = [];
  for (const name of [...acc.dirs.keys()].sort((a, b) => a.localeCompare(b))) {
    const childRel = relPath ? `${relPath}/${name}` : name;
    const children = dirNodes(acc.dirs.get(name)!, childRel, depth + 1, collapsedDirs);
    const childCount = children.reduce(
      (n, c) => n + (c.kind === "dir" ? c.childCount : 1),
      0,
    );
    const hasStaged = children.some((c) =>
      c.kind === "dir" ? c.hasStaged : c.file.staged,
    );
    const hasUnstaged = children.some((c) =>
      c.kind === "dir" ? c.hasUnstaged : !c.file.staged,
    );
    const hasUntracked = children.some((c) =>
      c.kind === "dir" ? c.hasUntracked : c.file.status === "untracked",
    );
    nodes.push({
      kind: "dir",
      name,
      relPath: childRel,
      depth,
      collapsed: collapsedDirs.has(childRel),
      childCount,
      hasStaged,
      hasUnstaged,
      hasUntracked,
      children,
    });
  }
  const files = acc.files
    .slice()
    .sort((a, b) => a.path.localeCompare(b.path));
  for (const f of files) {
    nodes.push({
      kind: "file",
      name: f.path.split("/").pop() ?? f.path,
      relPath: f.path,
      depth,
      file: f,
    });
  }
  return nodes;
}

/** 树 → 扁平可见行（折叠目录不展开子级） */
export function flattenRows(nodes: GitTreeNode[]): GitTreeNode[] {
  const rows: GitTreeNode[] = [];
  for (const n of nodes) {
    rows.push(n);
    if (n.kind === "dir" && !n.collapsed) {
      rows.push(...flattenRows(n.children));
    }
  }
  return rows;
}
