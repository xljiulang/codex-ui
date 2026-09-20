import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderMarkdown } from "../markdownRenderer";

// happy-dom 的 Worker 不会真正解析消息；测试固定走同步回退路径
const origWorker = globalThis.Worker;

/** 渲染结果的可见文本（折叠空白），用于断言「用户写的内容没有消失」 */
function visibleText(html: string): string {
  const el = document.createElement("div");
  el.innerHTML = html;
  return (el.textContent ?? "").replace(/\s+/g, " ").trim();
}

describe("renderMarkdown（同步回退路径）", () => {
  beforeEach(() => {
    (globalThis as Record<string, unknown>).Worker = undefined;
  });

  afterEach(() => {
    (globalThis as Record<string, unknown>).Worker = origWorker;
  });

  it("普通文本渲染为清洗后的 HTML", async () => {
    const html = await renderMarkdown("hello **world**");
    expect(html).toContain("<strong>world</strong>");
  });

  it("消息以代码块开头时保留根 <pre>（回归：DOMPurify 根元素修复）", async () => {
    const html = await renderMarkdown("```js\nconst x = 1;\n```");
    expect(html).toContain("<pre>");
    expect(html).toContain('class="language-js"');
  });

  it("file: 链接 href 不被剥离", async () => {
    const html = await renderMarkdown("[本地](file:///D:/a.txt)");
    expect(html).toContain('href="file:///D:/a.txt"');
  });

  it("盘符路径链接 href 不被剥离（marked 编码为 %5C）", async () => {
    const html = await renderMarkdown("[计划.md](D:\\codex\\a.md)");
    expect(html).toContain('href="D:%5Ccodex%5Ca.md"');
  });

  it("尖括号内容转义为字面文本且不生成元素（回归：<dd>/<ABCD> 渲染成空白）", async () => {
    const literal = [
      "<dd>",
      "<foo>",
      "<ABCD>",
      "<T>",
      "<user_instructions>",
      "</dd>",
      "<hr>",
      "<br>",
      '<img src="https://e.com/a.png">',
      "<!-- 注释 -->",
      "<![CDATA[x]]>",
      "<?php echo 1;?>",
    ];
    for (const src of literal) {
      const html = await renderMarkdown(src);
      expect(visibleText(html)).toBe(src);
      const el = document.createElement("div");
      el.innerHTML = html;
      expect(el.querySelector("*")).toBeNull();
    }
  });

  it("嵌在句子里的尖括号内容不丢标签、不整段消失", async () => {
    expect(visibleText(await renderMarkdown("a<b>c"))).toBe("a<b>c");
    expect(
      visibleText(await renderMarkdown("把 <user_message> 里的东西改掉")),
    ).toBe("把 <user_message> 里的东西改掉");
    const html = await renderMarkdown('<a href="D:/a.txt">x</a>');
    expect(visibleText(html)).toBe('<a href="D:/a.txt">x</a>');
    const el = document.createElement("div");
    el.innerHTML = html;
    expect(el.querySelector("a")).toBeNull();
  });

  it("marked 本就不当 HTML 的尖括号文本保持原样", async () => {
    for (const src of ["<中文标签>", "<1+2>", "<=>", "x < y", "<>"]) {
      expect(visibleText(await renderMarkdown(src))).toBe(src);
    }
  });

  it("代码块与行内代码里的标签仍是代码文本（不双转义）", async () => {
    const fenced = await renderMarkdown("```\n<dd>\n```");
    expect(fenced).toContain("<pre>");
    expect(visibleText(fenced)).toBe("<dd>");
    const inline = await renderMarkdown("`<dd>`");
    expect(inline).toContain("<code>");
    expect(visibleText(inline)).toBe("<dd>");
  });
});

/** 可控的假 Worker：测试手动决定何时回哪一条响应 */
class FakeWorker {
  static instances: FakeWorker[] = [];
  posted: { id: number; text: string }[] = [];
  terminated = false;
  onmessage: ((e: MessageEvent<{ id: number; raw: string }>) => void) | null =
    null;
  onerror: (() => void) | null = null;

  constructor() {
    FakeWorker.instances.push(this);
  }

  postMessage(msg: { id: number; text: string }) {
    this.posted.push(msg);
  }

  terminate() {
    this.terminated = true;
  }

  /** 模拟 Worker 回第 index 条响应（原始 HTML 交给主线程 sanitize） */
  respond(index: number, raw = "<p>ok</p>") {
    const msg = this.posted[index];
    this.onmessage?.({
      data: { id: msg.id, raw },
    } as MessageEvent<{ id: number; raw: string }>);
  }
}

/**
 * Worker 队列语义：大历史会话一次挂载上千个 MarkdownText，请求会在 Worker 里排队，
 * 单请求等待数秒是正常的；只有「队列非空且长时间无任何响应」才算卡死。
 */
describe("renderMarkdown（Worker 停滞看门狗）", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    FakeWorker.instances = [];
    vi.stubGlobal("Worker", FakeWorker);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("深队列下单条请求等 4s 不触发回退，仍由 Worker 结算", async () => {
    const mod = await import("../markdownRenderer");
    const results: string[] = [];
    const p1 = mod.renderMarkdown("a").then(() => results.push("a"));
    const p2 = mod.renderMarkdown("b").then(() => results.push("b"));
    const w = FakeWorker.instances[0];
    expect(w.posted).toHaveLength(2);
    expect(mod.pendingMarkdownCount()).toBe(2);

    await vi.advanceTimersByTimeAsync(4000);
    expect(results).toEqual([]);
    expect(w.terminated).toBe(false);

    w.respond(0);
    w.respond(1);
    await vi.advanceTimersByTimeAsync(0);
    await Promise.all([p1, p2]);
    expect(results).toEqual(["a", "b"]);
    expect(mod.pendingMarkdownCount()).toBe(0);
  });

  it("Worker 有响应即重置停滞看门狗，不因早前排队而误判卡死", async () => {
    const mod = await import("../markdownRenderer");
    void mod.renderMarkdown("a");
    const w = FakeWorker.instances[0];
    await vi.advanceTimersByTimeAsync(4000);

    w.respond(0);
    const second = mod.renderMarkdown("b");
    // 距首次 post 已 8s，但期间有响应：仍在正常排队，不得停用 Worker
    await vi.advanceTimersByTimeAsync(4000);
    expect(w.terminated).toBe(false);
    expect(mod.pendingMarkdownCount()).toBe(1);

    w.respond(1, "<p>b</p>");
    await vi.advanceTimersByTimeAsync(0);
    await expect(second).resolves.toContain("b");
  });

  it("Worker 停滞到点：停用并终止 Worker，全部待处理请求按 FIFO 分帧回退", async () => {
    const mod = await import("../markdownRenderer");
    const order: string[] = [];
    const texts = Array.from({ length: 20 }, (_, i) => `# h${i}`);
    const promises = texts.map((t, i) =>
      mod.renderMarkdown(t).then((html) => {
        order.push(`r${i}`);
        return html;
      }),
    );
    const w = FakeWorker.instances[0];
    expect(mod.pendingMarkdownCount()).toBe(20);

    await vi.advanceTimersByTimeAsync(5000);
    expect(w.terminated).toBe(true);
    expect(mod.pendingMarkdownCount()).toBe(0);

    // 分帧回退：让出主线程后逐批结算
    await vi.advanceTimersByTimeAsync(50);
    await Promise.all(promises);
    expect(order).toEqual(texts.map((_, i) => `r${i}`));

    // 之后不再尝试 Worker，直接走同步解析
    await expect(mod.renderMarkdown("# 之后")).resolves.toContain("<h1>");
    expect(FakeWorker.instances).toHaveLength(1);
  });
});
