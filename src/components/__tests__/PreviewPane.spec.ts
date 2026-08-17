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
    expect(w.text()).toContain("图像");
    expect(w.text()).toContain("assets/logo.png");
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
