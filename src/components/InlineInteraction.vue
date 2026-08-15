<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { call } from "../lib/ipc";
import { respondInteraction, setToast, store } from "../composables/useCodex";
import { focusComposer } from "../lib/composerFocus";
import type { PendingInteraction } from "../lib/types";

const current = computed<PendingInteraction | undefined>(() => store.interactions[0]);
const params = computed(() => (current.value?.params ?? {}) as Record<string, unknown>);
const bubbleEl = ref<HTMLElement | null>(null);

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
const isPermissionsApproval = computed(
  () => current.value?.method === "item/permissions/requestApproval",
);
const hasCommandDetails = computed(
  () =>
    Boolean(params.value.cwd) ||
    commandActions().length > 0 ||
    Boolean(params.value.additionalPermissions),
);

const selectedOptions = reactive<Record<string, string>>({});
const otherInputs = reactive<Record<string, string>>({});
const formValues = reactive<Record<string, string>>({});
/** 多选 enum（schema type=array）的选中项 */
const formMulti = reactive<Record<string, string[]>>({});

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
  for (const k of Object.keys(formMulti)) delete formMulti[k];
  qIndex.value = 0;
  if (current.value) {
    // 用 schema 的 default 预填表单
    for (const [key, prop] of schemaProperties()) {
      if (isMultiEnum(prop)) {
        formMulti[key] = list(prop.default).map(str);
      } else if (prop.type === "boolean") {
        if (typeof prop.default === "boolean") {
          formValues[key] = prop.default ? "true" : "false";
        }
      } else if (prop.type === "number" || prop.type === "integer") {
        if (typeof prop.default === "number") formValues[key] = String(prop.default);
      } else if (typeof prop.default === "string") {
        formValues[key] = prop.default;
      }
    }
    // 打开时把焦点移入气泡（首个主按钮）
    void nextTick(() => {
      const primary = bubbleEl.value?.querySelector<HTMLElement>(
        ".interaction-foot .btn.primary",
      );
      (primary ?? bubbleEl.value)?.focus();
    });
  } else {
    // 交互解决后把焦点还给输入框
    focusComposer();
  }
});

function onKeydown(e: KeyboardEvent) {
  // Escape：仅对提问/表单类执行“取消”，审批必须明确选择
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
  if (isPermissionsApproval.value) {
    // 协议：权限应答需回传被授予的权限子集与作用域；全量批准即原样回传请求的 permissions
    void respondInteraction(current.value, {
      permissions: params.value.permissions ?? {},
      scope: "turn",
    });
    return;
  }
  void respondInteraction(
    current.value,
    isV2Approval.value ? { decision: "accept" } : { decision: "approved" },
  );
}

function approveSession() {
  if (!current.value) return;
  if (isPermissionsApproval.value) {
    void respondInteraction(current.value, {
      permissions: params.value.permissions ?? {},
      scope: "session",
    });
    return;
  }
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
  if (isElicitation.value) {
    // 协议：elicitation 应答为 { action: "decline"|"cancel", content: null }
    void respondInteraction(current.value, { action: "decline", content: null });
    return;
  }
  if (isPermissionsApproval.value) {
    // 协议：权限应答没有 decline 语义，返回空授予子集即全部拒绝
    void respondInteraction(current.value, { permissions: {}, scope: "turn" });
    return;
  }
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
      await call("open_url", { url: str(params.value.url) });
    } catch {
      // ignore
    }
    // url 模式：无表单字段，接受时返回空内容
    void respondInteraction(current.value, { action: "accept", content: null });
    return;
  }
  const missing = missingRequired();
  if (missing) {
    setToast(`请填写必填字段：${missing}`);
    return;
  }
  // 协议：action 仅 accept/decline/cancel，content 为结构化 JSON 值（表单字段对象）
  void respondInteraction(current.value, {
    action: "accept",
    content: buildFormContent(),
  });
}

function schemaProperties(): [string, Record<string, unknown>][] {
  const schema = obj(params.value.requestedSchema);
  return Object.entries(obj(schema.properties)).map(([k, v]) => [k, obj(v)]);
}

function requiredFields(): string[] {
  return list(obj(obj(params.value.requestedSchema).required)).map(str);
}

function isRequired(key: string): boolean {
  return requiredFields().includes(key);
}

/** 单选/多选 enum schema：oneOf/anyOf（const+title）或 enum（+enumNames） */
function isEnumSchema(s: Record<string, unknown>): boolean {
  return (
    Array.isArray(s.oneOf) ||
    Array.isArray(s.anyOf) ||
    Array.isArray(s.enum)
  );
}

function isMultiEnum(s: Record<string, unknown>): boolean {
  return s.type === "array" && isEnumSchema(obj(s.items));
}

function enumOptions(s: Record<string, unknown>): { value: string; title: string }[] {
  const oneOf = list(s.oneOf).map(obj).filter((o) => o.const !== undefined);
  if (oneOf.length) {
    return oneOf.map((o) => ({
      value: str(o.const),
      title: str(o.title ?? o.const),
    }));
  }
  const anyOf = list(s.anyOf).map(obj).filter((o) => o.const !== undefined);
  if (anyOf.length) {
    return anyOf.map((o) => ({
      value: str(o.const),
      title: str(o.title ?? o.const),
    }));
  }
  const enums = list(s.enum).map(str);
  const names = list(s.enumNames).map(str);
  return enums.map((v, i) => ({ value: v, title: names[i] || v }));
}

/** 缺失的必填字段标题；全部填齐返回 null */
function missingRequired(): string | null {
  const schema = obj(params.value.requestedSchema);
  const props = obj(schema.properties);
  for (const key of requiredFields()) {
    const prop = obj(props[key]);
    if (Object.keys(prop).length === 0) return key;
    if (isMultiEnum(prop)) {
      if (!(formMulti[key] ?? []).length) return str(prop.title) || key;
      continue;
    }
    if (prop.type === "boolean") continue; // 布尔始终有值（默认 false）
    if (prop.type === "number" || prop.type === "integer") {
      if ((formValues[key] ?? "") === "") return str(prop.title) || key;
      continue;
    }
    if (!(formValues[key] ?? "").trim()) return str(prop.title) || key;
  }
  return null;
}

/** 按 schema 类型把表单值转成结构化 content：数字/布尔保留类型，可选空字段省略 */
function buildFormContent(): Record<string, unknown> {
  const content: Record<string, unknown> = {};
  for (const [key, prop] of schemaProperties()) {
    const raw = formValues[key] ?? "";
    if (isMultiEnum(prop)) {
      const sel = formMulti[key] ?? [];
      const def = list(prop.default).map(str);
      const v = sel.length ? sel : def;
      if (v.length) content[key] = v;
      continue;
    }
    if (prop.type === "boolean") {
      const def = typeof prop.default === "boolean" ? prop.default : false;
      content[key] = raw === "" ? def : raw === "true";
      continue;
    }
    if (prop.type === "number" || prop.type === "integer") {
      if (raw !== "") content[key] = Number(raw);
      else if (typeof prop.default === "number") content[key] = prop.default;
      continue;
    }
    // string / 单选 enum
    if (raw !== "") content[key] = raw;
    else if (typeof prop.default === "string") content[key] = prop.default;
  }
  return content;
}
</script>

<template>
  <div v-if="current" class="msg msg-agent">
    <div ref="bubbleEl" class="interaction-bubble" tabindex="-1">
      <div class="interaction-head">
        <span class="interaction-title">
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
        <span v-if="store.interactions.length > 1" class="interaction-pending">
          还有 {{ store.interactions.length - 1 }} 个待处理
        </span>
      </div>

      <div class="interaction-body">
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
              <div class="question-text">
                {{ prop.title ?? key }}
                <span v-if="isRequired(key)" class="required-mark"> *</span>
              </div>
              <select
                v-if="isEnumSchema(prop)"
                v-model="formValues[key]"
                :placeholder="str(prop.description)"
              >
                <option value="" disabled>请选择</option>
                <option v-for="opt in enumOptions(prop)" :key="opt.value" :value="opt.value">
                  {{ opt.title }}
                </option>
              </select>
              <div v-else-if="isMultiEnum(prop)" class="multi-options">
                <label
                  v-for="opt in enumOptions(obj(prop.items))"
                  :key="opt.value"
                  class="option-btn"
                >
                  <input type="checkbox" :value="opt.value" v-model="formMulti[key]" />
                  {{ opt.title }}
                </label>
              </div>
              <select v-else-if="prop.type === 'boolean'" v-model="formValues[key]">
                <option value="true">是</option>
                <option value="false">否</option>
              </select>
              <input
                v-else-if="prop.type === 'number' || prop.type === 'integer'"
                v-model="formValues[key]"
                type="number"
                :placeholder="str(prop.description)"
              />
              <input
                v-else
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

      <div class="interaction-foot">
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
