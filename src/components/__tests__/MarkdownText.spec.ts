import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import MarkdownText from "../MarkdownText.vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

import { invoke } from "@tauri-apps/api/core";
import { store } from "../../composables/useCodex";

const mockedInvoke = vi.mocked(invoke);
const origWorker = globalThis.Worker;

describe("MarkdownText 流式渲染与代码高亮", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  beforeEach(() => {
    mockedInvoke.mockClear();
    store.workspace = "D:/repo";
    // 测试环境固定走同步回退解析，避免 happy-dom Worker 挂起
    (globalThis as Record<string, unknown>).Worker = undefined;
  });

  afterEach(() => {
    (globalThis as Record<string, unknown>).Worker = origWorker;
  });

  it("静态文本立即渲染 Markdown", async () => {
    const wrapper = mount(MarkdownText, {
      props: { text: "hello **world**" },
    });
    await flushPromises();
    expect(wrapper.find(".md strong").text()).toBe("world");
  });

  it("流式期间最多每 80ms 刷新一次，结束时立即刷净", async () => {
    vi.useFakeTimers();
    const wrapper = mount(MarkdownText, {
      props: { text: "abc", streaming: true },
    });
    await flushPromises();

    await wrapper.setProps({ text: "abc def" });
    await flushPromises();
    expect(wrapper.text()).not.toContain("def");

    vi.advanceTimersByTime(80);
    await flushPromises();
    expect(wrapper.text()).toContain("def");

    await wrapper.setProps({ text: "abc def ghi", streaming: false });
    await flushPromises();
    expect(wrapper.text()).toContain("ghi");
  });

  it("带语言标记的代码块注入高亮与语言徽标", async () => {
    const wrapper = mount(MarkdownText, {
      props: { text: "```js\nconst x = 1;\n```" },
    });
    await flushPromises();
    const code = wrapper.find("pre code.language-js");
    expect(code.exists()).toBe(true);
    expect(code.classes()).toContain("hljs");
    expect(wrapper.find(".code-lang").text()).toBe("js");
    expect(wrapper.find(".code-copy-btn").exists()).toBe(true);
  });

  it("流式期间代码块不高亮，结束后一次性高亮", async () => {
    const wrapper = mount(MarkdownText, {
      props: { text: "```js\nconst x = 1;\n```", streaming: true },
    });
    await flushPromises();
    const code = wrapper.find("pre code.language-js");
    expect(code.exists()).toBe(true);
    // 流式中跳过语法高亮，避免每 tick 全量重高亮；徽标仍即时渲染
    expect(code.classes()).not.toContain("hljs");
    expect(wrapper.find(".code-lang").text()).toBe("js");
    await wrapper.setProps({ streaming: false });
    await flushPromises();
    expect(wrapper.find("pre code.language-js").classes()).toContain("hljs");
  });

  it("无语言标记的代码块显示 code 徽标且不强制高亮", async () => {
    const wrapper = mount(MarkdownText, {
      props: { text: "```\nplain\n```" },
    });
    await flushPromises();
    expect(wrapper.find(".code-lang").text()).toBe("code");
    expect(wrapper.find("pre code").classes()).not.toContain("hljs");
  });

  it("点击网页链接调用 open_url（外置浏览器）", async () => {
    const wrapper = mount(MarkdownText, {
      props: { text: "[示例](https://example.com/path)" },
    });
    await flushPromises();
    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("open_url", {
      url: "https://example.com/path",
    });
  });

  it("点击 file:/// 本地文本链接在应用内打开（工作区外以父目录为根）", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: { text: "[本地](file:///D:/a%20b.txt)" },
    });
    await flushPromises();
    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: "D:\\",
      path: "a b.txt",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_read", {
      workspace: "D:\\",
      path: "a b.txt",
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "reveal_path",
      expect.anything(),
    );
  });

  it("点击相对文本链接按工作目录解析后在应用内打开", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: { text: "[源码](src/a.ts)" },
    });
    await flushPromises();
    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: "D:/repo",
      path: "D:\\repo\\src\\a.ts",
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "reveal_path",
      expect.anything(),
    );
  });

  it("本地链接为二进制文件时降级调用 reveal_path", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(false);
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: { text: "[二进制](D:/repo/data.bin)" },
    });
    await flushPromises();
    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "D:\\repo\\data.bin",
    });
  });

  it("本地链接探测失败（目录/缺失）时降级调用 reveal_path", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") {
        return Promise.reject(new Error("不是文件"));
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: { text: "[目录](D:/repo/src)" },
    });
    await flushPromises();
    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("reveal_path", {
      path: "D:\\repo\\src",
    });
  });

  it("mailto 链接点击不调用任何命令", async () => {
    const wrapper = mount(MarkdownText, {
      props: { text: "[邮件](mailto:a@b.c)" },
    });
    await flushPromises();
    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("流式追加只替换尾部节点，保留前面块的 DOM 引用", async () => {
    const wrapper = mount(MarkdownText, {
      props: {
        text: "第一段\n\nhello **world**",
        streaming: true,
      },
    });
    await flushPromises();
    const firstP = wrapper.find(".md > p").element;
    expect(wrapper.text()).toContain("world");

    await wrapper.setProps({
      text: "第一段\n\nhello **world** 更多",
      streaming: false,
    });
    await flushPromises();
    expect(wrapper.find(".md > p").element).toBe(firstP); // 未整段替换
    expect(wrapper.text()).toContain("更多");
    expect(wrapper.find(".md strong").text()).toBe("world");
  });

  it("流式同段落链接后追加内容（结构变化）后链接仍可点击", async () => {
    vi.useFakeTimers();
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: { text: "[文件](D:/repo/a.ts)", streaming: true },
    });
    await flushPromises();
    vi.advanceTimersByTime(80);
    await flushPromises();
    await wrapper.setProps({
      text: "[文件](D:/repo/a.ts) 后续内容",
      streaming: true,
    });
    vi.advanceTimersByTime(80);
    await flushPromises();
    await wrapper.setProps({ streaming: false });
    await flushPromises();

    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith(
      "session_fs_probe_text",
      expect.anything(),
    );
  });

  it("助手回复的盘符路径链接（D:/…）点击在应用内打开，工作区外以父目录为根", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: {
        text: "收到文件路径链接：[index.html](D:/codex/codex-ui/index.html)",
      },
    });
    await flushPromises();
    await wrapper.find("a").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: "D:\\codex\\codex-ui",
      path: "index.html",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_read", {
      workspace: "D:\\codex\\codex-ui",
      path: "index.html",
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "reveal_path",
      expect.anything(),
    );
  });

  it("代码块闭合等结构重排时回退整段替换", async () => {
    const wrapper = mount(MarkdownText, {
      props: { text: "```js\nconst x = 1;", streaming: true },
    });
    await flushPromises();
    await wrapper.setProps({
      text: "```js\nconst x = 1;\n```",
      streaming: false,
    });
    await flushPromises();
    const code = wrapper.find("pre code.language-js");
    expect(code.exists()).toBe(true);
    expect(code.classes()).toContain("hljs");
  });

  it("反斜杠盘符路径链接保留 href 并在应用内打开（marked 编码为 %5C）", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: { text: "[计划.md](D:\\codex\\codex-ui\\docs\\计划.md)" },
    });
    await flushPromises();
    const a = wrapper.find("a");
    expect(a.attributes("href")).toContain("D:");
    await a.trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: "D:\\codex\\codex-ui\\docs",
      path: "计划.md",
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "reveal_path",
      expect.anything(),
    );
  });

  it("正斜杠盘符路径文本链接点击在应用内打开", async () => {
    mockedInvoke.mockImplementation((cmd) => {
      if (cmd === "session_fs_probe_text") return Promise.resolve(true);
      if (cmd === "session_fs_read") {
        return Promise.resolve({ content: "hello", validUtf8: true, byteSize: 5 });
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(MarkdownText, {
      props: { text: "[a.md](D:/codex/a.md)" },
    });
    await flushPromises();
    const a = wrapper.find("a");
    expect(a.attributes("href")).toBe("D:/codex/a.md");
    await a.trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("session_fs_probe_text", {
      workspace: "D:\\codex",
      path: "a.md",
    });
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "reveal_path",
      expect.anything(),
    );
  });
});
