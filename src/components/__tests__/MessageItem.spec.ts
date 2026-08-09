import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { flushPromises } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => "asset://mock/" + encodeURIComponent(p),
}));

import MessageItem from "../MessageItem.vue";
import type { ThreadItem } from "../../lib/types";

const origWorker = globalThis.Worker;

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
      props: {
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
      props: {
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
      props: { item: { id: "c1", type: "contextCompaction" } as ThreadItem },
    });
    expect(wrapper.text()).toContain("上下文已压缩");
  });

  it("commentary 阶段显示进行中徽标，final_answer 不显示", () => {
    const w1 = mount(MessageItem, {
      props: {
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
      props: {
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
      props: {
        item: { id: "a2", type: "agentMessage", text: "答案", phase: "final_answer" } as ThreadItem,
      },
    });
    expect(w2.find(".phase-badge").exists()).toBe(false);
  });

  it("imageView 渲染为图片", () => {
    const wrapper = mount(MessageItem, {
      props: {
        item: { id: "v1", type: "imageView", path: "C:\\x\\y.png" } as ThreadItem,
      },
    });
    const img = wrapper.find("img.user-image");
    expect(img.exists()).toBe(true);
    expect(img.attributes("loading")).toBe("lazy");
  });

  it("流式中的助手消息显示闪烁光标，完成后消失", () => {
    const w1 = mount(MessageItem, {
      props: {
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
      props: {
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
      props: {
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
      props: {
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

  it("多个文件引用渲染多个 @ 标签，正文不含协议段落", async () => {
    const wrapper = mount(MessageItem, {
      props: {
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
      props: {
        item: userItem([
          { type: "text", text: "# 标题\n\n**加粗**", text_elements: [] },
        ]),
      },
    });
    await flushPromises();
    expect(wrapper.find(".msg-user .md h1").text()).toBe("标题");
    expect(wrapper.find(".msg-user .md strong").text()).toBe("加粗");
  });

  it("用户消息不显示时间戳，助手最终答复显示时间戳", () => {
    const ts = new Date(2026, 7, 9, 14, 5).getTime();
    const user = mount(MessageItem, {
      props: {
        item: {
          id: "u2",
          type: "userMessage",
          startedAtMs: ts,
          content: [{ type: "text", text: "hi", text_elements: [] }],
        } as ThreadItem,
      },
    });
    expect(user.find(".msg-time").exists()).toBe(false);

    const agent = mount(MessageItem, {
      props: {
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

  it("图片点击打开灯箱，Esc 关闭", async () => {
    const wrapper = mount(MessageItem, {
      props: {
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
      props: {
        item: userItem([{ type: "localImage", path: "C:\\x\\bad.png" }]),
      },
    });
    await wrapper.find(".user-image").trigger("error");
    await flushPromises();
    expect(wrapper.find(".img-fallback").text()).toBe("图片加载失败");
  });

  it("未知消息类型显示友好提示并可展开原始数据", async () => {
    const wrapper = mount(MessageItem, {
      props: { item: { id: "x1", type: "weirdType" } as ThreadItem },
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
      props: {
        item: {
          id: "a1",
          type: "agentMessage",
          phase: "final_answer",
          streaming: false,
          text: "完整答案",
        } as ThreadItem,
      },
    });
    const btn = wrapper.find(".agent-actions .msg-action-btn");
    expect(btn.attributes("aria-label")).toBe("复制回复");
    await btn.trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("完整答案");
    expect(btn.text()).toBe("已复制");
  });

});
