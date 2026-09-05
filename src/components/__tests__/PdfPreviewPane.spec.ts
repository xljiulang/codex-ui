import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { reactive } from "vue";

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
    xlsxData: null,
    docxData: null,
    xlsxSheetIndex: 0,
    pageCount: null,
    stale: false,
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

  it("pdfData 变化（外部刷新）触发重载并保持页码与缩放", async () => {
    const tab = reactive(makeTab());
    const w = mount(PdfPreviewPane, { props: { tab } });
    await flushPromises();
    await w.find('[aria-label="下一页"]').trigger("click");
    expect(w.text()).toContain("2 / 3");
    await w.find('[aria-label="放大"]').trigger("click");
    expect(w.text()).toContain("125%");
    const callsBefore = mockedGetDocument.mock.calls.length;

    // 同一标签对象替换 pdfData：应重新加载（旧实现只监听 props.tab 对象不会触发）
    tab.pdfData = new Uint8Array([4, 5, 6]);
    await flushPromises();
    expect(mockedGetDocument.mock.calls.length).toBeGreaterThan(callsBefore);
    await flushPromises();
    expect(w.text()).toContain("2 / 3");
    expect(w.text()).toContain("125%");
  });

  it("空数据提示 PDF 内容为空", async () => {
    const w = mount(PdfPreviewPane, {
      props: { tab: makeTab({ pdfData: new Uint8Array(0) }) },
    });
    await flushPromises();
    expect(w.text()).toContain("无法预览该 PDF（PDF 内容为空）");
  });

  it("传给 pdfjs 的是副本：模拟 pdf.js 转移 buffer 后标签 pdfData 仍完整", async () => {
    const tab = reactive(makeTab()); // pdfData = new Uint8Array([1, 2, 3])
    const doc = {
      numPages: 3,
      getPage: vi.fn(async () => mockPage),
      destroy: vi.fn().mockResolvedValue(undefined),
    };
    let receivedIsCopy = false;
    let receivedLenBeforeTransfer = 0;
    mockedGetDocument.mockImplementation((params) => {
      // 模拟 pdf.js 真实行为：把传入 data 的 ArrayBuffer 转移（detach）
      const data = params?.data;
      if (data instanceof Uint8Array) {
        receivedIsCopy = data !== tab.pdfData;
        receivedLenBeforeTransfer = data.length;
        try {
          structuredClone(data, { transfer: [data.buffer] });
        } catch {}
      }
      return {
        promise: Promise.resolve(doc),
        destroy: vi.fn().mockResolvedValue(undefined),
      } as never;
    });
    const w = mount(PdfPreviewPane, { props: { tab } });
    await flushPromises();
    // 传给 getDocument 的是副本（非原引用），且转移前字节数完整
    expect(receivedIsCopy).toBe(true);
    expect(receivedLenBeforeTransfer).toBe(3);
    // 标签上的 pdfData 保持完整（未被转移 detach）
    expect(tab.pdfData!.length).toBe(3);
    expect(w.text()).toContain("1 / 3");
    w.unmount();
  });

  it("同一标签卸载后重挂载仍能渲染（切标签切回不再误报内容为空）", async () => {
    const tab = reactive(makeTab());
    const w = mount(PdfPreviewPane, { props: { tab } });
    await flushPromises();
    expect(w.text()).toContain("1 / 3");
    w.unmount();

    const w2 = mount(PdfPreviewPane, { props: { tab } });
    await flushPromises();
    expect(w2.text()).toContain("1 / 3");
    expect(w2.text()).not.toContain("PDF 内容为空");
    w2.unmount();
  });

  it("提供 actionsTarget 时工具栏 Teleport 到头部容器", async () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const w = mount(PdfPreviewPane, {
      props: { tab: makeTab(), actionsTarget: target },
    });
    await flushPromises();
    expect(target.querySelector(".pdf-toolbar")).toBeTruthy();
    expect(w.find(".pdf-toolbar").exists()).toBe(false);
    w.unmount();
    target.remove();
  });

  it("内容区 Ctrl+滚轮缩放", async () => {
    // happy-dom 无 2D canvas：stub getContext 让 canvas host 正常渲染
    const origGetContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = vi.fn(() => ({})) as never;
    try {
      const w = mount(PdfPreviewPane, { props: { tab: makeTab() } });
      await flushPromises();
      expect(w.find(".pdf-toolbar-percent").text()).toBe("100%");
      const host = w.find(".pdf-canvas-host").element;
      const e = new WheelEvent("wheel", {
        deltaY: -100,
        bubbles: true,
        cancelable: true,
      });
      // happy-dom 的 WheelEvent 构造不接收 ctrlKey，手动定义
      Object.defineProperty(e, "ctrlKey", { value: true, configurable: true });
      host.dispatchEvent(e);
      await flushPromises();
      expect(w.find(".pdf-toolbar-percent").text()).toBe("125%");
      w.unmount();
    } finally {
      HTMLCanvasElement.prototype.getContext = origGetContext;
    }
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
