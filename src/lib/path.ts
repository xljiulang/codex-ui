/**
 * Windows 路径统一处理模块（唯一实现，禁止在各处散落 replace/compare）：
 * - 外部传入的路径可以是正斜杠或反斜杠、任意大小写，本模块一律接受；
 * - 存储/展示形态：反斜杠 + 去尾分隔符（盘符根保留尾分隔符）+ 保留大小写（normalizeFsPath）；
 * - 比较/键形态：反斜杠 + 去尾分隔符（含盘符根）+ 小写（normalizePathKey），Windows 大小写不敏感；
 * - 路径用作 Map/Set 键（childrenByPath/expanded/分组 key 等）时必须先经
 *   normalizeFsPath / normalizePathKey，否则同一逻辑路径会因分隔符/大小写不同产生多个键。
 */

/** 存储/展示形态：统一反斜杠、去尾分隔符（盘符根保留）、保留大小写 */
export function normalizeFsPath(p: string): string {
  const bs = p.replace(/\//g, "\\");
  if (/^[A-Za-z]:\\$/.test(bs)) return bs;
  return bs.replace(/\\+$/, "");
}

/** 比较/键形态：统一反斜杠、去尾分隔符（含盘符根）、小写 */
export function normalizePathKey(p: string): string {
  return p.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
}

/** 路径相等（Windows 大小写不敏感，正/反斜杠等价） */
export function pathEquals(a: string, b: string): boolean {
  return normalizePathKey(a) === normalizePathKey(b);
}

/** 路径是否位于根目录之内（大小写不敏感，按分隔符边界判定） */
export function isPathUnderRoot(root: string, path: string): boolean {
  const a = normalizePathKey(root);
  const b = normalizePathKey(path);
  if (!a) return false;
  return b === a || b.startsWith(a + "\\");
}

/** Windows 路径拼接：base 统一反斜杠并去尾后追加反斜杠 + name */
export function joinFsPath(base: string, name: string): string {
  return normalizeFsPath(base).replace(/\\+$/, "") + "\\" + name;
}

/** Windows 路径 dirname：取最后一个分隔符前的部分；无分隔符返回空串 */
export function dirNameOf(path: string): string {
  const norm = normalizeFsPath(path);
  const idx = norm.lastIndexOf("\\");
  if (idx < 0) return "";
  const dir = norm.slice(0, idx).replace(/\\+$/, "");
  return /^[A-Za-z]:$/.test(dir) ? dir + "\\" : dir;
}

/** 路径相对 root：统一反斜杠、大小写不敏感地剥离 root 前缀；
 * path 等于 root 返回空串；非 root 下的路径原样返回 */
export function relPathOf(root: string, path: string): string {
  if (pathEquals(root, path)) return "";
  if (!isPathUnderRoot(root, path)) return path;
  const normRoot = normalizeFsPath(root).replace(/\\+$/, "");
  const normPath = normalizeFsPath(path);
  return normPath.slice(normRoot.length + 1);
}
