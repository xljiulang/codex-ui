import { describe, expect, it, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../../composables/useCodex")>();
  return {
    ...mod,
    loadScheduledTasks: vi.fn(async () => {}),
    loadScheduledTaskRuns: vi.fn(async () => []),
    openSession: vi.fn(async () => {}),
    removeScheduledTask: vi.fn(async () => {}),
    runScheduledTaskNow: vi.fn(async () => {}),
    setScheduledTaskBusyPolicy: vi.fn(async () => {}),
    setScheduledTaskEnabled: vi.fn(async () => {}),
    askConfirm: vi.fn(async () => true),
    setToast: vi.fn(),
    toastError: vi.fn((e: unknown) => String(e)),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));

import ScheduledTasksSection from "../settings/ScheduledTasksSection.vue";
import {
  askConfirm,
  loadScheduledTaskRuns,
  openSession,
  removeScheduledTask,
  runScheduledTaskNow,
  setScheduledTaskBusyPolicy,
  store,
} from "../../composables/useCodex";
import { ICON_HISTORY, ICON_SKIP } from "../../lib/icons";

const mockedRuns = vi.mocked(loadScheduledTaskRuns);
const mockedRemove = vi.mocked(removeScheduledTask);
const mockedRunNow = vi.mocked(runScheduledTaskNow);
const mockedSetPolicy = vi.mocked(setScheduledTaskBusyPolicy);
const mockedConfirm = vi.mocked(askConfirm);
const mockedOpenSession = vi.mocked(openSession);

const now = Math.floor(Date.now() / 1000);

const activeTask = {
  id: "task-1",
  name: "每日总结",
  prompt: "总结昨天的提交",
  cron: "0 0 9 * * *",
  threadId: "t1",
  busyPolicy: "defer" as const,
  enabled: true,
  done: false,
  createdAt: now - 100,
  nextRun: now + 3600,
};

beforeEach(() => {
  store.scheduledTasks = [];
  store.threads = [
    { id: "t1", name: "会话一", preview: "", createdAt: now, recencyAt: now, cwd: "D:\\repo" },
  ] as never;
  vi.clearAllMocks();
});

describe("ScheduledTasksSection 定时任务管理区块", () => {
  it("无任务时显示空状态引导文案", () => {
    const wrapper = mount(ScheduledTasksSection);
    expect(wrapper.text()).toContain("暂无定时任务");
    expect(wrapper.text()).toContain("可在对话中让 codex 创建");
  });

  it("任务行展示名称/调度描述/绑定会话（实时解析）/下次运行", () => {
    store.scheduledTasks = [{ ...activeTask }];
    const wrapper = mount(ScheduledTasksSection);
    expect(wrapper.text()).toContain("每日总结");
    expect(wrapper.text()).toContain("每天 09:00");
    expect(wrapper.text()).toContain("会话一");
    expect(wrapper.text()).not.toContain("已完成（");
  });

  it("展开任务显示 prompt 全文与按任务分开的执行记录", async () => {
    store.scheduledTasks = [{ ...activeTask }];
    mockedRuns.mockResolvedValue([
      {
        id: 1,
        taskId: "task-1",
        startedAt: now,
        status: "success",
        durationMs: 1500,
        turnId: "turn-1",
        result: "已完成总结",
      },
    ]);
    const wrapper = mount(ScheduledTasksSection);
    await wrapper.find(".sched-task-main").trigger("click");
    await nextTick();
    expect(mockedRuns).toHaveBeenCalledWith("task-1", 20, 0);
    expect(wrapper.find(".sched-prompt").text()).toBe("总结昨天的提交");
    expect(wrapper.find(".sched-run-status").classes()).toContain("st-success");
    expect(wrapper.text()).toContain("已完成总结");
    // 点击结果摘要展开全文
    await wrapper.find(".sched-run-result").trigger("click");
    expect(wrapper.find(".sched-run-result").classes()).toContain("open");
  });

  it("行内动作顺序与命令：忙时策略 → 立即执行 → 打开会话 → 删除", async () => {
    store.scheduledTasks = [{ ...activeTask }];
    const wrapper = mount(ScheduledTasksSection);
    const actionBtns = wrapper.findAll(".model-provider-actions .btn-icon");
    expect(actionBtns.map((b) => b.attributes("aria-label"))).toEqual([
      "会话忙时顺延执行（切换为跳过本次）",
      "立即执行",
      "打开会话",
      "删除",
    ]);
    // 忙时策略图标：时钟（顺延态）→ 点击切为跳过
    expect(actionBtns[0]!.find("path").attributes("d")).toBe(ICON_HISTORY);
    await actionBtns[0]!.trigger("click");
    expect(mockedSetPolicy).toHaveBeenCalledWith("task-1", "skip");
    await actionBtns[1]!.trigger("click"); // 立即执行
    expect(mockedRunNow).toHaveBeenCalledWith("task-1");
    await actionBtns[2]!.trigger("click"); // 打开会话
    expect(mockedOpenSession).toHaveBeenCalledWith("t1");
  });

  it("忙时策略图标随状态切换：跳过态显示快进图标，点击切回顺延", async () => {
    store.scheduledTasks = [{ ...activeTask, busyPolicy: "skip" }];
    const wrapper = mount(ScheduledTasksSection);
    const btn = wrapper.find(".sched-policy-btn");
    expect(btn.find("path").attributes("d")).toBe(ICON_SKIP);
    await btn.trigger("click");
    expect(mockedSetPolicy).toHaveBeenCalledWith("task-1", "defer");
  });

  it("删除需确认，确认后调用删除（记录一并删除）", async () => {
    store.scheduledTasks = [{ ...activeTask }];
    const wrapper = mount(ScheduledTasksSection);
    // 第一次：用户取消 → 不删除
    mockedConfirm.mockResolvedValueOnce(false);
    await wrapper.find(".sched-row-del").trigger("click");
    expect(mockedConfirm).toHaveBeenCalledTimes(1);
    const req = mockedConfirm.mock.calls[0]![0];
    expect(req.message).toContain("全部执行记录将一并删除");
    expect(mockedRemove).not.toHaveBeenCalled();
    // 第二次：用户确认 → 删除
    await wrapper.find(".sched-row-del").trigger("click");
    expect(mockedRemove).toHaveBeenCalledWith("task-1");
  });

  it("单次任务完成后进入「已完成」归档分组，可展开查看记录", async () => {
    store.scheduledTasks = [
      { ...activeTask },
      { ...activeTask, id: "task-2", name: "一次性任务", enabled: false, done: true, cron: "0 0 9 20 1 * 2026" },
    ];
    const wrapper = mount(ScheduledTasksSection);
    expect(wrapper.text()).toContain("已完成（1）");
    // 折叠时隐藏任务名
    expect(wrapper.text()).not.toContain("一次性任务");
    const storeTasks = store.scheduledTasks;
    const idx = storeTasks.findIndex((t) => t.id === "task-2");
    expect(idx).toBeGreaterThanOrEqual(0);
    // 展开归档分组：主列表不再渲染活跃任务行，done 行出现
    const doneHead = wrapper.find(".sched-done-head");
    await doneHead.trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("一次性任务");
    // 活跃任务仍在主列表
    expect(wrapper.text()).toContain("每日总结");
  });

  it("曾拉到空结果的任务，执行后重新展开会重新拉取记录", async () => {
    store.scheduledTasks = [{ ...activeTask }];
    mockedRuns.mockResolvedValueOnce([]); // 首次展开：任务当时还没执行
    const wrapper = mount(ScheduledTasksSection);
    await wrapper.find(".sched-task-main").trigger("click");
    await nextTick();
    expect(wrapper.text()).toContain("暂无执行记录");
    // 收起，任务执行完成
    await wrapper.find(".sched-task-main").trigger("click");
    mockedRuns.mockResolvedValueOnce([
      { id: 1, taskId: "task-1", startedAt: now, status: "success", result: "结果" },
    ]);
    // 重新展开：应重置 done 标记并重新拉取
    await wrapper.find(".sched-task-main").trigger("click");
    await nextTick();
    expect(mockedRuns).toHaveBeenCalledTimes(2);
    expect(wrapper.find(".sched-run-status").classes()).toContain("st-success");
    expect(wrapper.text()).toContain("结果");
  });

  it("绑定会话被删除时显示「会话已删除」", () => {
    store.scheduledTasks = [{ ...activeTask, threadId: "t-gone" }];
    const wrapper = mount(ScheduledTasksSection);
    expect(wrapper.text()).toContain("会话已删除");
  });
});
