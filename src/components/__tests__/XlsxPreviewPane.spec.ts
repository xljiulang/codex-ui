import { describe, expect, it } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import * as XLSX from "xlsx";
import XlsxPreviewPane from "../XlsxPreviewPane.vue";
import type { PreviewEditorTab } from "../../composables/useEditorTabs";
import { TabIcon, TabKind } from "../../lib/tabs";

function workbookBytes(wb: XLSX.WorkBook): Uint8Array {
  const out = XLSX.write(wb, { type: "array", bookType: "xlsx" });
  return new Uint8Array(out as ArrayBuffer);
}

function makeTab(xlsxData: Uint8Array | null): PreviewEditorTab {
  return {
    kind: TabKind.Preview,
    previewType: "xlsx",
    id: "preview-xlsx",
    workspace: "D:\\repo",
    path: "book.xlsx",
    title: "book.xlsx",
    icon: TabIcon.File,
    loading: false,
    error: "",
    imageUrl: "",
    pdfData: null,
    xlsxData,
    xlsxSheetIndex: 0,
    pageCount: null,
    stale: false,
  } as unknown as PreviewEditorTab;
}

function twoSheetBytes(): Uint8Array {
  const wb = XLSX.utils.book_new();
  XLSX.utils.book_append_sheet(
    wb,
    XLSX.utils.aoa_to_sheet([
      ["名称", "数量"],
      ["苹果", 3],
    ]),
    "数据",
  );
  XLSX.utils.book_append_sheet(
    wb,
    XLSX.utils.aoa_to_sheet([["仅一列"]]),
    "汇总",
  );
  return workbookBytes(wb);
}

describe("XlsxPreviewPane 表格预览", () => {
  it("解析字节并渲染：工作表按钮、列头、单元格文本与行列信息", async () => {
    const wrapper = mount(XlsxPreviewPane, {
      props: { tab: makeTab(twoSheetBytes()) },
    });
    await flushPromises();

    const buttons = wrapper.findAll(".xlsx-sheet-btn");
    expect(buttons).toHaveLength(2);
    expect(buttons[0].text()).toBe("数据");
    expect(buttons[1].text()).toBe("汇总");
    expect(wrapper.find(".xlsx-meta").text()).toBe("2 行 × 2 列");
    expect(wrapper.find(".xlsx-th").text()).toBe("A");
    expect(wrapper.text()).toContain("名称");
    expect(wrapper.text()).toContain("苹果");
    expect(wrapper.text()).toContain("3");
  });

  it("切换工作表：更新内容与行列信息，并写回标签序号", async () => {
    const tab = makeTab(twoSheetBytes());
    const wrapper = mount(XlsxPreviewPane, { props: { tab } });
    await flushPromises();

    await wrapper.findAll(".xlsx-sheet-btn")[1].trigger("click");
    await flushPromises();
    expect(wrapper.find(".xlsx-meta").text()).toBe("1 行 × 1 列");
    expect(wrapper.text()).toContain("仅一列");
    expect(tab.xlsxSheetIndex).toBe(1);
  });

  it("缩放：放大/缩小/100% 复位，行高与列宽随缩放变化", async () => {
    const wrapper = mount(XlsxPreviewPane, {
      props: { tab: makeTab(twoSheetBytes()) },
    });
    await flushPromises();
    expect(wrapper.find(".preview-zoom-percent").text()).toBe("100%");
    expect(wrapper.find(".xlsx-table").attributes("style")).toContain(
      "--xlsx-row-h: 28px",
    );

    const btns = wrapper.findAll(".preview-zoom-btn");
    await btns[1].trigger("click"); // ＋ → 125%
    await flushPromises();
    expect(wrapper.find(".preview-zoom-percent").text()).toBe("125%");
    expect(wrapper.find(".xlsx-table").attributes("style")).toContain(
      "--xlsx-row-h: 35px",
    );

    await btns[0].trigger("click"); // − → 100%
    await flushPromises();
    expect(wrapper.find(".preview-zoom-percent").text()).toBe("100%");

    await btns[1].trigger("click"); // ＋ → 125%
    await flushPromises();
    await btns[2].trigger("click"); // 100% 复位
    await flushPromises();
    expect(wrapper.find(".preview-zoom-percent").text()).toBe("100%");
    expect(wrapper.find(".xlsx-table").attributes("style")).toContain(
      "--xlsx-row-h: 28px",
    );
  });

  it("外部替换 xlsxData（自动刷新）：重解析并重渲染", async () => {
    const wrapper = mount(XlsxPreviewPane, {
      props: { tab: makeTab(twoSheetBytes()) },
    });
    await flushPromises();
    expect(wrapper.text()).toContain("苹果");

    const wb = XLSX.utils.book_new();
    XLSX.utils.book_append_sheet(
      wb,
      XLSX.utils.aoa_to_sheet([["更新后内容"]]),
      "数据",
    );
    await wrapper.setProps({ tab: makeTab(workbookBytes(wb)) });
    await flushPromises();
    expect(wrapper.text()).toContain("更新后内容");
    expect(wrapper.text()).not.toContain("苹果");
    expect(wrapper.find(".xlsx-meta").text()).toBe("1 行 × 1 列");
  });

  it("非法字节：展示解析错误态", async () => {
    const wrapper = mount(XlsxPreviewPane, {
      props: { tab: makeTab(new Uint8Array([1, 2, 3, 4])) },
    });
    await flushPromises();
    expect(wrapper.find(".preview-error").text()).toContain("无法预览该表格");
  });

  it("空字节：展示内容为空错误态", async () => {
    const wrapper = mount(XlsxPreviewPane, {
      props: { tab: makeTab(null) },
    });
    await flushPromises();
    expect(wrapper.find(".preview-error").text()).toContain("表格内容为空");
  });
});
