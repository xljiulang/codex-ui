<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { respondInteraction, store } from "../composables/useCodex";
import type { PendingInteraction } from "../lib/types";

const current = computed<PendingInteraction | undefined>(() => store.interactions[0]);
const params = computed(() => (current.value?.params ?? {}) as Record<string, unknown>);
const modalEl = ref<HTMLElement | null>(null);

const V2_APPROVAL_METHODS = [
  "item/commandExecution/requestApproval",
  "item/fileChange/requestApproval",
  "item/permissions/requestApproval",
];
const LEGACY_APPROVAL_METHODS = ["execCommandApproval", "applyPatchApproval"];

const isV2Approval = computed(() =>
  V2_APPROVAL_METHODS.includes(current.value?.method ?? ""),
);
const isLegacyApproval = computed(() =>
  LEGACY_APPROVAL_METHODS.includes(current.value?.method ?? ""),
);
const isCommandApproval = computed(
  () =>
    current.value?.method === "item/commandExecution/requestApproval" ||
    current.value?.method === "execCommandApproval",
);
const isReviewApproval = computed(
  () =>
    !isCommandApproval.value &&
    (isV2Approval.value || isLegacyApproval.value),
);
const isUserInput = computed(() => current.value?.method === "item/tool/requestUserInput");
const isElicitation = computed(() => current.value?.method === "mcpServer/elicitation/request");
const hasCommandDetails = computed(
  () =>
    Boolean(params.value.cwd) ||
    commandActions().length > 0 ||
    Boolean(params.value.additionalPermissions),
);

const selectedOptions = reactive<Record<string, string>>({});
const otherInputs = reactive<Record<string, string>>({});
const formValues = reactive<Record<string, string>>({});
let lastFocus: HTMLElement | null = null;

// 分步提问：一次只展示一道题
const qIndex = ref(0);
const questionList = computed(() => questions());
const currentQuestion = computed(
  () => questionList.value[qIndex.value] ?? null,
);
const hasMultipleQuestions = computed(() => questionList.value.length > 1);
const isLastQuestion = computed(
  () => qIndex.value >= questionList.value.length - 1,
);

watch(current, () => {
  for (const k of Object.keys(selectedOptions)) delete selectedOptions[k];
  for (const k of Object.keys(otherInputs)) delete otherInputs[k];
  for (const k of Object.keys(formValues)) delete formValues[k];
  qIndex.value = 0;
  if (current.value) {
    // 打开时把焦点移入弹窗（首个主按钮），关闭时还原
    lastFocus = document.activeElement as HTMLElement | null;
    void nextTick(() => {
      const primary = modalEl.value?.querySelector<HTMLElement>(
        ".modal-foot .btn.primary",
      );
      (primary ?? modalEl.value)?.focus();
    });
  } else if (lastFocus) {
    lastFocus.focus?.();
    lastFocus = null;
  }
});

function onKeydown(e: KeyboardEvent) {
  // Escape：仅对提问/表单类弹窗执行“取消”，审批必须明确选择
  if (e.key === "Escape" && current.value && (isUserInput.value || isElicitation.value)) {
    reject();
  }
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));

function str(v: unknown): string {
  return v == null ? "" : String(v);
}

function list(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}

function obj(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" ? (v as Record<string, unknown>) : {};
}

function availableDecisions(): unknown[] {
  return list(params.value.availableDecisions);
}

function hasDecision(needle: string): boolean {
  return availableDecisions().some((d) => {
    if (typeof d === "string") return d === needle;
    if (d && typeof d === "object") {
      const keys = Object.keys(d as Record<string, unknown>);
      return keys.some((k) => k.startsWith(needle));
    }
    return false;
  });
}

function commandText(): string {
  const c = params.value.command;
  if (typeof c === "string") return c;
  return list(c).join(" ");
}

function commandActions(): string[] {
  return list(params.value.commandActions)
    .map((a) => {
      const o = obj(a);
      return [str(o.command), str(o.type)].filter(Boolean).join(" · ");
    })
    .filter(Boolean);
}

function questions(): Record<string, unknown>[] {
  return list(params.value.questions) as Record<string, unknown>[];
}

/** 把权限对象/数组转成易读文本（对象渲染为 `键：值`，数组用“；”连接） */
function permText(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (Array.isArray(v)) {
    return v
      .map((x) => permText(x))
      .filter(Boolean)
      .join("；");
  }
  if (typeof v === "object") {
    return Object.entries(v as Record<string, unknown>)
      .map(([k, val]) => `${k}：${permText(val)}`)
      .join("；");
  }
  return String(v);
}

function approve() {
  if (!current.value) return;
  void respondInteraction(
    current.value,
    isV2Approval.value ? { decision: "accept" } : { decision: "approved" },
  );
}

function approveSession() {
  if (!current.value) return;
  void respondInteraction(
    current.value,
    isV2Approval.value
      ? { decision: "acceptForSession" }
      : { decision: "approved_for_session" },
  );
}

function approveWithRule() {
  if (!current.value) return;
  void respondInteraction(current.value, {
    decision: {
      acceptWithExecpolicyAmendment: {
        execpolicy_amendment: params.value.proposedExecpolicyAmendment,
      },
    },
  });
}

function reject() {
  if (!current.value) return;
  const decision = isV2Approval.value
    ? hasDecision("decline")
      ? "decline"
      : isCommandApproval.value
        ? "cancel"
        : "decline"
    : { denied: { rejection: "用户拒绝" } };
  void respondInteraction(current.value, { decision });
}

function submitUserInput() {
  if (!current.value) return;
  // 协议要求按问题 id 返回：{ answers: { [questionId]: { answers: [选择] } } }
  const answers: Record<string, { answers: string[] }> = {};
  for (const q of questions()) {
    const id = str(q.id);
    const selected = selectedOptions[id];
    const value =
      q.isOther && selected === "__other__" ? (otherInputs[id] ?? "") : (selected ?? "");
    answers[id] = { answers: [value] };
  }
  void respondInteraction(current.value, { answers });
}

function nextQuestion() {
  if (qIndex.value < questionList.value.length - 1) qIndex.value++;
}

function prevQuestion() {
  if (qIndex.value > 0) qIndex.value--;
}

async function handleElicitation() {
  if (!current.value) return;
  const mode = obj(params.value).mode ?? "form";
  if (mode === "url") {
    try {
      await invoke("open_url", { url: str(params.value.url) });
    } catch {
      // ignore
    }
    void respondInteraction(current.value, { action: "approve" });
    return;
  }
  const content = JSON.stringify(formValues);
  void respondInteraction(current.value, {
    action: "approve",
    content: [{ type: "text", text: content }],
  });
}

function schemaProperties(): [string, Record<string, unknown>][] {
  const schema = obj(params.value.requestedSchema);
  return Object.entries(obj(schema.properties)).map(([k, v]) => [k, obj(v)]);
}
</script>

<template>
  <div v-if="current" class="modal-mask">
    <div ref="modalEl" class="modal" tabindex="-1">
      <div class="modal-head">
        <span class="modal-title">
          {{
            isUserInput
              ? "Codex 需要输入"
              : isElicitation
                ? "MCP 工具请求"
                : isCommandApproval
                  ? "批准执行命令"
                  : "批准操作"
          }}
        </span>
        <span v-if="store.interactions.length > 1" class="modal-title" style="font-size: 11px">
          还有 {{ store.interactions.length - 1 }} 个待处理
        </span>
      </div>

      <div class="modal-body">
        <!-- 命令审批 -->
        <template v-if="isCommandApproval">
          <div class="approval-hero">
            <div class="approval-label">将执行命令</div>
            <div class="approval-command">{{ commandText() }}</div>
            <div v-if="params.reason" class="approval-reason">{{ params.reason }}</div>
          </div>
          <details v-if="hasCommandDetails" class="approval-details">
            <summary>详细信息</summary>
            <div class="approval-meta">
              <div v-if="params.cwd">
                <b>工作目录：</b>{{ params.cwd }}
              </div>
              <div v-if="commandActions().length">
                <b>操作：</b>{{ commandActions().join("；") }}
              </div>
              <div v-if="params.additionalPermissions">
                <b>额外权限：</b>{{ permText(params.additionalPermissions) }}
              </div>
            </div>
          </details>
        </template>

        <!-- 其他审批（文件变更/权限） -->
        <template v-else-if="isReviewApproval">
          <div class="approval-hero">
            <div class="approval-label">请求的操作</div>
            <div class="approval-reason">
              {{ params.reason ?? "是否允许此操作？" }}
            </div>
          </div>
          <details v-if="params.grantRoot || params.permissions" class="approval-details">
            <summary>详细信息</summary>
            <div class="approval-meta">
              <div v-if="params.grantRoot">
                <b>授权目录：</b>{{ params.grantRoot }}
              </div>
              <div v-if="params.permissions">
                <b>请求的权限：</b>{{ permText(params.permissions) }}
              </div>
            </div>
          </details>
        </template>

        <!-- 用户输入 -->
        <template v-else-if="isUserInput">
          <div v-if="hasMultipleQuestions" class="question-progress">
            <span>第 {{ qIndex + 1 }} / {{ questionList.length }} 题</span>
            <div class="question-progress-bar">
              <div
                class="question-progress-fill"
                :style="{ width: `${((qIndex + 1) / questionList.length) * 100}%` }"
              ></div>
            </div>
          </div>
          <div v-if="currentQuestion" class="question-row">
            <div class="question-text">
              {{ currentQuestion.header ? `${str(currentQuestion.header)}：` : "" }}{{
                str(currentQuestion.question)
              }}
            </div>
            <div
              v-if="
                Array.isArray(currentQuestion.options) &&
                (currentQuestion.options as unknown[]).length
              "
              class="question-options"
            >
              <button
                v-for="opt in currentQuestion.options as {
                  label: string;
                  description?: string;
                }[]"
                :key="opt.label"
                class="option-btn"
                :class="{ selected: selectedOptions[str(currentQuestion.id)] === opt.label }"
                @click="selectedOptions[str(currentQuestion.id)] = opt.label"
              >
                {{ opt.label }}
              </button>
              <button
                v-if="currentQuestion.isOther"
                class="option-btn"
                :class="{ selected: selectedOptions[str(currentQuestion.id)] === '__other__' }"
                @click="selectedOptions[str(currentQuestion.id)] = '__other__'"
              >
                其他…
              </button>
            </div>
            <input
              v-if="
                selectedOptions[str(currentQuestion.id)] === '__other__' ||
                !Array.isArray(currentQuestion.options) ||
                !(currentQuestion.options as unknown[]).length
              "
              v-model="otherInputs[str(currentQuestion.id)]"
              :type="currentQuestion.isSecret ? 'password' : 'text'"
              placeholder="输入内容"
              style="width: 100%; margin-top: 6px"
            />
          </div>
        </template>

        <!-- MCP elicitation -->
        <template v-else-if="isElicitation">
          <div class="approval-hero">
            <div class="approval-label">来自 {{ params.serverName }} 的请求</div>
            <div v-if="params.message" class="approval-reason">{{ params.message }}</div>
          </div>
          <template v-if="obj(params).mode !== 'url'">
            <div v-for="[key, prop] in schemaProperties()" :key="key" class="question-row">
              <div class="question-text">{{ prop.title ?? key }}</div>
              <input
                v-model="formValues[key]"
                :type="prop.format === 'password' ? 'password' : 'text'"
                style="width: 100%"
                :placeholder="str(prop.description)"
              />
            </div>
          </template>
          <div v-else class="approval-meta" style="margin-top: 6px">
            将在浏览器中打开：{{ params.url }}
          </div>
        </template>
      </div>

      <div class="modal-foot">
        <template v-if="isUserInput">
          <button class="btn" @click="reject()">取消</button>
          <template v-if="hasMultipleQuestions">
            <button class="btn" :disabled="qIndex === 0" @click="prevQuestion()">上一题</button>
            <button v-if="!isLastQuestion" class="btn primary" @click="nextQuestion()">
              下一题
            </button>
            <button v-else class="btn primary" @click="submitUserInput()">提交</button>
          </template>
          <button v-else class="btn primary" @click="submitUserInput()">提交</button>
        </template>
        <template v-else-if="isElicitation">
          <button class="btn" @click="reject()">拒绝</button>
          <button class="btn primary" @click="handleElicitation()">
            {{ obj(params).mode === "url" ? "已在浏览器打开" : "提交" }}
          </button>
        </template>
        <template v-else>
          <button v-if="hasDecision('acceptWithExecpolicyAmendment')" class="btn" @click="approveWithRule()">
            批准并记住此规则
          </button>
          <button v-if="hasDecision('acceptForSession')" class="btn" @click="approveSession()">
            本次会话批准
          </button>
          <button class="btn danger" @click="reject()">拒绝</button>
          <button class="btn primary" @click="approve()">批准</button>
        </template>
      </div>
    </div>
  </div>
</template>
