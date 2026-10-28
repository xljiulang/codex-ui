import { beforeEach, describe, expect, it } from "vitest";
import {
  __resetSubagentThreadsForTest,
  isSubagentThread,
  registerSubagentThread,
  subagentParentThread,
} from "../useCodex/subagentThreads";

describe("子代理线程登记", () => {
  beforeEach(() => {
    __resetSubagentThreadsForTest();
  });

  it("按 source 或 parentThreadId 识别并保留父线程", () => {
    expect(
      registerSubagentThread({
        id: "by-source",
        source: "subagent",
        parentThreadId: "parent-a",
      }),
    ).toBe("parent-a");
    expect(
      registerSubagentThread({
        id: "by-parent",
        source: "appServer",
        parentThreadId: "parent-b",
      }),
    ).toBe("parent-b");
    expect(registerSubagentThread({ id: "normal", source: "appServer" })).toBe(
      null,
    );
    expect(isSubagentThread("by-source")).toBe(true);
    expect(subagentParentThread("by-parent")).toBe("parent-b");
    expect(isSubagentThread("normal")).toBe(false);
  });

  it("登记表达到容量上限后清空旧映射，避免无界增长", () => {
    for (let index = 0; index < 1024; index++) {
      registerSubagentThread({
        id: `child-${index}`,
        source: "subagent",
        parentThreadId: "parent",
      });
    }
    registerSubagentThread({
      id: "overflow",
      source: "subagent",
      parentThreadId: "parent",
    });

    expect(isSubagentThread("child-0")).toBe(false);
    expect(isSubagentThread("overflow")).toBe(true);
  });

  it("嵌套子代理解析到最上层主会话", () => {
    registerSubagentThread({
      id: "child",
      source: "subagent",
      parentThreadId: "root",
    });
    registerSubagentThread({
      id: "grandchild",
      source: "subagent",
      parentThreadId: "child",
    });
    expect(subagentParentThread("grandchild")).toBe("root");
  });

  it("后续元数据缺少父线程时保留已登记的父子关系", () => {
    registerSubagentThread({
      id: "child",
      source: "subagent",
      parentThreadId: "parent",
    });
    expect(
      registerSubagentThread({
        id: "child",
        source: "subagent",
      }),
    ).toBe("parent");
    expect(subagentParentThread("child")).toBe("parent");
  });
});
