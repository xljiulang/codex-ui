import { describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

import { invoke } from "@tauri-apps/api/core";
import DiffWindowView from "../DiffWindowView.vue";
import type { DiffRow } from "../../lib/types";

const mockedInvoke = vi.mocked(invoke);

const PARAMS = {
  path: "D:\\repo\\a.txt",
  kind: "update",
  diff: "@@ -1,3 +1,3 @@\n a\n-b\n+X\n c",
  workspace_root: "D:/repo",
};

const ROWS: DiffRow[] = [
  { kind: "ctx", oldNo: 1, newNo: 1, text: "a" },
  { kind: "del", oldNo: 2, text: "b" },
  { kind: "sep" },
  { kind: "add", newNo: 2, text: "X" },
  { kind: "ctx", oldNo: 3, newNo: 3, text: "c" },
];

function mockOk(path = PARAMS.path, rows: DiffRow[] = ROWS) {
  mockedInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "take_diff_params") return { ...PARAMS, path };
    if (cmd === "build_diff_preview") return rows;
    return null;
  });
}

describe("DiffWindowView 独立窗口", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("取参数并渲染内联行与头部", async () => {
    mockOk();
    const wrapper = mount(DiffWindowView);
    await flushPromises();
    expect(wrapper.find(".diff-window-path").text()).toContain("a.txt");
    expect(wrapper.find(".change-kind").text()).toBe("修改");
    const rows = wrapper.findAll(".diff-row");
    expect(rows.map((r) => r.classes()[1])).toEqual([
      "ctx",
      "del",
      "sep",
      "add",
      "ctx",
    ]);
    expect(wrapper.find(".diff-sep-text").text()).toBe("旧 | 新");
    expect(rows[1].find(".diff-text").text()).toBe("b");
    expect(rows[3].find(".diff-text").text()).toBe("X");
  });

  it("解析失败回退统一 diff", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "take_diff_params") return PARAMS;
      if (cmd === "build_diff_preview") throw new Error("diff 格式异常");
      return null;
    });
    const wrapper = mount(DiffWindowView);
    await flushPromises();
    expect(wrapper.find(".diff-fallback-note").exists()).toBe(true);
    expect(wrapper.find(".diff-preview").exists()).toBe(true);
  });

  it("未取到参数时提示", async () => {
    mockedInvoke.mockResolvedValue(null);
    const wrapper = mount(DiffWindowView);
    await flushPromises();
    expect(wrapper.text()).toContain("未找到 diff 参数");
  });

  it("代码文件逐行语法高亮", async () => {
    mockOk(
      "D:\\repo\\sample.ts",
      [{ kind: "add", newNo: 1, text: "const answer: number = 42;" }],
    );
    const wrapper = mount(DiffWindowView);
    await flushPromises();
    expect(wrapper.find(".diff-text .hljs-keyword").exists()).toBe(true);
    expect(wrapper.find(".diff-text").text()).toContain("const");
  });

  it("未知扩展名不进行高亮", async () => {
    mockOk();
    const wrapper = mount(DiffWindowView);
    await flushPromises();
    expect(wrapper.find(".diff-text .hljs-keyword").exists()).toBe(false);
  });

  it("右键无自定义菜单且阻止默认（无刷新）", async () => {
    mockOk();
    const wrapper = mount(DiffWindowView, { attachTo: document.body });
    await flushPromises();
    const el = wrapper.find(".diff-text").element;
    const ev = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 50,
      clientY: 50,
    });
    el.dispatchEvent(ev);
    await flushPromises();
    expect(ev.defaultPrevented).toBe(true);
    expect(wrapper.find(".ctx-menu").exists()).toBe(false);
  });
});
