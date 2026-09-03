import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { flushPromises } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => "asset://mock/" + encodeURIComponent(p),
}));

import MessageItem from "../MessageItem.vue";
import { store } from "../../composables/useCodex";
import type { SessionTab } from "../../composables/useCodex";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";
import type { ThreadItem } from "../../lib/types";

const origWorker = globalThis.Worker;

/** MessageItem 挂载共用的会话标签 fixture（插件/技能缓存随标签提供） */
const TEST_TAB: SessionTab = makeSessionTab("s-test", "t-test", {
  plugins: {
    loaded: true,
    plugins: [
      {
        id: "documents@openai-primary-runtime",
        name: "documents",
        displayName: "Documents",
        description: "文档处理插件",
        path: "C:/x/documents",
        iconPath: "",
        iconUrl: "",
        brandColor: "",
      },
    ],
  },
  skills: {
    loaded: true,
    skills: [
      {
        name: "csharp-code-rules",
        key: "csharp-code-rules",
        path: "C:/x/SKILL.md",
        desc: "C# 代码规范技能",
        shortDesc: "C# 代码规范短说明",
      },
    ],
  },
});

function userItem(content: unknown[]): ThreadItem {
  return { id: "u1", type: "userMessage", content } as ThreadItem;
}

describe("用户消息中的图片附件", () => {
  beforeEach(() => {
    // 走同步回退解析，避免 happy-dom Worker 挂起
    (globalThis as Record<string, unknown>).Worker = undefined;
  });

  afterEach(() => {
    (globalThis as Record<string, unknown>).Worker = origWorker;
  });

  it("localImage 渲染为图片而不是路径文本", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          { type: "text", text: "里面只有一个人吗", text_elements: [] },
          { type: "localImage", path: "C:\\Users\\Admin\\Desktop\\lai\\out_0003.png" },
        ]),
      },
    });
    await flushPromises();
    const img = wrapper.find("img.user-image");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toContain("asset://mock/");
    expect(img.attributes("loading")).toBe("lazy");
    expect(img.attributes("decoding")).toBe("async");
    expect(wrapper.text()).toContain("里面只有一个人吗");
    expect(wrapper.text()).not.toContain("[图片:");
    expect(wrapper.text()).not.toContain("out_0003.png");
  });

  it("mention 附件渲染为 @名称", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          { type: "text", text: "看看这个", text_elements: [] },
          { type: "mention", name: "src/main.ts", path: "D:\\p\\src\\main.ts" },
        ]),
      },
    });
    expect(wrapper.text()).toContain("@src/main.ts");
    expect(wrapper.find(".mention-inline").exists()).toBe(true);
  });

  it("上下文压缩显示提示", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB, item: { id: "c1", type: "contextCompaction" } as ThreadItem },
    });
    expect(wrapper.text()).toContain("上下文已压缩");
  });

  it("commentary 阶段显示进行中徽标，final_answer 不显示", () => {
    const w1 = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "a1",
          type: "agentMessage",
          text: "思考中…",
          phase: "commentary",
          streaming: true,
        } as ThreadItem,
      },
    });
    expect(w1.find(".phase-badge").exists()).toBe(true);
    // 流式结束后（即使 phase 仍是 commentary）不再显示进行中
    const w3 = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "a3",
          type: "agentMessage",
          text: "思考完毕",
          phase: "commentary",
          streaming: false,
        } as ThreadItem,
      },
    });
    expect(w3.find(".phase-badge").exists()).toBe(false);
    const w2 = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: { id: "a2", type: "agentMessage", text: "答案", phase: "final_answer" } as ThreadItem,
      },
    });
    expect(w2.find(".phase-badge").exists()).toBe(false);
  });

  it("imageView 渲染为图片", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: { id: "v1", type: "imageView", path: "C:\\x\\y.png" } as ThreadItem,
      },
    });
    const img = wrapper.find("img.user-image");
    expect(img.exists()).toBe(true);
    expect(img.attributes("loading")).toBe("lazy");
  });

  it("流式中的助手消息显示闪烁光标，完成后消失", () => {
    const w1 = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "s1",
          type: "agentMessage",
          text: "正在生成…",
          streaming: true,
        } as ThreadItem,
      },
    });
    expect(w1.find(".stream-cursor").exists()).toBe(true);
    const w2 = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "s2",
          type: "agentMessage",
          text: "完成",
          streaming: false,
        } as ThreadItem,
      },
    });
    expect(w2.find(".stream-cursor").exists()).toBe(false);
  });

  it("同一条消息混合 @ 与 $：文件段与技能链接都渲染为引用标签", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n[$csharp-code-rules](C:/x/SKILL.md) 按规则检查\n",
            text_elements: [],
          },
          {
            type: "skill",
            name: "csharp-code-rules",
            path: "C:/x/SKILL.md",
          },
        ]),
      },
    });
    await flushPromises();
    expect(wrapper.text()).toContain("@a.cs");
    expect(wrapper.text()).toContain("$csharp-code-rules");
    expect(wrapper.text()).toContain("按规则检查");
    // 结构化 skill 项存在时文本链接不重复渲染
    expect(wrapper.text().split("$csharp-code-rules").length - 1).toBe(1);
  });

  it("只有文本技能链接（无结构化项）时也能渲染 $ 标签", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n[$csharp-code-rules](C:/x/SKILL.md) 按规则检查\n",
            text_elements: [],
          },
        ]),
      },
    });
    expect(wrapper.text()).toContain("@a.cs");
    expect(wrapper.text()).toContain("$csharp-code-rules");
  });

  it("插件链接 [@documents](path) 渲染 @documents 标签", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "[@documents](C:/x/plugins/documents) 看下\n",
            text_elements: [],
          },
        ]),
      },
    });
    expect(wrapper.text()).toContain("@documents");
    expect(wrapper.text()).not.toContain("$documents");
  });

  it("无前缀本地路径链接 [a.cs](src/a.cs) 渲染为 @a.cs 文件 chip", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "[a.cs](src/a.cs) 看下",
            text_elements: [],
          },
        ]),
      },
    });
    expect(wrapper.text()).toContain("@a.cs");
    expect(wrapper.find(".mention-inline").exists()).toBe(true);
  });

  it("插件链接 [@documents](plugin://...) 渲染 @documents 标签", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "[@documents](plugin://documents@openai-primary-runtime) 看下",
            text_elements: [],
          },
        ]),
      },
    });
    expect(wrapper.text()).toContain("@documents");
    expect(wrapper.text()).not.toContain("$documents");
  });

  it("插件链接 + 同名结构化项去重：只渲染一个 @documents", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "[@documents](C:/x/plugins/documents) 看下\n",
            text_elements: [],
          },
          {
            type: "skill",
            name: "documents",
            path: "C:/x/plugins/documents",
          },
        ]),
      },
    });
    await flushPromises();
    expect(wrapper.text()).toContain("@documents");
    expect(wrapper.text().split("@documents").length - 1).toBe(1);
    expect(wrapper.text()).not.toContain("$documents");
  });

  it("技能链接与同名结构化项去重：只渲染一个 $csharp-code-rules", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "[$csharp-code-rules](C:/x/SKILL.md) 看下\n",
            text_elements: [],
          },
          {
            type: "skill",
            name: "csharp-code-rules",
            path: "C:/x/SKILL.md",
          },
        ]),
      },
    });
    await flushPromises();
    expect(wrapper.text()).toContain("$csharp-code-rules");
    expect(wrapper.text().split("$csharp-code-rules").length - 1).toBe(1);
  });

  it("内联链接回显顺序与输入一致：文本-文件-文本-技能-文本", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "先 [@a.cs](D:/repo/a.cs) 中间 [$csharp-code-rules](C:/x/SKILL.md) 结尾",
            text_elements: [],
          },
        ]),
      },
    });
    await flushPromises();
    const bubble = wrapper.find(".bubble");
    const text = bubble.text();
    expect(text).toContain("先");
    expect(text).toContain("中间");
    expect(text).toContain("结尾");
    expect(text.indexOf("先")).toBeLessThan(text.indexOf("@a.cs"));
    expect(text.indexOf("@a.cs")).toBeLessThan(text.indexOf("中间"));
    expect(text.indexOf("中间")).toBeLessThan(
      text.indexOf("$csharp-code-rules"),
    );
    expect(text.indexOf("$csharp-code-rules")).toBeLessThan(
      text.indexOf("结尾"),
    );
    expect(wrapper.findAll(".mention-inline").length).toBe(2);
  });

  it("混编回显片段为内联盒子：气泡直接子节点无块级 .md", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "先 [@a.cs](D:/repo/a.cs) 中间 [$csharp-code-rules](C:/x/SKILL.md) 结尾",
            text_elements: [],
          },
        ]),
      },
    });
    await flushPromises();
    const bubble = wrapper.find(".bubble").element as HTMLElement;
    const directChildren = Array.from(bubble.children);
    // 直接子节点只允许 chip（.mention-inline）与内联包装（.md-inline），不存在块级 .md
    expect(
      directChildren.some(
        (el) =>
          el.classList.contains("md") && !el.classList.contains("md-inline"),
      ),
    ).toBe(false);
    expect(wrapper.findAll(".bubble > .md-inline").length).toBe(3);
    expect(wrapper.findAll(".bubble > .mention-inline").length).toBe(2);
    const texts = wrapper
      .findAll(".bubble > .md-inline")
      .map((x) => x.text().trim());
    expect(texts).toEqual(["先", "中间", "结尾"]);
  });

  it("文件/技能 chip 可点击（测试钩子下回退 reveal_path），插件 chip 不可点击", async () => {
    store.workspace = "D:/repo";
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__ = true;
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST_LOG__ = [];
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "[a.cs](src/a.cs) 与 [$csharp-code-rules](C:/x/SKILL.md) 与 [@documents](plugin://documents@openai-primary-runtime)",
            text_elements: [],
          },
        ]),
      },
    });
    await flushPromises();
    const chips = wrapper.findAll(".mention-inline");
    const fileChip = chips.find((c) => c.text() === "@a.cs");
    const skillChip = chips.find((c) => c.text() === "$csharp-code-rules");
    const pluginChip = chips.find((c) => c.text() === "@documents");
    expect(fileChip?.classes()).toContain("clickable");
    expect(skillChip?.classes()).toContain("clickable");
    expect(pluginChip?.classes()).not.toContain("clickable");
    expect(fileChip?.attributes("title")).toBeUndefined();
    // 悬浮卡片：文件显示路径、技能/插件显示说明
    await fileChip!.trigger("mouseenter");
    await new Promise((r) => setTimeout(r, 160));
    expect(
      document.body.querySelector(".ref-tooltip")?.textContent,
    ).toContain("src/a.cs");
    await fileChip!.trigger("mouseleave");
    await skillChip!.trigger("mouseenter");
    await new Promise((r) => setTimeout(r, 160));
    expect(
      document.body.querySelector(".ref-tooltip")?.textContent,
    ).toContain("C# 代码规范技能");
    await skillChip!.trigger("mouseleave");
    await pluginChip!.trigger("mouseenter");
    await new Promise((r) => setTimeout(r, 160));
    expect(
      document.body.querySelector(".ref-tooltip")?.textContent,
    ).toContain("文档处理插件");
    await pluginChip!.trigger("mouseleave");
    await fileChip!.trigger("click");
    await skillChip!.trigger("click");
    await pluginChip!.trigger("click");
    await flushPromises();
    const log = (window as unknown as Record<string, unknown>)
      .__CODEX_UI_TEST_LOG__ as { cmd: string; args: { path: string } }[];
    expect(
      log
        .filter((l) => l.cmd === "reveal_path")
        .map((l) => l.args.path),
    ).toEqual(["D:\\repo\\src\\a.cs", "C:\\x\\SKILL.md"]);
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST__ = false;
    (window as unknown as Record<string, unknown>).__CODEX_UI_TEST_LOG__ = [];
  });

  it("含 Files 段的旧消息（无内联链接）仍按 legacy 路径渲染", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## My request:\n看看这个\n",
            text_elements: [],
          },
        ]),
      },
    });
    await flushPromises();
    expect(wrapper.find(".bubble").text()).toContain("@a.cs");
    expect(wrapper.find(".bubble").text()).toContain("看看这个");
    expect(wrapper.find(".bubble").text()).not.toContain("Files mentioned");
  });

  it("多个文件引用渲染多个 @ 标签，正文不含协议段落", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          {
            type: "text",
            text: "\n# Files mentioned by the user:\n\n## a.cs: D:/repo/a.cs\n\n## b.txt: D:/repo/b.txt\n\n## My request:\n看看这两个文件\n",
            text_elements: [],
          },
        ]),
      },
    });
    await flushPromises();
    expect(wrapper.text()).toContain("@a.cs");
    expect(wrapper.text()).toContain("@b.txt");
    expect(wrapper.text()).toContain("看看这两个文件");
    expect(wrapper.text()).not.toContain("# Files mentioned by the user:");
    expect(wrapper.text()).not.toContain("## My request:");
  });

  it("用户消息文本按 Markdown 渲染", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          { type: "text", text: "# 标题\n\n**加粗**", text_elements: [] },
        ]),
      },
    });
    await flushPromises();
    expect(wrapper.find(".msg-user .md h1").text()).toBe("标题");
    expect(wrapper.find(".msg-user .md strong").text()).toBe("加粗");
  });

  it("用户消息与助手最终答复都显示时间戳", () => {
    const today = new Date();
    const ts = new Date(
      today.getFullYear(),
      today.getMonth(),
      today.getDate(),
      14,
      5,
    ).getTime();
    const user = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "u2",
          type: "userMessage",
          startedAtMs: ts,
          content: [{ type: "text", text: "hi", text_elements: [] }],
        } as ThreadItem,
      },
    });
    expect(user.find(".msg-time").exists()).toBe(true);
    expect(user.find(".msg-time").text()).toBe("14:05");

    const agent = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "a2",
          type: "agentMessage",
          phase: "final_answer",
          streaming: false,
          startedAtMs: ts,
          text: "答案",
        } as ThreadItem,
      },
    });
    expect(agent.find(".msg-time").exists()).toBe(true);
    expect(agent.find(".msg-time").text()).toBe("14:05");
  });

  it("非当天消息时间带日期", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 8, 3, 12, 0));
    try {
      const user = mount(MessageItem, {
        props: {
          tab: TEST_TAB,
          item: {
            id: "u3",
            type: "userMessage",
            startedAtMs: new Date(2026, 7, 9, 14, 5).getTime(),
            content: [{ type: "text", text: "hi", text_elements: [] }],
          } as ThreadItem,
        },
      });
      expect(user.find(".msg-time").exists()).toBe(true);
      expect(user.find(".msg-time").text()).toBe("8月9日 14:05");
    } finally {
      vi.useRealTimers();
    }
  });

  it("仅最终答复（非流式）包 agent-final 卡片，流式与 commentary 不包", async () => {
    const final = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "f1",
          type: "agentMessage",
          phase: "final_answer",
          streaming: false,
          text: "最终答案",
        } as ThreadItem,
      },
    });
    await flushPromises();
    expect(final.find(".agent-final").exists()).toBe(true);
    expect(final.find(".agent-final").text()).toContain("最终答案");

    const streaming = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "s1",
          type: "agentMessage",
          phase: "final_answer",
          streaming: true,
          text: "正在生成…",
        } as ThreadItem,
      },
    });
    expect(streaming.find(".agent-final").exists()).toBe(false);

    const commentary = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "c1",
          type: "agentMessage",
          phase: "commentary",
          streaming: false,
          text: "中间过程",
        } as ThreadItem,
      },
    });
    expect(commentary.find(".agent-final").exists()).toBe(false);
  });

  it("图片点击打开灯箱，Esc 关闭", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([
          { type: "text", text: "看这张图", text_elements: [] },
          { type: "localImage", path: "C:\\x\\a.png" },
        ]),
      },
    });
    await wrapper.find(".user-image").trigger("click");
    await flushPromises();
    expect(wrapper.find(".lightbox").exists()).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(wrapper.find(".lightbox").exists()).toBe(false);
  });

  it("图片加载失败显示占位", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: userItem([{ type: "localImage", path: "C:\\x\\bad.png" }]),
      },
    });
    await wrapper.find(".user-image").trigger("error");
    await flushPromises();
    expect(wrapper.find(".img-fallback").text()).toBe("图片加载失败");
  });

  it("未知消息类型显示友好提示并可展开原始数据", async () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB, item: { id: "x1", type: "weirdType" } as ThreadItem },
    });
    expect(wrapper.text()).toContain("暂不支持显示的项目类型：weirdType");
    await wrapper.find(".unknown-toggle").trigger("click");
    expect(wrapper.find(".unknown-raw").exists()).toBe(true);
  });

  it("助手最终答复可复制全文", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "a1",
          type: "agentMessage",
          phase: "final_answer",
          streaming: false,
          text: "完整答案",
        } as ThreadItem,
      },
    });
    const btn = wrapper.find(".msg-agent .copy-btn");
    expect(btn.attributes("aria-label")).toBe("复制");
    await btn.trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("完整答案");
    expect(btn.text()).toBe("已复制");
  });

  it("用户消息正文支持复制正文文本", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const wrapper = mount(MessageItem, {
      props: {
        tab: TEST_TAB,
        item: userItem([
          { type: "text", text: "要复制的用户文本", text_elements: [] },
        ]),
      },
    });
    await flushPromises();
    const btn = wrapper.find(".msg-user .copy-btn");
    expect(btn.exists()).toBe(true);
    expect(btn.attributes("aria-label")).toBe("复制");
    await btn.trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("要复制的用户文本");
    expect(btn.text()).toBe("已复制");
  });

});

describe("子代理活动提示", () => {
  it("subAgentActivity 渲染 kind 与 agentPath", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "s1",
          type: "subAgentActivity",
          kind: "subAgentSpawned",
          agentPath: "/root/agent-1",
        } as ThreadItem,
      },
    });
    expect(wrapper.find(".sub-agent-note").exists()).toBe(true);
    expect(wrapper.text()).toContain("子代理活动");
    expect(wrapper.text()).toContain("subAgentSpawned");
    expect(wrapper.text()).toContain("/root/agent-1");
  });
});

describe("执行计划用户消息卡片", () => {
  it("命中 PLEASE IMPLEMENT THIS PLAN 前缀：渲染卡片且默认不显示正文", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "u1",
          type: "userMessage",
          content: [
            {
              type: "text",
              text: "PLEASE IMPLEMENT THIS PLAN:\n# 计划A\n- 步骤1",
              text_elements: [],
            },
          ],
        } as ThreadItem,
      },
    });
    expect(wrapper.find(".assistant-card").exists()).toBe(true);
    // 标题取自计划本身（首行标题），正文默认折叠不可见
    expect(wrapper.find(".assistant-card .assistant-card-title").text()).toBe("计划A");
    expect(wrapper.text()).not.toContain("步骤1");
  });

  it("带内联引用的执行计划消息：引用照常渲染", () => {
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "u1",
          type: "userMessage",
          content: [
            { type: "mention", name: "a.txt", path: "D:\\p\\a.txt" },
            {
              type: "text",
              text: "PLEASE IMPLEMENT THIS PLAN:\n- 步骤",
              text_elements: [],
            },
          ],
        } as ThreadItem,
      },
    });
    expect(wrapper.find(".assistant-card").exists()).toBe(true);
    expect(wrapper.find(".mention-inline").exists()).toBe(true);
  });

  it("非前缀消息仍走普通 Markdown（回归）", async () => {
    // 走同步回退解析，避免 happy-dom Worker 挂起（与文件内既有图片用例一致）
    (globalThis as Record<string, unknown>).Worker = undefined;
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "u1",
          type: "userMessage",
          content: [
            { type: "text", text: "普通问题", text_elements: [] },
          ],
        } as ThreadItem,
      },
    });
    await flushPromises();
    expect(wrapper.find(".assistant-card").exists()).toBe(false);
    expect(wrapper.text()).toContain("普通问题");
  });

  it("助理端 plan 条目渲染为计划卡片：默认展开，可收起", async () => {
    (globalThis as Record<string, unknown>).Worker = undefined;
    const wrapper = mount(MessageItem, {
      props: { tab: TEST_TAB,
        item: {
          id: "p1",
          type: "plan",
          text: "# 方案\n- 步骤",
        } as ThreadItem,
      },
    });
    await flushPromises();
    const card = wrapper.find(".assistant-card");
    expect(card.exists()).toBe(true);
    // 标题取自计划自身（# 方案），正文默认展开
    expect(card.find(".assistant-card-title").text()).toBe("方案");
    expect(card.find(".assistant-card-body").exists()).toBe(true);
    expect(card.text()).toContain("步骤");
    await card.find(".assistant-card-toggle").trigger("click");
    expect(card.find(".assistant-card-body").exists()).toBe(false);
  });
});
