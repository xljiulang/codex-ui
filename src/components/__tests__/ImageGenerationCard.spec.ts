import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => "asset://mock/" + encodeURIComponent(p),
}));

import ImageGenerationCard from "../ImageGenerationCard.vue";
import type { ImageGenerationItem } from "../../lib/types";

function makeItem(over: Partial<ImageGenerationItem>): ImageGenerationItem {
  return {
    id: "g1",
    type: "imageGeneration",
    status: "completed",
    ...over,
  };
}

describe("ImageGenerationCard 生成图片卡片", () => {
  it("本地路径 result 渲染为 asset 图片，状态完成", () => {
    const wrapper = mount(ImageGenerationCard, {
      props: { item: makeItem({ result: "C:\\tmp\\gen.png" }) },
    });
    expect(wrapper.text()).toContain("完成");
    const img = wrapper.find("img.image-gen-img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toContain("asset://mock/");
  });

  it("http(s) 与 data: URL 直接展示", () => {
    const w1 = mount(ImageGenerationCard, {
      props: { item: makeItem({ result: "https://x/y.png" }) },
    });
    expect(w1.find("img.image-gen-img").attributes("src")).toBe(
      "https://x/y.png",
    );
    const w2 = mount(ImageGenerationCard, {
      props: { item: makeItem({ result: "data:image/png;base64,AAAA" }) },
    });
    expect(w2.find("img.image-gen-img").attributes("src")).toContain(
      "data:image/png",
    );
  });

  it("对象 result 提取 path", () => {
    const wrapper = mount(ImageGenerationCard, {
      props: { item: makeItem({ result: { path: "D:/x/gen.png" } }) },
    });
    expect(wrapper.find("img.image-gen-img").attributes("src")).toContain(
      "asset://mock/",
    );
  });

  it("灯箱：点图片即关闭，data-copy-source 取原始来源", async () => {
    const wrapper = mount(ImageGenerationCard, {
      props: { item: makeItem({ result: "C:\\tmp\\gen.png" }) },
    });
    await wrapper.find("img.image-gen-img").trigger("click");
    const lightbox = wrapper.find(".lightbox");
    expect(lightbox.exists()).toBe(true);
    expect(lightbox.find("img").attributes("data-copy-source")).toBe(
      "C:\\tmp\\gen.png",
    );
    await lightbox.find("img").trigger("click");
    expect(wrapper.find(".lightbox").exists()).toBe(false);

    // data URL 来源原样带回（后端自行解析）
    const w2 = mount(ImageGenerationCard, {
      props: { item: makeItem({ result: "data:image/png;base64,AAAA" }) },
    });
    await w2.find("img.image-gen-img").trigger("click");
    expect(w2.find(".lightbox img").attributes("data-copy-source")).toBe(
      "data:image/png;base64,AAAA",
    );
  });

  it("生成中显示状态，无图时不显示原始数据", () => {
    const wrapper = mount(ImageGenerationCard, {
      props: { item: makeItem({ status: "in_progress" }) },
    });
    expect(wrapper.text()).toContain("生成中");
    expect(wrapper.find(".tool-json").exists()).toBe(false);
  });

  it("失败且无可用图时降级为原始数据", () => {
    const wrapper = mount(ImageGenerationCard, {
      props: {
        item: makeItem({ status: "failed", revisedPrompt: "改成红色" }),
      },
    });
    expect(wrapper.text()).toContain("失败");
    expect(wrapper.text()).toContain("改成红色");
    expect(wrapper.find(".tool-json").exists()).toBe(true);
  });
});
