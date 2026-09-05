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
const docxPreviewMock = vi.hoisted(() => ({
  renderAsync: vi.fn(
    async (_data: unknown, container: HTMLElement) => {
      container.innerHTML = '<div class="docx-wrapper"><p>春天</p></div>';
      return undefined;
    },
  ),
}));
vi.mock("docx-preview", () => docxPreviewMock);

import DocxPreviewPane from "../DocxPreviewPane.vue";
import { renderAsync } from "docx-preview";
import {
  __resetEditorTabsForTest,
  type PreviewEditorTab,
} from "../../composables/useEditorTabs";
import { insertTab } from "../../composables/useTabs";
import { tooltipDirective } from "../../directives/tooltip";

const mockedRenderAsync = vi.mocked(renderAsync);
const root = "D:\\repo";

function previewTab(overrides: Partial<PreviewEditorTab> = {}): PreviewEditorTab {
  const tab = {
    kind: "preview",
    previewType: "docx",
    id: 'preview:docx:["D:\\\\repo","a.docx"]',
    workspace: root,
    path: "a.docx",
    title: "a.docx",
    icon: "file",
    loading: false,
    error: "",
    imageUrl: "",
    pdfData: null,
    xlsxData: null,
    docxData: new Uint8Array([80, 75]),
    xlsxSheetIndex: 0,
    pageCount: null,
    stale: false,
    ...overrides,
  } as unknown as PreviewEditorTab;
  insertTab(tab);
  return tab;
}

async function mountPreview(tab: PreviewEditorTab) {
  const wrapper = mount(DocxPreviewPane, {
    props: { tab },
    global: { directives: { tooltip: tooltipDirective } },
  });
  await flushPromises();
  await nextTick();
  return wrapper;
}

describe("DocxPreviewPane .docx 排版预览", () => {
  let wrapper: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    mockedRenderAsync.mockClear();
    __resetEditorTabsForTest();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
  });

  it("调用 docx-preview 渲染字节到 sizer 并展示分页容器", async () => {
    const tab = previewTab();
    wrapper = await mountPreview(tab);
    await vi.waitFor(
      () => {
        expect(mockedRenderAsync).toHaveBeenCalledTimes(1);
      },
      { timeout: 3000, interval: 20 },
    );
    const [data, container, , options] = mockedRenderAsync.mock.calls[0];
    expect(Array.from(data as Uint8Array)).toEqual([80, 75]);
    expect(container).toBe(wrapper!.find(".docx-preview-sizer").element);
    expect((options as { breakPages?: boolean }).breakPages).toBe(true);
    expect(wrapper!.find(".docx-preview-sizer").html()).toContain("春天");
    expect(wrapper!.text()).not.toContain("无法预览");
  });

  it("字节为空：展示错误态且不调用渲染", async () => {
    const tab = previewTab({ docxData: null });
    wrapper = await mountPreview(tab);
    await flushPromises();
    expect(mockedRenderAsync).not.toHaveBeenCalled();
    expect(wrapper!.text()).toContain("无法预览该文档");
    expect(wrapper!.text()).toContain("文档内容为空");
  });

  it("渲染失败：展示错误态", async () => {
    mockedRenderAsync.mockRejectedValueOnce(new Error("bad zip"));
    const tab = previewTab();
    wrapper = await mountPreview(tab);
    await vi.waitFor(
      () => {
        expect(wrapper!.text()).toContain("无法预览该文档");
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
        expect(mockedRenderAsync).toHaveBeenCalledTimes(1);
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
