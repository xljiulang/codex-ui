// useCodex 拆分模块：定时任务（列表加载、管理命令封装、调度描述纯函数）
// 后端 scheduled_tasks.rs 持有 SQLite 存储与调度器；本模块只做命令转发与展示辅助。
import { invoke } from "@tauri-apps/api/core";
import type { ScheduledTask, TaskRunRecord } from "../../lib/types";
import { store } from "./store";

/** 加载任务列表到全局 store（boot 时调用一次；此后由 scheduled-tasks/event 快照驱动）。 */
export async function loadScheduledTasks(): Promise<void> {
  try {
    const tasks = await invoke<ScheduledTask[]>("scheduled_tasks_list");
    // 防御：后端不可用/返回异常时保持现有列表不变
    if (Array.isArray(tasks)) store.scheduledTasks = tasks;
  } catch {
    // 非 Tauri 环境/后端不可用时静默，管理区块展示空列表
  }
}

/** 创建定时任务（唯一创建入口：动态工具 codexui.add_scheduled_task 的确认后调用）。 */
export async function addScheduledTask(input: {
  name: string;
  prompt: string;
  cron: string;
  threadId: string;
  busyPolicy?: "defer" | "skip";
}): Promise<ScheduledTask> {
  const task = await invoke<ScheduledTask>("scheduled_task_add", {
    name: input.name,
    prompt: input.prompt,
    cron: input.cron,
    threadId: input.threadId,
    busyPolicy: input.busyPolicy ?? null,
  });
  store.scheduledTasks = [
    ...store.scheduledTasks.filter((t) => t.id !== task.id),
    task,
  ];
  return task;
}

export async function removeScheduledTask(id: string): Promise<void> {
  await invoke("scheduled_task_remove", { id });
}

export async function setScheduledTaskEnabled(id: string, enabled: boolean): Promise<void> {
  await invoke("scheduled_task_set_enabled", { id, enabled });
}

export async function setScheduledTaskBusyPolicy(
  id: string,
  policy: "defer" | "skip",
): Promise<void> {
  await invoke("scheduled_task_set_busy_policy", { id, policy });
}

/** 编辑定时任务：更新标题、提示词与忙时策略（cron/绑定会话不变）。 */
export async function updateScheduledTask(
  id: string,
  input: { name: string; prompt: string; busyPolicy: "defer" | "skip" },
): Promise<ScheduledTask> {
  const task = await invoke<ScheduledTask>("scheduled_task_update", {
    id,
    name: input.name,
    prompt: input.prompt,
    busyPolicy: input.busyPolicy,
  });
  store.scheduledTasks = store.scheduledTasks.map((t) =>
    t.id === task.id ? task : t,
  );
  return task;
}

export async function runScheduledTaskNow(id: string): Promise<void> {
  await invoke("scheduled_task_run_now", { id });
}

/** 按任务分页拉取执行记录（倒序）。 */
export async function loadScheduledTaskRuns(
  taskId: string,
  limit = 20,
  offset = 0,
): Promise<TaskRunRecord[]> {
  return invoke<TaskRunRecord[]>("scheduled_task_runs", { taskId, limit, offset });
}

const WEEKDAYS = ["日", "一", "二", "三", "四", "五", "六"];

function pad2(v: string): string {
  return v.padStart(2, "0");
}

/**
 * cron 调度描述（6 字段「秒 分 时 日 月 周」/ 7 字段末尾年份）：
 * 识别常见模式给出友好文案，识别不了回退显示原表达式（纯展示，不影响调度）。
 */
export function describeSchedule(cron: string): string {
  const parts = cron.trim().split(/\s+/);
  if (parts.length !== 6 && parts.length !== 7) return cron;
  const [sec, min, hour, dom, mon, dow, year] = parts;
  const wild = (f: string) => f === "*";
  const isNum = (f: string) => /^\d+$/.test(f);
  const hm = `${pad2(hour)}:${pad2(min)}`;

  // 每 N 分钟：0 */N * * * *
  const stepMin = /^\*\/(\d+)$/.exec(min);
  if (stepMin && (sec === "0" || sec === "*") && wild(hour) && wild(dom) && wild(mon) && wild(dow)) {
    const n = Number(stepMin[1]);
    if (n > 0) return `每 ${n} 分钟`;
  }
  // 每小时：0 0 * * * *
  if (sec === "0" && min === "0" && wild(hour) && wild(dom) && wild(mon) && wild(dow)) {
    return "每小时";
  }
  // 每天 / 每周：时分固定
  if ((sec === "0" || sec === "*") && isNum(hour) && isNum(min) && wild(mon)) {
    if (wild(dom) && wild(dow)) return `每天 ${hm}`;
    if (wild(dom) && isNum(dow)) {
      const wd = Number(dow) % 7;
      return `每周${WEEKDAYS[wd] ?? dow} ${hm}`;
    }
  }
  // 单次（带年份）
  if (
    parts.length === 7 &&
    isNum(year!) && isNum(mon) && isNum(dom) && isNum(hour) && isNum(min)
  ) {
    return `单次：${year}-${pad2(mon)}-${pad2(dom)} ${hm}`;
  }
  return cron;
}
