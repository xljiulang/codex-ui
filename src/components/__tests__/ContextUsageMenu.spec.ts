import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick, reactive } from "vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, setToast: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import ContextUsageMenu from "../ContextUsageMenu.vue";
import { setToast, type SessionTab } from "../../composables/useCodex";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";
import { ICON_SIGMA } from "../../lib/icons";

const mockedInvoke = vi.mocked(invoke);
const mockedSetToast = vi.mocked(setToast);

let wrapper: VueWrapper | null = null;

/** 会话标签 fixture：仅本组件关注的字段（threadId/model/effort/threadTokenUsage） */
function makeTab(): SessionTab {
  const tab = reactive(makeSessionTab("s1", "t1"));
  tab.model = "glm-4.7";
  tab.effort = "high";
  return tab;
}

const FULL_USAGE = {
  contextUsed: 5000,
  window: 10000,
  input: 12000,
  output: 34000,
  totalTokens: 46000,
  cachedInput: 8000,
  cacheWriteInput: 1000,
  reasoningOutput: 2000,
};

async function mountWith(usage: SessionTab["threadTokenUsage"]) {
  const tab = makeTab();
  tab.threadTokenUsage = usage;
  wrapper = mount(ContextUsageMenu, { props: { tab } });
  await nextTick();
  return wrapper;
}

describe("ContextUsageMenu 上下文用量圆环与悬浮菜单", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue({});
    mockedSetToast.mockClear();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  it("threadTokenUsage 为 null 时整组件不渲染", async () => {
    wrapper = await mountWith(null);
    expect(wrapper.find(".ctx-ring-anchor").exists()).toBe(false);
  });

  it("圆环渲染上下文占用百分比进度", async () => {
    wrapper = await mountWith({ contextUsed: 5000, window: 10000 });
    const ring = wrapper.find(".ctx-ring");
    expect(ring.exists()).toBe(true);
    expect(ring.find(".ctx-ring-pct").text()).toBe("50");
    const bar = ring.find(".ctx-ring-bar");
    expect(bar.exists()).toBe(true);
    expect(bar.attributes("stroke-dasharray")).toContain("31.4159");
    expect(ring.attributes("aria-label")).toContain("上下文已用");
  });

  it("环心数字一位数补 %，两位及以上纯数字", async () => {
    wrapper = await mountWith({ contextUsed: 500, window: 10000 });
    expect(wrapper.find(".ctx-ring-pct").text()).toBe("5%");
    wrapper!.unmount();

    wrapper = await mountWith({ contextUsed: 9900, window: 10000 });
    expect(wrapper.find(".ctx-ring-pct").text()).toBe("99");
  });

  it("悬停向上展开菜单：窗口占用、剩余与压缩胶囊", async () => {
    wrapper = await mountWith({ contextUsed: 5000, window: 10000 });
    expect(wrapper.find(".usage-menu").exists()).toBe(false);
    await wrapper.find(".ctx-ring-anchor").trigger("mouseenter");
    const menu = wrapper.find(".usage-menu");
    expect(menu.exists()).toBe(true);
    expect(menu.find(".usage-menu-title").text()).toBe("上下文与 Token");
    expect(menu.find(".usage-menu-capsule-pct").text()).toBe("50%");
    expect(menu.find(".usage-menu-line").text()).toContain("已用 5000 / 窗口 1万");
    expect(menu.find(".usage-menu-line").text()).toContain("剩余 5000");
    const compact = menu.find(".usage-menu-capsule-btn");
    expect(compact.exists()).toBe(true);
    expect(compact.find("svg").exists()).toBe(true);
    expect(compact.attributes("aria-label")).toBe("压缩上下文");
  });

  it("菜单展示会话累计与细分 token 行（缺字段整行隐藏）", async () => {
    wrapper = await mountWith(FULL_USAGE);
    await wrapper.find(".ctx-ring-anchor").trigger("mouseenter");
    const menu = wrapper.find(".usage-menu");
    const rows = menu.findAll(".usage-menu-row");
    expect(rows.map((r) => r.find(".usage-menu-label").text().trim())).toEqual([
      "输入",
      "输出",
      "合计",
      "缓存读取",
      "缓存写入",
      "推理输出",
    ]);
    expect(rows.map((r) => r.find(".usage-menu-value").text())).toEqual([
      "1.2万",
      "3.4万",
      "4.6万",
      "8000",
      "1000",
      "2000",
    ]);
    // 合计行带 Σ 求和图标（与输入/输出箭头行同风格图标）
    const totalRow = rows[2];
    expect(totalRow.find(".usage-menu-label svg").exists()).toBe(true);
    expect(totalRow.find(".usage-menu-label svg path").attributes("d")).toBe(
      ICON_SIGMA,
    );
    // 缓存/推理为缩进的子集行
    expect(rows[0].classes()).not.toContain("sub");
    expect(rows[3].classes()).toContain("sub");
  });

  it("窗口未知时不画进度，菜单提示窗口大小未知但仍展示累计", async () => {
    wrapper = await mountWith({ contextUsed: 3000, window: null, input: 12000 });
    expect(wrapper.find(".ctx-ring-pct").exists()).toBe(false);
    expect(wrapper.find(".ctx-ring-bar").exists()).toBe(false);
    await wrapper.find(".ctx-ring-anchor").trigger("mouseenter");
    const menu = wrapper.find(".usage-menu");
    expect(menu.find(".usage-menu-bar").exists()).toBe(false);
    expect(menu.text()).toContain("窗口大小未知");
    const rows = menu.findAll(".usage-menu-row");
    expect(rows.map((r) => r.find(".usage-menu-label").text().trim())).toEqual([
      "输入",
    ]);
  });

  it("鼠标移出后延迟关闭菜单", async () => {
    vi.useFakeTimers();
    try {
      wrapper = await mountWith({ contextUsed: 5000, window: 10000 });
      await wrapper.find(".ctx-ring-anchor").trigger("mouseenter");
      expect(wrapper.find(".usage-menu").exists()).toBe(true);
      await wrapper.find(".ctx-ring-anchor").trigger("mouseleave");
      expect(wrapper.find(".usage-menu").exists()).toBe(true);
      await vi.advanceTimersByTimeAsync(200);
      expect(wrapper.find(".usage-menu").exists()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("点击压缩按钮发起压缩并提示；压缩中禁用，完成后恢复", async () => {
    wrapper = await mountWith({ contextUsed: 5000, window: 10000 });
    await wrapper.find(".ctx-ring-anchor").trigger("mouseenter");
    let resolveCompact!: (v: unknown) => void;
    mockedInvoke.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveCompact = resolve;
        }),
    );
    const compactBtn = () => wrapper!.find(".usage-menu-capsule-btn");
    expect((compactBtn().element as HTMLButtonElement).disabled).toBe(false);
    await compactBtn().trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("codex_rpc", {
      method: "thread/compact/start",
      params: { threadId: "t1" },
    });
    expect((compactBtn().element as HTMLButtonElement).disabled).toBe(true);
    expect(compactBtn().attributes("aria-label")).toBe("正在压缩上下文");
    resolveCompact({});
    await flushPromises();
    expect((compactBtn().element as HTMLButtonElement).disabled).toBe(false);
    expect(mockedSetToast).toHaveBeenCalledWith("已开始压缩上下文");
  });

  it("压缩失败时 toast 错误", async () => {
    wrapper = await mountWith({ contextUsed: 5000, window: 10000 });
    await wrapper.find(".ctx-ring-anchor").trigger("mouseenter");
    mockedInvoke.mockRejectedValueOnce(new Error("压缩失败"));
    await wrapper.find(".usage-menu-capsule-btn").trigger("click");
    await flushPromises();
    expect(mockedSetToast).toHaveBeenCalledWith("压缩失败");
  });
});
