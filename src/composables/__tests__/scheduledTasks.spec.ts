import { describe, expect, it } from "vitest";
import { describeSchedule } from "../useCodex/scheduledTasks";

describe("describeSchedule cron 调度描述", () => {
  it("每 N 分钟", () => {
    expect(describeSchedule("0 */30 * * * *")).toBe("每 30 分钟");
    expect(describeSchedule("0 */5 * * * *")).toBe("每 5 分钟");
  });

  it("每小时", () => {
    expect(describeSchedule("0 0 * * * *")).toBe("每小时");
  });

  it("每天 / 每周", () => {
    expect(describeSchedule("0 0 9 * * *")).toBe("每天 09:00");
    expect(describeSchedule("0 30 21 * * *")).toBe("每天 21:30");
    expect(describeSchedule("0 0 9 * * 1")).toBe("每周一 09:00");
  });

  it("单次（带年份）显示具体时间", () => {
    expect(describeSchedule("0 0 9 20 1 * 2026")).toBe("单次：2026-01-20 09:00");
  });

  it("识别不了的模式回退原表达式", () => {
    expect(describeSchedule("0 0 9 * * MON")).toBe("0 0 9 * * MON");
    expect(describeSchedule("bad")).toBe("bad");
  });
});
