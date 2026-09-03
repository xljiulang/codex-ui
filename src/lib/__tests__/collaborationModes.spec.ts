import { describe, expect, it } from "vitest";
import { COLLABORATION_MODES, collaborationMode } from "../collaborationModes";

describe("COLLABORATION_MODES 协作模式清单", () => {
  it("仅包含默认/计划两种模式（目标已从协作模式移除）", () => {
    expect(COLLABORATION_MODES.map((m) => m.id)).toEqual(["default", "plan"]);
  });

  it("每项都有中文 label 与非空图标", () => {
    for (const m of COLLABORATION_MODES) {
      expect(m.label.length).toBeGreaterThan(0);
      expect(m.icon.length).toBeGreaterThan(0);
    }
  });

  it("collaborationMode() 对未知 id 回退到默认模式（default）", () => {
    expect(collaborationMode("goal").id).toBe("default");
    expect(collaborationMode("default").id).toBe("default");
    expect(collaborationMode("plan").id).toBe("plan");
  });
});
