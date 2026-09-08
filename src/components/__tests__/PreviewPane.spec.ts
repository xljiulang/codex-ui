import { describe, expect, it } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { reactive } from "vue";
import PreviewPane from "../PreviewPane.vue";
import type { PreviewEditorTab } from "../../composables/useEditorTabs";

function makeTab(
  over: Partial<PreviewEditorTab> = {},
): PreviewEditorTab {
  return {
    kind: "preview",
    icon: "file",
    previewType: "image",
    id: "p1",
    workspace: "D:/repo",
    path: "assets/logo.png",
    title: "logo.png",
    loading: false,
    error: "",
    imageUrl: "asset://D:/repo/assets/logo.png",
    pdfData: null,
    xlsxData: null,
    docxData: null,
    pptxData: null,
    xlsxSheetIndex: 0,
    pageCount: null,
    stale: false,
    ...over,
  };
}

describe("PreviewPane 预览标签", () => {
  it("图像类型渲染真实 img 与类型标签", () => {
    const w = mount(PreviewPane, { props: { tab: makeTab() } });
    const img = w.find(".preview-image img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("asset://D:/repo/assets/logo.png");
    expect(w.find(".preview-zoom-toolbar").exists()).toBe(true);
    expect(w.text()).toContain("图像");
    expect(w.text()).toContain("assets/logo.png");
  });

  it("图像预览结构：缩放控件并入头部行，图片位于 preview-image-stage 内", () => {
    const w = mount(PreviewPane, { props: { tab: makeTab() } });
    // 缩放控件渲染在头部右侧动作区
    expect(
      w.find(".preview-head-actions .preview-zoom-toolbar").exists(),
    ).toBe(true);
    const stage = w.find(".preview-image-stage");
    expect(stage.exists()).toBe(true);
    expect(stage.find("img").exists()).toBe(true);
    // 内容区只剩滚动区，不再有独立工具栏行
    const children = Array.from(w.find(".preview-image").element.children);
    expect(children.length).toBe(1);
    expect(children[0].className).toContain("preview-image-stage");
  });

  it("图像内容区 Ctrl+滚轮缩放", async () => {
    const w = mount(PreviewPane, { props: { tab: makeTab() } });
    await flushPromises();
    const stage = w.find(".preview-image-stage").element;
    const e = new WheelEvent("wheel", {
      deltaY: -100,
      bubbles: true,
      cancelable: true,
    });
    // happy-dom 的 WheelEvent 构造不接收 ctrlKey，手动定义
    Object.defineProperty(e, "ctrlKey", { value: true, configurable: true });
    stage.dispatchEvent(e);
    await flushPromises();
    expect(w.find(".preview-zoom-percent").text()).toBe("125%");
  });

  it("图像缩放：放大按原始像素等比缩放，适应窗口复位", async () => {
    const w = mount(PreviewPane, { props: { tab: makeTab() } });
    const img = w.find(".preview-image img").element as HTMLImageElement;
    Object.defineProperty(img, "naturalWidth", {
      value: 200,
      configurable: true,
    });
    Object.defineProperty(img, "naturalHeight", {
      value: 100,
      configurable: true,
    });
    img.dispatchEvent(new Event("load"));

    const btns = w.findAll(".preview-zoom-btn");
    expect(w.find(".preview-zoom-percent").text()).toBe("100%");
    await btns[1].trigger("click"); // ＋ → 125%
    expect(w.find(".preview-zoom-percent").text()).toBe("125%");
    expect(w.find(".preview-image img").attributes("style")).toContain(
      "width: 250px",
    );
    expect(w.find(".preview-image img").attributes("style")).toContain(
      "height: 125px",
    );
    expect(w.find(".preview-image img").classes()).toContain("zoomed");

    await btns[2].trigger("click"); // 适应窗口
    expect(w.find(".preview-zoom-percent").text()).toBe("100%");
    expect(w.find(".preview-image img").attributes("style") ?? "").not.toContain(
      "250px",
    );
    expect(w.find(".preview-image img").classes()).not.toContain("zoomed");
  });

  it("imageUrl 变化（外部刷新）复位缩放", async () => {
    const tab = reactive(makeTab());
    const w = mount(PreviewPane, { props: { tab } });
    const img = w.find(".preview-image img").element as HTMLImageElement;
    Object.defineProperty(img, "naturalWidth", {
      value: 200,
      configurable: true,
    });
    Object.defineProperty(img, "naturalHeight", {
      value: 100,
      configurable: true,
    });
    img.dispatchEvent(new Event("load"));
    await w.findAll(".preview-zoom-btn")[1].trigger("click");
    expect(w.find(".preview-zoom-percent").text()).toBe("125%");

    tab.imageUrl = "asset://D:/repo/assets/logo.png?t=456";
    await flushPromises();
    expect(w.find(".preview-zoom-percent").text()).toBe("100%");
  });

  it("图像加载失败显示友好提示", async () => {
    const w = mount(PreviewPane, { props: { tab: makeTab() } });
    await w.find(".preview-image img").trigger("error");
    expect(w.text()).toContain("无法预览该图片");
  });

  it("imageUrl 变化（外部刷新）复位图片加载失败状态", async () => {
    const tab = reactive(makeTab());
    const w = mount(PreviewPane, { props: { tab } });
    await w.find(".preview-image img").trigger("error");
    expect(w.text()).toContain("无法预览该图片");
    tab.imageUrl = "asset://D:/repo/assets/logo.png?t=123";
    await flushPromises();
    expect(w.text()).not.toContain("无法预览该图片");
    expect(w.find(".preview-image img").exists()).toBe(true);
  });

  it("loading/error 状态分别展示", () => {
    const loading = mount(PreviewPane, {
      props: { tab: makeTab({ loading: true }) },
    });
    expect(loading.text()).toContain("正在加载预览…");
    const errored = mount(PreviewPane, {
      props: { tab: makeTab({ error: "文件过大" }) },
    });
    expect(errored.text()).toContain("无法预览该文件（文件过大）");
  });

  it("PDF 类型委托给 PdfPreviewPane", () => {
    const w = mount(PreviewPane, {
      props: { tab: makeTab({ previewType: "pdf", pdfData: new Uint8Array([1]) }) },
      global: {
        stubs: { PdfPreviewPane: { template: '<div class="pdf-stub" />' } },
      },
    });
    expect(w.find(".pdf-stub").exists()).toBe(true);
    expect(w.text()).toContain("PDF");
  });
});
