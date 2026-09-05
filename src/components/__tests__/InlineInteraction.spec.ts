import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import { nextTick } from "vue";

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
    store.interactions.splice(0);
    store.interactions.push({
      requestId: 15,
      method: "item/commandExecution/requestApproval",
      params: { command: "npm test" },
      at: Date.now(),
    });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(mockedRespond).not.toHaveBeenCalled();
    store.interactions.splice(0);
  });
});

describe("permissions / elicitation 应答按协议", () => {
  beforeEach(() => {
    store.interactions = [];
    mockedRespond.mockClear();
  });

  it("权限审批批准：回传请求的 permissions 子集 + scope turn", async () => {
    store.interactions.push({
      requestId: 50,
      method: "item/permissions/requestApproval",
      params: {
        threadId: "thr_1",
        turnId: "turn_1",
        itemId: "call_1",
        environmentId: "local",
        cwd: "D:/repo",
        reason: "Select a workspace root",
        permissions: {
          fileSystem: { write: ["D:/repo"] },
          network: { enabled: false },
        },
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    await wrapper
      .findAll(".interaction-foot .btn")
      .find((b) => b.text().trim() === "批准")!
      .trigger("click");
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      permissions: {
        fileSystem: { write: ["D:/repo"] },
        network: { enabled: false },
      },
      scope: "turn",
    });
  });

  it("权限审批会话级批准：scope session", async () => {
    store.interactions.push({
      requestId: 51,
      method: "item/permissions/requestApproval",
      params: {
        permissions: { fileSystem: { write: ["D:/repo"] } },
        availableDecisions: ["acceptForSession"],
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    await wrapper
      .findAll(".interaction-foot .btn")
      .find((b) => b.text().trim() === "本次会话批准")!
      .trigger("click");
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      permissions: { fileSystem: { write: ["D:/repo"] } },
      scope: "session",
    });
  });

  it("权限审批拒绝：空授予子集即全部拒绝", async () => {
    store.interactions.push({
      requestId: 55,
      method: "item/permissions/requestApproval",
      params: {
        permissions: { fileSystem: { write: ["D:/repo"] } },
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    await wrapper
      .findAll(".interaction-foot .btn")
      .find((b) => b.text().trim() === "拒绝")!
      .trigger("click");
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      permissions: {},
      scope: "turn",
    });
  });

  it("elicitation 表单提交：action accept + 结构化 content", async () => {
    store.interactions.push({
      requestId: 52,
      method: "mcpServer/elicitation/request",
      params: {
        threadId: "thr_1",
        turnId: null,
        serverName: "my-mcp",
        mode: "form",
        message: "填写表单",
        requestedSchema: {
          type: "object",
          properties: {
            field: { title: "字段", description: "desc", type: "string" },
          },
        },
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    await wrapper.find("input").setValue("abc");
    await wrapper
      .findAll(".btn")
      .find((b) => b.text().trim() === "提交")!
      .trigger("click");
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      action: "accept",
      content: { field: "abc" },
    });
  });

  it("elicitation enum/boolean/number：按 schema 类型渲染并保留类型", async () => {
    store.interactions.push({
      requestId: 56,
      method: "mcpServer/elicitation/request",
      params: {
        serverName: "my-mcp",
        mode: "form",
        message: "填写配置",
        requestedSchema: {
          type: "object",
          properties: {
            mode: {
              title: "模式",
              type: "string",
              enum: ["fast", "careful"],
              enumNames: ["快速", "谨慎"],
            },
            flag: { title: "开关", type: "boolean" },
            count: { title: "数量", type: "integer" },
          },
        },
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);

    // 单选 enum → AppSelect，布尔 → AppSelect，数字 → number input
    const triggers = wrapper.findAll("button.app-select");
    expect(triggers).toHaveLength(2);
    // 展开模式下拉（弹层 Teleport 到 body）并选择「谨慎」
    await triggers[0].trigger("click");
    await nextTick();
    const target = Array.from(
      document.body.querySelectorAll<HTMLElement>(".app-select-option"),
    ).find((o) => o.textContent?.trim() === "谨慎");
    expect(target).toBeTruthy();
    target!.click();
    await nextTick();
    await wrapper.find('input[type="number"]').setValue("3");

    await wrapper
      .findAll(".btn")
      .find((b) => b.text().trim() === "提交")!
      .trigger("click");

    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      action: "accept",
      content: { mode: "careful", flag: false, count: 3 },
    });
  });

  it("elicitation 必填校验：缺失必填字段不提交并提示", async () => {
    store.toast = "";
    store.interactions.push({
      requestId: 57,
      method: "mcpServer/elicitation/request",
      params: {
        serverName: "my-mcp",
        mode: "form",
        message: "填写必填项",
        requestedSchema: {
          type: "object",
          required: ["field"],
          properties: {
            field: { title: "必填字段", description: "desc", type: "string" },
          },
        },
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);

    await wrapper
      .findAll(".btn")
      .find((b) => b.text().trim() === "提交")!
      .trigger("click");

    expect(mockedRespond).not.toHaveBeenCalled();
    expect(store.toast).toContain("必填");
  });

  it("elicitation url 模式接受：action accept + content null", async () => {
    store.interactions.push({
      requestId: 53,
      method: "mcpServer/elicitation/request",
      params: {
        serverName: "my-mcp",
        mode: "url",
        message: "打开链接",
        url: "https://example.com",
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    await wrapper
      .findAll(".btn")
      .find((b) => b.text().trim() === "已在浏览器打开")!
      .trigger("click");
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      action: "accept",
      content: null,
    });
  });

  it("elicitation 拒绝：action decline + content null", async () => {
    store.interactions.push({
      requestId: 54,
      method: "mcpServer/elicitation/request",
      params: {
        serverName: "my-mcp",
        mode: "url",
        message: "打开链接",
        url: "https://example.com",
      },
      at: Date.now(),
    });
    const wrapper = mount(InlineInteraction);
    await wrapper
      .findAll(".btn")
      .find((b) => b.text().trim() === "拒绝")!
      .trigger("click");
    expect(mockedRespond).toHaveBeenCalledTimes(1);
    expect(mockedRespond.mock.calls[0][1]).toEqual({
      action: "decline",
      content: null,
    });
  });
});
