/**
 * 剪贴板 HTML 链接改写：把 `<a href>` 转为「标题 + URL」纯文本。
 *
 * 背景：从浏览器复制链接（地址栏/网页锚链）粘贴到聊天输入框时，剪贴板 HTML 里是
 * `<a href="...">标题</a>`。ProseMirror 默认粘贴会经 StarterKit 的 Link 扩展解析成
 * link mark——编辑器里只显示标题、URL 藏在属性里；发送序列化又只看文本节点、丢 href，
 * 于是发出去只剩标题。这里在 `transformPastedHTML` 阶段把每个 `<a>` 就地改写为
 * 「标题 空格 URL」的纯文本，让 URL 在编辑器里可见、随发送保留。
 *
 * 只改 `<a>`：其余 HTML 结构与标签原样交回 ProseMirror 默认解析（粗体/列表等不变）。
 */

/**
 * 读取元素文本：取 `textContent`（递归覆盖 `<strong>`/`<span>` 等嵌套标签），
 * 并压平空白到单个空格后 trim；无文本时为空串。
 */
function anchorLabel(el: Element): string {
  return (el.textContent ?? "").replace(/\s+/g, " ").trim();
}

/**
 * 把剪贴板 HTML 里的每个 `<a href>` 改写为「标题 空格 URL」纯文本：
 * - 标题为空或纯空白时只保留 URL；
 * - `href` 首尾空白剔除；
 * - `mailto:` 等 URL 原样保留。
 * 无 `<a href>` 时原样返回输入。运行环境必须提供 `DOMParser`（浏览器/WebView2/jsdom）。
 */
export function linksToPlainText(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const anchors = doc.querySelectorAll("a[href]");
  if (!anchors.length) return html;

  for (const a of Array.from(anchors)) {
    const href = a.getAttribute("href")?.trim() ?? "";
    const label = anchorLabel(a);
    const text = label ? `${label} ${href}` : href;
    a.replaceWith(doc.createTextNode(text));
  }
  // 只取 body：DOMParser 会把 html 包裹进 head/body，避免把空 head/body 标签带进编辑器
  return doc.body.innerHTML;
}
