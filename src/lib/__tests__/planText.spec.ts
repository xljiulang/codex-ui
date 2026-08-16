import { describe, expect, it } from "vitest";
import { splitPlanTitle } from "../planText";

describe("splitPlanTitle 计划标题提取", () => {
  it("提取首个 # 标题为卡片标题，正文不含标题行", () => {
    const r = splitPlanTitle("# 修复方案\n1. 改代码\n2. 验证");
    expect(r.title).toBe("修复方案");
    expect(r.body).toBe("1. 改代码\n2. 验证");
  });

  it("支持 ## / ### 多级标题", () => {
    expect(splitPlanTitle("## 二级标题\n正文").title).toBe("二级标题");
    expect(splitPlanTitle("### 三级标题\n正文").title).toBe("三级标题");
  });

  it("前导空行不干扰标题提取", () => {
    const r = splitPlanTitle("\n\n# 方案\n正文");
    expect(r.title).toBe("方案");
    expect(r.body).toBe("正文");
  });

  it("正文保留标题行之后的其它标题", () => {
    const r = splitPlanTitle("# 方案\n## 阶段一\n- 步骤");
    expect(r.title).toBe("方案");
    expect(r.body).toBe("## 阶段一\n- 步骤");
  });

  it("无标题行时回退 计划，正文为原文", () => {
    const r = splitPlanTitle("直接是内容\n- 步骤");
    expect(r.title).toBe("计划");
    expect(r.body).toBe("直接是内容\n- 步骤");
  });

  it("空文本回退 计划", () => {
    expect(splitPlanTitle("")).toEqual({ title: "计划", body: "" });
  });
});
