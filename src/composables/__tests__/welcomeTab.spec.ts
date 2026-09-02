import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  closeTab,
  openWelcomeTab,
  WELCOME_TAB_ID,
  tabs,
  activeTabId,
} from "../useEditorTabs";
import { __resetTabsForTest } from "../useTabs";
import { TabKind } from "../../lib/tabs";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "" }),
}));

describe("欢迎介绍标签生命周期", () => {
  beforeEach(() => {
    __resetTabsForTest();
  });

  it("首次打开创建唯一欢迎标签并激活", () => {
    openWelcomeTab();
    expect(tabs).toHaveLength(1);
    expect(tabs[0]!.id).toBe(WELCOME_TAB_ID);
    expect(tabs[0]!.kind).toBe(TabKind.Welcome);
    expect(tabs[0]!.title).toBe("欢迎");
    expect(tabs[0]!.workspace).toBeNull();
    expect(activeTabId.value).toBe(WELCOME_TAB_ID);
  });

  it("重复调用幂等：不产生第二个欢迎标签", () => {
    openWelcomeTab();
    openWelcomeTab();
    expect(tabs.filter((t) => t.kind === TabKind.Welcome)).toHaveLength(1);
    expect(activeTabId.value).toBe(WELCOME_TAB_ID);
  });

  it("关闭欢迎标签后回到空状态，可再次打开", async () => {
    openWelcomeTab();
    await closeTab(WELCOME_TAB_ID);
    expect(tabs).toHaveLength(0);
    expect(activeTabId.value).toBe("");
    openWelcomeTab();
    expect(tabs).toHaveLength(1);
    expect(activeTabId.value).toBe(WELCOME_TAB_ID);
  });
});
