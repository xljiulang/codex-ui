import { describe, expect, it } from "vitest";
import {
  isThreadItemType,
  isUserInput,
  type AgentMessageItem,
  type CommandExecutionItem,
  type ThreadItem,
} from "../types";

describe("isUserInput", () => {
  it("识别合法内容项", () => {
    expect(isUserInput({ type: "text", text: "hi", text_elements: [] })).toBe(
      true,
    );
    expect(
      isUserInput({ type: "mention", name: "a.cs", path: "D:/a.cs" }),
    ).toBe(true);
    expect(
      isUserInput({ type: "skill", name: "s", path: "C:/x/SKILL.md" }),
    ).toBe(true);
    expect(isUserInput({ type: "localImage", path: "D:/p.png" })).toBe(true);
  });

  it("拒绝缺失字段/未知类型/非对象", () => {
    expect(isUserInput({ type: "text" })).toBe(false);
    expect(isUserInput({ type: "mention", name: "a" })).toBe(false);
    expect(isUserInput({ type: "weird" })).toBe(false);
    expect(isUserInput(null)).toBe(false);
    expect(isUserInput("x")).toBe(false);
    expect(isUserInput(undefined)).toBe(false);
  });
});

describe("isThreadItemType", () => {
  it("按 type 收窄到指定消息类型", () => {
    const agent = { id: "a", type: "agentMessage", text: "hi" };
    const cmd = {
      id: "c",
      type: "commandExecution",
      command: "ls",
      status: "in_progress",
    };
    expect(
      isThreadItemType<AgentMessageItem>(agent as ThreadItem, "agentMessage"),
    ).toBe(true);
    expect(
      isThreadItemType<CommandExecutionItem>(
        cmd as ThreadItem,
        "commandExecution",
      ),
    ).toBe(true);
    expect(
      isThreadItemType<CommandExecutionItem>(
        agent as ThreadItem,
        "commandExecution",
      ),
    ).toBe(false);
  });
});
