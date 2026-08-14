import { describe, expect, it } from "vitest";
import { TASK_MODES, taskMode } from "../tasks";

describe("TASK_MODES 任务模式清单", () => {
  it("仅包含执行/计划两种模式（目标已从任务模式移除）", () => {
    expect(TASK_MODES.map((m) => m.id)).toEqual(["execute", "plan"]);
  });

  it("每项都有中文 label 与非空图标", () => {
    for (const m of TASK_MODES) {
      expect(m.label.length).toBeGreaterThan(0);
      expect(m.icon.length).toBeGreaterThan(0);
    }
  });

  it("taskMode() 对未知 id 回退到执行模式", () => {
    expect(taskMode("goal").id).toBe("execute");
    expect(taskMode("execute").id).toBe("execute");
    expect(taskMode("plan").id).toBe("plan");
  });
});
