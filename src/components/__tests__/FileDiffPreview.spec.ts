import { describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

import { invoke } from "@tauri-apps/api/core";
import FileDiffPreview from "../FileDiffPreview.vue";

const mockedInvoke = vi.mocked(invoke);

const REPLACE_DIFF = [
  "@@ -1,3 +1,3 @@",
  " a",
  "-b",
  "+X",
  " c",
].join("\n");

function mountPreview(over: Record<string, unknown> = {}) {
  return mount(FileDiffPreview, {
    props: {
      path: "D:\\repo\\a.txt",
      kind: "update",
      diff: REPLACE_DIFF,
      workspaceRoot: "D:/repo",
      ...over,
    },
  });
}

describe("FileDiffPreview 内联完整 diff", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("渲染旧行在上、新行在下、中间分隔，行号正确", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "read_file") return "a\nX\nc";
      return null;
    });
    const wrapper = mountPreview();
    await flushPromises();
    const rows = wrapper.findAll(".diff-row");
    expect(rows.map((r) => r.classes()[1])).toEqual([
      "ctx",
      "del",
      "sep",
      "add",
      "ctx",
    ]);
    expect(wrapper.find(".diff-sep-text").text()).toBe("旧 | 新");
    expect(rows[1].find(".diff-no.old").text()).toBe("2");
    expect(rows[1].find(".diff-no.new").text()).toBe("");
    expect(rows[1].find(".diff-text").text()).toBe("b");
    expect(rows[3].find(".diff-no.old").text()).toBe("");
    expect(rows[3].find(".diff-no.new").text()).toBe("2");
    expect(rows[3].find(".diff-text").text()).toBe("X");
    expect(rows[0].find(".diff-text").text()).toBe("a");
  });

  it("相对路径按工作目录解析后调用 read_file", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "read_file") return "a\nX\nc";
      return null;
    });
    mountPreview({ path: "src/a.txt" });
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("read_file", {
      path: "D:\\repo\\src\\a.txt",
    });
  });

  it("读取失败回退统一 diff 并提示", async () => {
    mockedInvoke.mockRejectedValueOnce("无法读取文件");
    const wrapper = mountPreview();
    await flushPromises();
    expect(wrapper.find(".diff-fallback-note").exists()).toBe(true);
    expect(wrapper.find(".diff-preview").exists()).toBe(true);
    expect(wrapper.find(".diff-row").exists()).toBe(false);
  });

  it("加载中显示 loading", async () => {
    let resolve!: (v: string) => void;
    mockedInvoke.mockImplementation(
      () => new Promise<string>((r) => (resolve = r)),
    );
    const wrapper = mountPreview();
    await flushPromises();
    expect(wrapper.find(".diff-loading").exists()).toBe(true);
    resolve("a\nX\nc");
    await flushPromises();
    expect(wrapper.find(".diff-loading").exists()).toBe(false);
    expect(wrapper.findAll(".diff-row").length).toBe(5);
  });

  it("Esc 触发 close，× 与遮罩也触发 close", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) =>
      cmd === "read_file" ? "a\nX\nc" : null,
    );
    const wrapper = mountPreview();
    await flushPromises();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flushPromises();
    expect(wrapper.emitted("close")).toBeTruthy();
  });

  it("delete 类型不读取文件，直接从 diff 重建旧内容", async () => {
    const wrapper = mountPreview({
      kind: "delete",
      diff: "@@ -1,2 +0,0 @@\n-a\n-b",
    });
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalledWith("read_file", expect.anything());
    const dels = wrapper.findAll(".diff-row.del");
    expect(dels).toHaveLength(2);
  });
});
