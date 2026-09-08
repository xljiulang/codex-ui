import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const previewMock = vi.hoisted(() =>
  vi.fn(async (_data: ArrayBuffer) => undefined),
);
const initMock = vi.hoisted(() =>
  vi.fn((container: HTMLElement, _options: { width?: number }) => {
    const wrapper = document.createElement("div");
    wrapper.className = "pptx-preview-wrapper";
    container.appendChild(wrapper);
    const slide = document.createElement("div");
    slide.className = "pptx-preview-slide-wrapper";
    slide.textContent = "春天";
    wrapper.appendChild(slide);
    return { preview: previewMock };
  }),
);
vi.mock("pptx-preview", () => ({ init: initMock }));

import PptxPreviewPane from "../PptxPreviewPane.vue";
import {
  __resetEditorTabsForTest,
  type PreviewEditorTab,
} from "../../composables/useEditorTabs";
import { insertTab } from "../../composables/useTabs";
import { tooltipDirective } from "../../directives/tooltip";

const mockedInit = vi.mocked(initMock);
const root = "D:\\repo";

function previewTab(overrides: Partial<PreviewEditorTab> = {}): PreviewEditorTab {
  const tab = {
    kind: "preview",
    previewType: "pptx",
    id: 'preview:pptx:["D:\\\\repo","a.pptx"]',
    workspace: root,
    path: "a.pptx",
    title: "a.pptx",
    icon: "file",
    loading: false,
    error: "",
    imageUrl: "",
    pdfData: null,
    xlsxData: null,
    docxData: null,
    pptxData: new Uint8Array([80, 75]),
    xlsxSheetIndex: 0,
    pageCount: null,
    stale: false,
    ...overrides,
  } as unknown as PreviewEditorTab;
  insertTab(tab);
  return tab;
}

async function mountPreview(tab: PreviewEditorTab) {
  const wrapper = mount(PptxPreviewPane, {
    props: { tab },
    global: { directives: { tooltip: tooltipDirective } },
  });
  await flushPromises();
  await nextTick();
  return wrapper;
}

describe("PptxPreviewPane .pptx 版式预览", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    previewMock.mockClear();
    mockedInit.mockClear();
    __resetEditorTabsForTest();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("调用 pptx-preview 渲染字节到 sizer 并展示幻灯片容器", async () => {
    const tab = previewTab();
    wrapper = await mountPreview(tab);
    await vi.waitFor(
      () => {
        expect(mockedInit).toHaveBeenCalledTimes(1);
      },
      { timeout: 3000, interval: 20 },
    );
    expect(previewMock).toHaveBeenCalledTimes(1);
    const [container, options] = mockedInit.mock.calls[0];
    expect(container).toBe(wrapper!.find(".pptx-preview-sizer").element);
    expect((options as { width?: number }).width).toBe(960);
    const [data] = previewMock.mock.calls[0];
    expect(data).toBeInstanceOf(ArrayBuffer);
    expect(Array.from(new Uint8Array(data))).toEqual([80, 75]);
    expect(wrapper!.find(".pptx-preview-sizer").html()).toContain("春天");
    expect(wrapper!.text()).not.toContain("无法预览");
  });

  it("字节为空：展示错误态且不调用渲染", async () => {
    const tab = previewTab({ pptxData: null });
    wrapper = await mountPreview(tab);
    await flushPromises();
    expect(mockedInit).not.toHaveBeenCalled();
    expect(previewMock).not.toHaveBeenCalled();
    expect(wrapper!.text()).toContain("无法预览该演示文稿");
    expect(wrapper!.text()).toContain("演示文稿内容为空");
  });

  it("渲染失败：展示错误态", async () => {
    previewMock.mockRejectedValueOnce(new Error("bad zip"));
    const tab = previewTab();
    wrapper = await mountPreview(tab);
    await vi.waitFor(
      () => {
        expect(wrapper!.text()).toContain("无法预览该演示文稿");
      },
      { timeout: 3000, interval: 20 },
    );
    expect(wrapper!.text()).toContain("bad zip");
  });

  it("缩放工具栏：放大/缩小按步进调整百分比并在端点禁用", async () => {
    const tab = previewTab();
    wrapper = await mountPreview(tab);
    await vi.waitFor(
      () => {
        expect(mockedInit).toHaveBeenCalledTimes(1);
      },
      { timeout: 3000, interval: 20 },
    );
    expect(wrapper!.find(".pdf-toolbar-percent").text()).toBe("100%");

    await wrapper!.find('button[aria-label="放大"]').trigger("click");
    expect(wrapper!.find(".pdf-toolbar-percent").text()).toBe("125%");

    await wrapper!.find('button[aria-label="缩小"]').trigger("click");
    await wrapper!.find('button[aria-label="缩小"]').trigger("click");
    expect(wrapper!.find(".pdf-toolbar-percent").text()).toBe("80%");
  });
});
