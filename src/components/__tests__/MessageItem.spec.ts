import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => "asset://mock/" + encodeURIComponent(p),
}));

import MessageItem from "../MessageItem.vue";
import type { ThreadItem } from "../../lib/types";

function userItem(content: unknown[]): ThreadItem {
  return { id: "u1", type: "userMessage", content } as ThreadItem;
}

describe("用户消息中的图片附件", () => {
  it("localImage 渲染为图片而不是路径文本", () => {
    const wrapper = mount(MessageItem, {
      props: {
        item: userItem([
          { type: "text", text: "里面只有一个人吗", text_elements: [] },
          { type: "localImage", path: "C:\\Users\\Admin\\Desktop\\lai\\out_0003.png" },
        ]),
      },
    });
    const img = wrapper.find("img.user-image");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toContain("asset://mock/");
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
    expect(wrapper.find("img.user-image").exists()).toBe(true);
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
});
