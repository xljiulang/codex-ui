import { describe, expect, it, vi, beforeEach } from "vitest";
import { flushPromises } from "@vue/test-utils";
import { nextTick } from "vue";

vi.mock("../../composables/useCodex/settings", () => ({
  saveSettings: vi.fn(() => Promise.resolve()),
}));

import {
  __resetLastSessionForTest,
  flushLastSession,
  readLastSessionId,
  trackLastSession,
} from "../useCodex/lastSession";
import { saveSettings } from "../useCodex/settings";
import { store } from "../useCodex/store";
import { __resetTabsForTest, activateTab, tabs } from "../useTabs";
import { makeSessionTab } from "./useCodexTestHarness";

const mockedSave = vi.mocked(saveSettings);

trackLastSession();

describe("lastSession 最后活跃会话记录与退出落盘", () => {
  beforeEach(() => {
    __resetLastSessionForTest();
    __resetTabsForTest();
    store.settings.last_session_id = null;
    mockedSave.mockClear();
  });

  it("激活会话标签仅记录内存，不写配置文件", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activateTab("s1");
    await nextTick();
    await flushPromises();
    expect(mockedSave).not.toHaveBeenCalled();
  });

  it("flushLastSession 把内存记录落盘 last_session_id", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activateTab("s1");
    await nextTick();
    await flushLastSession();
    expect(mockedSave).toHaveBeenCalledTimes(1);
    expect(mockedSave).toHaveBeenCalledWith({ last_session_id: "t1" });
  });

  it("切到非会话标签不影响内存记录，落盘值仍为最后活跃会话", async () => {
    tabs.push(makeSessionTab("s1", "t1"));
    activateTab("s1");
    await nextTick();
    // 非会话标签仅 kind 参与判定，最小构造（FileEditorTab 完整字段与本用例无关）
    tabs.push({ id: "f1", kind: "file", title: "文件" } as never);
    activateTab("f1");
    await nextTick();
    await flushLastSession();
    expect(mockedSave).toHaveBeenCalledWith({ last_session_id: "t1" });
  });

  it("值未变化时重复 flush 跳过写盘", async () => {
    // 模拟真实 saveSettings 的合并行为（写 store.settings），供第二次 flush 判定
    mockedSave.mockImplementation(async (patch) => {
      Object.assign(store.settings, patch);
    });
    tabs.push(makeSessionTab("s1", "t1"));
    activateTab("s1");
    await nextTick();
    await flushLastSession();
    await flushLastSession();
    expect(mockedSave).toHaveBeenCalledTimes(1);
  });

  it("本次运行未激活过会话：flush 不落盘，保留已有持久化值", async () => {
    store.settings.last_session_id = "t-old";
    await flushLastSession();
    expect(mockedSave).not.toHaveBeenCalled();
  });

  it("readLastSessionId：读 store.settings.last_session_id，缺省空串", () => {
    expect(readLastSessionId()).toBe("");
    store.settings.last_session_id = "t9";
    expect(readLastSessionId()).toBe("t9");
  });
});
