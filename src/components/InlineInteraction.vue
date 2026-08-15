<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { respondInteraction, store } from "../composables/useCodex";
import { focusComposer } from "../lib/composerFocus";
import { list, obj, permText, str } from "../lib/interaction";
import type { PendingInteraction } from "../lib/types";
import InteractionQuestionForm from "./InteractionQuestionForm.vue";
import InteractionSchemaForm from "./InteractionSchemaForm.vue";

const props = withDefaults(
  defineProps<{ interactions?: PendingInteraction[] }>(),
  { interactions: () => store.interactions },
);
const current = computed<PendingInteraction | undefined>(
  () => props.interactions[0],
);
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

watch(current, () => {
  if (current.value) {
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

/** 提问表单提交：按协议回传 { answers } */
function onQuestionSubmit(answers: Record<string, { answers: string[] }>) {
  if (!current.value) return;
  void respondInteraction(current.value, { answers });
}

/** MCP 表单提交：action accept + 结构化 content（url 模式为 null） */
function onElicitationSubmit(content: Record<string, unknown> | null) {
  if (!current.value) return;
  void respondInteraction(current.value, { action: "accept", content });
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
        <span v-if="interactions.length > 1" class="interaction-pending">
          还有 {{ interactions.length - 1 }} 个待处理
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

        <!-- 用户输入（分步提问，子组件持有选项/其他输入状态） -->
        <InteractionQuestionForm
          v-else-if="isUserInput"
          :key="current.requestId"
          :interaction="current"
          @submit="onQuestionSubmit"
          @cancel="reject()"
        />

        <!-- MCP elicitation（子组件持有 schema 表单状态） -->
        <InteractionSchemaForm
          v-else-if="isElicitation"
          :key="current.requestId"
          :interaction="current"
          @submit="onElicitationSubmit"
          @cancel="reject()"
        />
      </div>

      <div v-if="!isUserInput && !isElicitation" class="interaction-foot">
        <button v-if="hasDecision('acceptWithExecpolicyAmendment')" class="btn" @click="approveWithRule()">
          批准并记住此规则
        </button>
        <button v-if="hasDecision('acceptForSession')" class="btn" @click="approveSession()">
          本次会话批准
        </button>
        <button class="btn danger" @click="reject()">拒绝</button>
        <button class="btn primary" @click="approve()">批准</button>
      </div>
    </div>
  </div>
</template>
