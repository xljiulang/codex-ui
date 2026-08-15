import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

const { mockPage } = vi.hoisted(() => ({
  mockPage: {
    getViewport: vi.fn(() => ({ width: 100, height: 200 })),
    render: vi.fn(() => ({ promise: Promise.resolve() })),
  },
}));

vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: { workerSrc: "" },
  getDocument: vi.fn(),
}));

vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "" }));

import { getDocument } from "pdfjs-dist";
import PdfPreviewPane from "../PdfPreviewPane.vue";
import type { PreviewEditorTab } from "../../composables/useEditorTabs";

const mockedGetDocument = vi.mocked(getDocument);

function makeTab(over: Partial<PreviewEditorTab> = {}): PreviewEditorTab {
  return {
    kind: "preview",
    icon: "file",
    previewType: "pdf",
    id: "p1",
    workspace: "D:/repo",
    path: "doc.pdf",
    title: "doc.pdf",
    loading: false,
    error: "",
    imageUrl: "",
    pdfData: new Uint8Array([1, 2, 3]),
    pageCount: null,
    ...over,
  };
}

function mockDoc(numPages = 3) {
  const doc = {
    numPages,
    getPage: vi.fn(async () => mockPage),
    destroy: vi.fn().mockResolvedValue(undefined),
  };
  mockedGetDocument.mockReturnValue({
    promise: Promise.resolve(doc),
    destroy: vi.fn().mockResolvedValue(undefined),
  } as never);
  return doc;
}

describe("PdfPreviewPane PDF 预览", () => {
  beforeEach(() => {
    mockedGetDocument.mockReset();
    mockPage.getViewport.mockClear();
    mockPage.render.mockClear();
    mockDoc();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("加载文档后渲染页码 1 / N，首页禁用上一页", async () => {
    const w = mount(PdfPreviewPane, { props: { tab: makeTab() } });
    await flushPromises();
    expect(w.text()).toContain("1 / 3");
    const prev = w.find('[aria-label="上一页"]');
    expect(prev.attributes("disabled")).toBeDefined();
    expect(w.find('[aria-label="下一页"]').attributes("disabled")).toBeUndefined();
  });

  it("下一页/上一页切换页码，边界禁用", async () => {
    const w = mount(PdfPreviewPane, { props: { tab: makeTab() } });
    await flushPromises();
    await w.find('[aria-label="下一页"]').trigger("click");
    expect(w.text()).toContain("2 / 3");
    await w.find('[aria-label="下一页"]').trigger("click");
    await w.find('[aria-label="下一页"]').trigger("click");
    expect(w.text()).toContain("3 / 3");
    expect(w.find('[aria-label="下一页"]').attributes("disabled")).toBeDefined();
    await w.find('[aria-label="上一页"]').trigger("click");
    expect(w.text()).toContain("2 / 3");
  });

  it("缩放夹在 25% ~ 400% 之间，适应宽度复位为 100%", async () => {
    const w = mount(PdfPreviewPane, { props: { tab: makeTab() } });
    await flushPromises();
    for (let i = 0; i < 20; i++) {
      await w.find('[aria-label="放大"]').trigger("click");
    }
    expect(w.text()).toContain("400%");
    expect(w.find('[aria-label="放大"]').attributes("disabled")).toBeDefined();
    for (let i = 0; i < 20; i++) {
      await w.find('[aria-label="缩小"]').trigger("click");
    }
    expect(w.text()).toContain("25%");
    expect(w.find('[aria-label="缩小"]').attributes("disabled")).toBeDefined();
    await w.find(".pdf-toolbar-fit").trigger("click");
    expect(w.text()).toContain("100%");
  });

  it("空数据提示 PDF 内容为空", async () => {
    const w = mount(PdfPreviewPane, {
      props: { tab: makeTab({ pdfData: new Uint8Array(0) }) },
    });
    await flushPromises();
    expect(w.text()).toContain("无法预览该 PDF（PDF 内容为空）");
  });

  it("加载失败展示错误信息", async () => {
    mockedGetDocument.mockReturnValue({
      promise: Promise.reject(new Error("损坏的文件")),
      destroy: vi.fn().mockResolvedValue(undefined),
    } as never);
    const w = mount(PdfPreviewPane, { props: { tab: makeTab() } });
    await flushPromises();
    expect(w.text()).toContain("无法预览该 PDF（Error: 损坏的文件）");
  });
});
