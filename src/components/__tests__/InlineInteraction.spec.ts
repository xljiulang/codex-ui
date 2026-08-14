import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { enableAutoUnmount, mount } from "@vue/test-utils";

// 每个用例结束后卸载组件，避免 window keydown 监听器跨用例累积
enableAutoUnmount(afterEach);

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, respondInteraction: vi.fn() };
});

import InlineInteraction from "../InlineInteraction.vue";
import { respondInteraction, store } from "../../composables/useCodex";

const mockedRespond = vi.mocked(respondInteraction);

describe("requestUserInput 响应格式", () => {
  beforeEach(() => {
    store.interactions = [];
    mockedRespond.mockClear();
  });

  it("按问题 id 返回 { answers: { qid: { answers: [选择] } } }", async () => {
    store.interactions.push({
      requestId: 42,
      method: "item/tool/requestUserInput",
      params: {
        questions: [
          {
            id: "q1",
            header: "选择项目",
            question: "要处理哪个项目？",
            isOther: false,
            isSecret: false,
            options: [
              { label: "A", description: "项目 A" },
              { label: "B", description: "项目 B" },
            ],
          },
        ],
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    // 内嵌气泡渲染：不再使用模态遮罩/弹窗
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(wrapper.find(".modal").exists()).toBe(false);
    expect(wrapper.find(".interaction-bubble").exists()).toBe(true);
    const optionB = wrapper
      .findAll(".option-btn")
      .find((b) => b.text().trim() === "B");
    expect(optionB).toBeTruthy();
    await optionB!.trigger("click");
    const submit = wrapper.findAll(".btn").find((b) => b.text().trim() === "提交");
    await submit!.trigger("click");

    expect(mockedRespond).toHaveBeenCalledTimes(1);
    const [interaction, result] = mockedRespond.mock.calls[0];
    expect(interaction.requestId).toBe(42);
    expect(result).toEqual({
      answers: { q1: { answers: ["B"] } },
    });
  });

  it("任意多题时一页一题，逐题选择后提交收集全部答案", async () => {
    const N = 6;
    const questions = Array.from({ length: N }, (_, i) => ({
      id: `q${i + 1}`,
      header: `第${i + 1}题`,
      question: `问题${i + 1}？`,
      isOther: false,
      isSecret: false,
      options: [
        { label: "A", description: "" },
        { label: "B", description: "" },
        { label: "C", description: "" },
        { label: "D", description: "" },
      ],
    }));
    store.interactions.push({
      requestId: 7,
      method: "item/tool/requestUserInput",
      params: { questions },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);

    const expectedAnswers: Record<string, { answers: string[] }> = {};
    for (let i = 0; i < N; i++) {
      // 每页只显示当前题
      expect(wrapper.text()).toContain(`第 ${i + 1} / ${N} 题`);
      for (let k = 0; k < N; k++) {
        if (k === i) {
          expect(wrapper.text()).toContain(`问题${k + 1}？`);
        } else {
          expect(wrapper.text()).not.toContain(`问题${k + 1}？`);
        }
      }
      // 选择当前题的选项（轮换 A/B/C/D）
      const pick = ["A", "B", "C", "D"][i % 4];
      await wrapper
        .findAll(".option-btn")
        .find((b) => b.text().trim() === pick)!
        .trigger("click");
      expectedAnswers[`q${i + 1}`] = { answers: [pick] };
      // 最后一题点“提交”，否则点“下一题”
      if (i < N - 1) {
        await wrapper.findAll(".btn").find((b) => b.text().trim() === "下一题")!.trigger("click");
      } else {
        await wrapper.findAll(".btn").find((b) => b.text().trim() === "提交")!.trigger("click");
      }
    }
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    const [, result] = mockedRespond.mock.calls[0];
    expect(result).toEqual({ answers: expectedAnswers });
  });
});

describe("审批/询问弹窗信息层级", () => {
  beforeEach(() => {
    store.interactions = [];
    mockedRespond.mockClear();
  });

  it("命令审批：标题与命令/原因置顶，详细信息默认折叠，展开后展示次要元数据", async () => {
    store.interactions.push({
      requestId: 10,
      method: "item/commandExecution/requestApproval",
      params: {
        command: "npm run build",
        cwd: "D:/repo",
        reason: "构建前端产物",
        additionalPermissions: { type: "workspaceWrite", writableRoots: ["D:/repo"] },
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    expect(wrapper.text()).toContain("批准执行命令");
    expect(wrapper.text()).toContain("npm run build");
    expect(wrapper.text()).toContain("构建前端产物");

    const details = wrapper.find(".approval-details");
    expect((details.element as HTMLDetailsElement).open).toBe(false);
    await details.find("summary").trigger("click");
    expect((details.element as HTMLDetailsElement).open).toBe(true);
    expect(details.text()).toContain("D:/repo");
    expect(details.text()).toContain("type：workspaceWrite");
    expect(details.text()).not.toContain("{");
  });

  it("权限审批：无原因时显示兜底文案，权限为可读文本", async () => {
    store.interactions.push({
      requestId: 11,
      method: "item/permissions/requestApproval",
      params: {
        grantRoot: "D:/repo",
        permissions: { type: "workspaceWrite", writableRoots: ["D:/repo", "D:/tmp"] },
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    expect(wrapper.text()).toContain("是否允许此操作？");
    const details = wrapper.find(".approval-details");
    (details.element as HTMLDetailsElement).open = true;
    expect(details.text()).toContain("writableRoots：D:/repo；D:/tmp");
    expect(details.text()).not.toContain("{");
  });

  it("文件变更审批无任何元数据时只显示兜底文案，不出现详细信息折叠区", async () => {
    store.interactions.push({
      requestId: 12,
      method: "item/fileChange/requestApproval",
      params: {},
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    expect(wrapper.text()).toContain("是否允许此操作？");
    expect(wrapper.find(".approval-details").exists()).toBe(false);
  });

  it("命令审批：批准/拒绝按钮内嵌在气泡中，点击后按协议返回", async () => {
    store.interactions.push({
      requestId: 13,
      method: "item/commandExecution/requestApproval",
      params: { command: "npm test" },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    expect(wrapper.find(".modal-mask").exists()).toBe(false);
    expect(wrapper.find(".interaction-bubble").exists()).toBe(true);
    const buttons = wrapper.findAll(".interaction-foot .btn");
    expect(buttons.map((b) => b.text().trim())).toEqual(["拒绝", "批准"]);
    await buttons.find((b) => b.text().trim() === "批准")!.trigger("click");
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({ decision: "accept" });
  });

  it("提问类交互按 Escape 触发取消（拒绝），审批类不响应 Escape", async () => {
    store.interactions.push({
      requestId: 14,
      method: "item/tool/requestUserInput",
      params: {
        questions: [
          {
            id: "q1",
            header: "选择项目",
            question: "要处理哪个项目？",
            isOther: false,
            isSecret: false,
            options: [{ label: "A", description: "项目 A" }],
          },
        ],
      },
      at: Date.now(),
    });
    mount(InlineInteraction);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      decision: { denied: { rejection: "用户拒绝" } },
    });

    // 审批类：Escape 不触发任何响应
    mockedRespond.mockClear();
    store.interactions = [];
    store.interactions.push({
      requestId: 15,
      method: "item/commandExecution/requestApproval",
      params: { command: "npm test" },
      at: Date.now(),
    });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(mockedRespond).not.toHaveBeenCalled();
    store.interactions = [];
  });
});
