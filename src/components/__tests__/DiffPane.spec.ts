import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { reactive } from "vue";
import DiffPane from "../DiffPane.vue";
import type { DiffEditorTab } from "../../composables/useEditorTabs";

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
    const btn = wrapper.find(".diff-window-actions button");
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
    const btn = wrapper.find(".diff-window-actions button");
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
      expect(wrapper.find(".diff-window-actions button").exists()).toBe(false);
      wrapper.unmount();
    }
  });
});
