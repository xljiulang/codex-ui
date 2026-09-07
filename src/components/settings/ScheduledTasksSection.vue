<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import {
  askConfirm,
  describeSchedule,
  loadScheduledTaskRuns,
  loadScheduledTasks,
  openSession,
  removeScheduledTask,
  runScheduledTaskNow,
  setScheduledTaskBusyPolicy,
  setToast,
  threadTitle,
  toastError,
} from "../../composables/useCodex";
import { store } from "../../composables/useCodex";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_DELETE,
  ICON_HISTORY,
  ICON_PLAY,
  ICON_REFRESH,
  ICON_SKIP,
} from "../../lib/icons";
import type { ScheduledTask, TaskRunRecord, TaskRunStatus } from "../../lib/types";

const props = defineProps<{ active: boolean }>();

/** 展开的任务 id 与各任务的执行记录状态（按任务分开查看，无混排视图） */
const expandedId = ref("");
const runsState = reactive<Record<string, { items: TaskRunRecord[]; loading: boolean; done: boolean }>>({});
const RUN_PAGE = 20;
/** 已点开过全文的结果记录 id（未点开时只显示单行摘要） */
const resultOpen = reactive(new Set<number>());
/** 「已完成」归档分组默认折叠 */
const doneOpen = ref(false);

onMounted(() => {
  void loadScheduledTasks();
});

/** 渲染行：活跃任务 + 底部「已完成」归档分组（默认折叠，仅显示数量） */
interface TaskRow {
  kind: "task";
  task: ScheduledTask;
}
interface DoneHead {
  kind: "done-head";
}
type Row = TaskRow | DoneHead;

const rows = computed<Row[]>(() => {
  const active = store.scheduledTasks.filter((t) => !t.done);
  const done = store.scheduledTasks.filter((t) => t.done);
  const out: Row[] = active.map((task) => ({ kind: "task", task }));
  if (done.length) {
    out.push({ kind: "done-head" });
    if (doneOpen.value) for (const task of done) out.push({ kind: "task", task });
  }
  return out;
});

/** 会话信息实时解析：改名自动跟随；线程不在列表显示「会话已删除」 */
function threadLabel(threadId: string): string {
  const t = store.threads.find((x) => x.id === threadId);
  return t ? threadTitle(t) : "会话已删除";
}

function nextRunLabel(t: ScheduledTask): string {
  if (t.done) return "已完成";
  if (!t.enabled) return "已停用";
  if (t.nextRun == null) return "无";
  // 到期未跑 = 正在顺延等待会话空闲（next_run 在执行/跳过后才会推进）
  if (t.nextRun <= Date.now() / 1000) return "顺延中（等待会话空闲）";
  return fmtTime(t.nextRun);
}

function fmtTime(ts: number): string {
  const d = new Date(ts * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

const RUN_STATUS_LABELS: Record<TaskRunStatus, string> = {
  running: "进行中",
  success: "成功",
  failed: "失败",
  skipped: "已跳过",
  missed: "已错过",
};

function fmtDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${s % 60}s`;
}

function oneLine(text: string): string {
  const first = text.split("\n")[0] ?? text;
  return first.length > 120 ? `${first.slice(0, 120)}…` : first;
}

async function toggleExpand(t: ScheduledTask) {
  if (expandedId.value === t.id) {
    expandedId.value = "";
    return;
  }
  expandedId.value = t.id;
  if (!runsState[t.id]) runsState[t.id] = { items: [], loading: false, done: false };
  if (!runsState[t.id].items.length) {
    // 之前拉到过空结果（任务当时还没执行）也重置重拉，避免永久「暂无执行记录」
    runsState[t.id].done = false;
    await loadRuns(t.id);
  }
}

async function loadRuns(taskId: string) {
  const st = runsState[taskId];
  if (!st || st.loading || st.done) return;
  st.loading = true;
  try {
    const more = await loadScheduledTaskRuns(taskId, RUN_PAGE, st.items.length);
    st.items.push(...more);
    if (more.length < RUN_PAGE) st.done = true;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    st.loading = false;
  }
}

/** 后端推送任务变更且其记录正展开时重拉（执行完成后结果立即可见；seq 保证每次事件都触发） */
watch(
  () => store.scheduledTaskChange,
  async (chg) => {
    const st = chg.taskId ? runsState[chg.taskId] : undefined;
    if (!chg.taskId || expandedId.value !== chg.taskId || !st) return;
    st.done = false;
    st.items = [];
    await loadRuns(chg.taskId);
  },
);

/** 忙时策略图标切换：顺延（时钟）⇄ 跳过（快进） */
async function onPolicyToggle(t: ScheduledTask) {
  const policy = t.busyPolicy === "defer" ? "skip" : "defer";
  try {
    await setScheduledTaskBusyPolicy(t.id, policy);
  } catch (e) {
    setToast(toastError(e));
  }
}

async function onRunNow(t: ScheduledTask) {
  try {
    await runScheduledTaskNow(t.id);
    setToast(`任务「${t.name}」已开始执行`);
  } catch (e) {
    setToast(toastError(e));
  }
}

async function onRemove(t: ScheduledTask) {
  const ok = await askConfirm({
    title: "删除定时任务",
    message: `确定删除任务「${t.name}」吗？其全部执行记录将一并删除。`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  try {
    await removeScheduledTask(t.id);
    if (expandedId.value === t.id) expandedId.value = "";
  } catch (e) {
    setToast(toastError(e));
  }
}

function toggleResult(runId: number) {
  if (resultOpen.has(runId)) resultOpen.delete(runId);
  else resultOpen.add(runId);
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-scheduled">
    <h2 class="settings-section-title">定时任务</h2>
    <p class="settings-section-desc">
      到点后在绑定的原会话中自动发送任务指令；任务由 codex 在对话中创建（动态工具 codexui.add_scheduled_task）。
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>任务列表</h3>
        <div class="model-config-head-actions">
          <button
            class="btn btn-icon model-config-reload-btn"
            aria-label="刷新"
            v-tooltip="'刷新'"
            @click="loadScheduledTasks"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_REFRESH" />
            </svg>
          </button>
        </div>
      </div>

      <div class="sched-list">
        <div v-if="!store.scheduledTasks.length" class="plugin-empty">
          暂无定时任务。可在对话中让 codex 创建，例如：『每天早上 9 点总结昨天的提交』。
        </div>

        <template v-for="row in rows" :key="row.kind === 'task' ? row.task.id : 'sched-done-head'">
          <button
            v-if="row.kind === 'done-head'"
            class="sched-done-head"
            :aria-expanded="doneOpen"
            @click="doneOpen = !doneOpen"
          >
            已完成（{{ store.scheduledTasks.filter((t) => t.done).length }}）{{ doneOpen ? "▲" : "▼" }}
          </button>
          <div v-else class="sched-task" :class="{ expanded: expandedId === row.task.id }">
            <div class="sched-task-row">
              <button
                class="sched-task-main"
                :aria-expanded="expandedId === row.task.id"
                @click="toggleExpand(row.task)"
              >
                <span class="row-icon" aria-hidden="true">
                  <svg viewBox="0 0 24 24">
                    <path :d="ICON_HISTORY" fill="currentColor" />
                  </svg>
                </span>
                <span class="sched-meta">
                  <span class="sched-title-row">
                    <svg class="sched-arrow" viewBox="0 0 24 24" aria-hidden="true">
                      <path :d="expandedId === row.task.id ? ICON_ARROW_DOWN : ICON_ARROW_RIGHT" />
                    </svg>
                    <span class="sched-name">{{ row.task.name }}</span>
                  </span>
                  <span class="sched-badges">
                    <span class="sched-chip">{{ describeSchedule(row.task.cron) }}</span>
                    <span
                      class="sched-session"
                      role="button"
                      :aria-label="`打开会话（${threadLabel(row.task.threadId)}）`"
                      @click.stop="openSession(row.task.threadId)"
                    >
                      {{ threadLabel(row.task.threadId) }}
                    </span>
                    <span class="sched-next">{{ nextRunLabel(row.task) }}</span>
                  </span>
                </span>
              </button>
              <div class="model-provider-actions">
                <button
                  v-if="!row.task.done"
                  class="btn btn-icon sched-policy-btn"
                  :aria-label="
                    row.task.busyPolicy === 'defer'
                      ? '会话忙时顺延执行（切换为跳过本次）'
                      : '会话忙时跳过本次（切换为顺延执行）'
                  "
                  v-tooltip="
                    row.task.busyPolicy === 'defer'
                      ? '会话忙时：顺延执行'
                      : '会话忙时：跳过本次'
                  "
                  @click="onPolicyToggle(row.task)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="row.task.busyPolicy === 'defer' ? ICON_HISTORY : ICON_SKIP" />
                  </svg>
                </button>
                <button
                  v-if="!row.task.done"
                  class="btn btn-icon sched-row-btn"
                  :disabled="!row.task.enabled"
                  aria-label="立即执行"
                  v-tooltip="'立即执行'"
                  @click="onRunNow(row.task)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_PLAY" />
                  </svg>
                </button>
                <button
                  class="btn btn-icon danger sched-row-del"
                  aria-label="删除"
                  v-tooltip="'删除'"
                  @click="onRemove(row.task)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_DELETE" />
                  </svg>
                </button>
              </div>
            </div>
            <div v-if="expandedId === row.task.id" class="sched-detail">
              <div class="sched-detail-label">任务指令（prompt）</div>
              <pre class="sched-prompt">{{ row.task.prompt }}</pre>
              <div class="sched-detail-label">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path :d="ICON_HISTORY" /></svg>
                执行记录
              </div>
              <div v-if="runsState[row.task.id]?.items.length" class="sched-runs">
                <div v-for="r in runsState[row.task.id].items" :key="r.id" class="sched-run">
                  <div class="sched-run-line">
                    <span class="sched-run-status" :class="`st-${r.status}`">
                      {{ RUN_STATUS_LABELS[r.status] ?? r.status }}
                    </span>
                    <span class="sched-run-time">{{ fmtTime(r.startedAt) }}</span>
                    <span v-if="r.durationMs != null" class="sched-run-duration">
                      {{ fmtDuration(r.durationMs) }}
                    </span>
                    <span v-if="r.error" class="sched-run-error" v-tooltip="r.error">{{ r.error }}</span>
                  </div>
                  <div
                    v-if="r.result"
                    class="sched-run-result"
                    :class="{ open: resultOpen.has(r.id) }"
                    @click="toggleResult(r.id)"
                  >{{ resultOpen.has(r.id) ? r.result : oneLine(r.result) }}</div>
                </div>
                <button
                  v-if="!runsState[row.task.id].done"
                  class="sched-run-more"
                  :disabled="runsState[row.task.id].loading"
                  @click="loadRuns(row.task.id)"
                >
                  {{ runsState[row.task.id].loading ? "加载中…" : "加载更多" }}
                </button>
              </div>
              <div v-else-if="!runsState[row.task.id]?.loading" class="sched-run-more">暂无执行记录</div>
            </div>
          </div>
        </template>
      </div>
    </div>
  </section>
</template>
