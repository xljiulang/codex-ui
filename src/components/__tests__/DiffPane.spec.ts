import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { reactive } from "vue";
import DiffPane from "../DiffPane.vue";
import type { DiffEditorTab } from "../../composables/useEditorTabs";
import { DIFF_MAX_RENDER_ROWS } from "../../lib/diffView";

vi.mock("../../lib/clipboard", () => ({ copyText: vi.fn() }));
import { copyText } from "../../lib/clipboard";

function makeTab(over: Partial<DiffEditorTab> = {}): DiffEditorTab {
  return reactive({
    kind: "diff",
    id: "diff:1",
    path: "a.txt",
    changeKind: "modify",
    workspaceRoot: "D:\\repo",
    title: "a.txt",
    loading: false,
    error: "",
    rows: [
      { kind: "ctx", oldNo: 1, newNo: 1, text: "keep1" },
      { kind: "del", oldNo: 2, text: "old" },
      { kind: "sep" },
      { kind: "add", newNo: 2, text: "new" },
      { kind: "ctx", oldNo: 3, newNo: 3, text: "keep2" },
    ],
    fallback: "",
    brief: false,
    ...over,
  }) as unknown as DiffEditorTab;
}

describe("DiffPane 完整/简要切换", () => {
  it("默认完整模式渲染全部行（含上下文）", () => {
    const wrapper = mount(DiffPane, { props: { tab: makeTab() } });
    expect(wrapper.findAll(".diff-row")).toHaveLength(5);
    expect(wrapper.find(".diff-row.ctx").exists()).toBe(true);
    expect(wrapper.find(".diff-row.del").exists()).toBe(true);
    expect(wrapper.find(".diff-row.sep").exists()).toBe(true);
    expect(wrapper.find(".diff-row.add").exists()).toBe(true);
    wrapper.unmount();
  });

  it("点击切换进入简要模式：隐藏 ctx，只保留变更行与分隔", async () => {
    const tab = makeTab();
    const wrapper = mount(DiffPane, { props: { tab } });
    const btn = wrapper.find(".diff-pane-actions button");
    expect(btn.attributes("aria-label")).toBe("简要显示");

    await btn.trigger("click");
    expect(tab.brief).toBe(true);

    const rows = wrapper.findAll(".diff-row");
    expect(rows).toHaveLength(3);
    expect(wrapper.find(".diff-row.ctx").exists()).toBe(false);
    expect(rows.map((r) => r.classes())).toEqual(
      expect.arrayContaining([
        ["diff-row", "del"],
        ["diff-row", "sep"],
        ["diff-row", "add"],
      ]),
    );
    expect(btn.attributes("aria-label")).toBe("完整显示");
    wrapper.unmount();
  });

  it("再次点击恢复完整模式", async () => {
    const tab = makeTab();
    const wrapper = mount(DiffPane, { props: { tab } });
    const btn = wrapper.find(".diff-pane-actions button");
    await btn.trigger("click");
    expect(tab.brief).toBe(true);
    await btn.trigger("click");
    expect(tab.brief).toBe(false);
    expect(wrapper.findAll(".diff-row")).toHaveLength(5);
    wrapper.unmount();
  });

  it("加载中/出错/无行时不渲染切换按钮", () => {
    const cases: Partial<DiffEditorTab>[] = [
      { loading: true },
      { error: "boom" },
      { rows: [], fallback: "raw diff" },
    ];
    for (const over of cases) {
      const wrapper = mount(DiffPane, { props: { tab: makeTab(over) } });
      expect(wrapper.find(".diff-pane-actions button").exists()).toBe(false);
      wrapper.unmount();
    }
  });
});

describe("DiffPane 大文件保护", () => {
  it("压缩成一行的大 JSON：整份不高亮，按纯文本渲染", () => {
    const long = "{".repeat(5_000);
    const tab = makeTab({
      path: "big.json",
      rows: [
        { kind: "del", oldNo: 1, text: long },
        { kind: "sep" },
        { kind: "add", newNo: 1, text: `${long}x` },
      ],
    });
    const wrapper = mount(DiffPane, { props: { tab } });
    expect(wrapper.findAll(".diff-row")).toHaveLength(3);
    // 高亮启用时会渲染 <span v-html>，此处应无内部 span（纯文本直出）
    expect(wrapper.find(".diff-text > span").exists()).toBe(false);
    expect(wrapper.find(".diff-text").text()).toBe(long);
    wrapper.unmount();
  });

  it("普通大小的 JSON diff 仍逐行高亮", () => {
    const tab = makeTab({
      path: "a.json",
      rows: [
        { kind: "del", oldNo: 1, text: '{"a": 1}' },
        { kind: "sep" },
        { kind: "add", newNo: 1, text: '{"a": 2}' },
      ],
    });
    const wrapper = mount(DiffPane, { props: { tab } });
    expect(wrapper.find(".diff-text > span").exists()).toBe(true);
    wrapper.unmount();
  });

  // 该用例一次挂 1 万个 DOM 行（正是要验证的上限），并行跑全量时给足超时
  it("行数超上限时截断渲染并提示省略行数", { timeout: 20_000 }, () => {
    const rows = Array.from({ length: DIFF_MAX_RENDER_ROWS + 3 }, (_, i) => ({
      kind: "ctx" as const,
      oldNo: i + 1,
      newNo: i + 1,
      text: "x",
    }));
    const tab = makeTab({ rows });
    const wrapper = mount(DiffPane, { props: { tab } });
    expect(wrapper.findAll(".diff-row")).toHaveLength(DIFF_MAX_RENDER_ROWS);
    expect(wrapper.text()).toContain("差异过大");
    expect(wrapper.text()).toContain("已省略 3 行");
    wrapper.unmount();
  });

  it("简要模式提示折叠的未变更行数", () => {
    const tab = makeTab({ brief: true });
    const wrapper = mount(DiffPane, { props: { tab } });
    expect(wrapper.findAll(".diff-row")).toHaveLength(3);
    expect(wrapper.text()).toContain("已折叠未变更的 2 行");
    wrapper.unmount();
  });
});

describe("DiffPane 右键复制菜单", () => {
  beforeEach(() => {
    vi.mocked(copyText).mockClear();
  });

  it("右键行弹出自定义菜单，点击复制该行纯文本（不含行号）", async () => {
    const wrapper = mount(DiffPane, { props: { tab: makeTab() } });
    const delRow = wrapper.find(".diff-row.del");
    await delRow.trigger("contextmenu", { clientX: 100, clientY: 120 });

    const menu = wrapper.find(".ctx-menu");
    expect(menu.exists()).toBe(true);
    const items = menu.findAll(".ctx-menu-item");
    expect(items).toHaveLength(1);
    expect(items[0].text()).toBe("复制");

    await items[0].trigger("click");
    expect(copyText).toHaveBeenCalledWith("old");
    wrapper.unmount();
  });

  it("存在选区时优先复制选区文本", async () => {
    const selection = {
      isCollapsed: false,
      toString: () => "selected diff",
    } as unknown as Selection;
    const spy = vi.spyOn(window, "getSelection").mockReturnValue(selection);
    const wrapper = mount(DiffPane, { props: { tab: makeTab() } });
    await wrapper
      .find(".diff-pane-body")
      .trigger("contextmenu", { clientX: 50, clientY: 50 });

    const items = wrapper.findAll(".ctx-menu-item");
    expect(items).toHaveLength(1);
    await items[0].trigger("click");
    expect(copyText).toHaveBeenCalledWith("selected diff");
    spy.mockRestore();
    wrapper.unmount();
  });
});
