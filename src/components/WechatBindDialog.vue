<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { ThreadSummary } from "../lib/types";
import {
  bindingOfThread,
  refreshWeChatState,
  setToast,
  store,
  toastError,
  wechatBindLoginStart,
  wechatUnbind,
} from "../composables/useCodex";
import { ICON_DELETE } from "../lib/icons";
import ModalDialog from "./ModalDialog.vue";

const props = defineProps<{ thread: ThreadSummary }>();
const emit = defineEmits<{ close: [] }>();

/** 嵌套属性访问保留 Ref 对象（顶层绑定会被模板自动解包） */
const rootRefs = { dialogEl: ref<HTMLElement | null>(null) };
/** 扫码绑定按钮忙碌态（防连点重复下发 login） */
const bindBusy = ref(false);
/** 二维码位图（内容由后端事件推送，本地仅负责转 dataURL 渲染） */
const qrDataUrl = ref("");

const binding = computed(() => bindingOfThread(props.thread.id));
/** 是否正处于本会话的扫码绑定流程（pendingThreadId 匹配） */
const isPending = computed(
  () => store.wechat?.pendingThreadId === props.thread.id,
);
const stateLabel = computed(() => {
  const labels: Record<string, string> = {
    offline: "未连接",
    starting: "启动中…",
    awaiting_qr: "等待扫码",
    connected: "已连接",
    session_expired: "会话过期，请重新扫码",
    error: "异常",
  };
  const conn = binding.value?.connection ?? store.wechat?.connection ?? "offline";
  return labels[conn] ?? "未连接";
});

watch(
  () => store.wechat?.qrContent,
  async (content) => {
    if (!content || !isPending.value) {
      qrDataUrl.value = "";
      return;
    }
    try {
      const QRCode = await import("qrcode");
      qrDataUrl.value = await QRCode.toDataURL(content, { width: 220, margin: 1 });
    } catch {
      qrDataUrl.value = "";
    }
  },
  { immediate: true },
);

async function onBind() {
  if (bindBusy.value || binding.value) return;
  bindBusy.value = true;
  try {
    await wechatBindLoginStart(props.thread.id);
    await refreshWeChatState();
  } catch (e) {
    setToast(toastError(e));
  } finally {
    bindBusy.value = false;
  }
}

async function onUnbind() {
  try {
    await wechatUnbind(props.thread.id);
    qrDataUrl.value = "";
    setToast("已解除微信绑定");
  } catch (e) {
    setToast(toastError(e));
  }
}
</script>

<template>
  <ModalDialog
    :title="`微信接入 · ${thread.name ?? thread.id.slice(0, 8)}`"
    :root-ref="rootRefs.dialogEl"
  >
    <div class="wechat-bind-dialog">
      <template v-if="binding">
        <div class="wechat-status-row">
          <span
            class="wechat-state-badge"
            :class="`st-${binding.connection ?? 'offline'}`"
            >{{ stateLabel }}</span
          >
          <span v-if="store.wechat?.detail" class="wechat-detail">{{
            store.wechat.detail
          }}</span>
          <span v-else-if="binding.accountId" class="wechat-account">
            账号：{{ binding.accountId }}
          </span>
        </div>
        <div class="wechat-bound-note">
          该会话已绑定微信账号，绑定账号本人的消息将路由到此会话执行。
        </div>
        <button type="button" class="btn danger wechat-unbind-btn" @click="onUnbind">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_DELETE" />
          </svg>
          解除绑定
        </button>
      </template>

      <template v-else>
        <p class="wechat-unbound-hint">
          扫描二维码将该微信账号绑定到此会话。绑定后仅该账号本人的消息会触发此会话。
        </p>
        <div v-if="isPending && qrDataUrl" class="wechat-qr-wrap">
          <img :src="qrDataUrl" alt="微信登录二维码" />
          <p class="wechat-qr-hint">
            请使用要绑定的微信扫码并确认授权（二维码约 8 分钟内有效）
          </p>
        </div>
        <div v-else-if="isPending && store.wechat?.detail" class="wechat-detail">
          {{ store.wechat.detail }}
        </div>
        <div v-else-if="isPending" class="wechat-detail">
          正在生成二维码，请稍候…
        </div>
        <div v-else-if="store.wechat?.pendingThreadId" class="wechat-detail">
          已有其他会话正在绑定微信，请稍候或完成后再试。
        </div>
        <button
          type="button"
          class="btn primary wechat-bind-btn"
          :disabled="bindBusy || !!store.wechat?.pendingThreadId"
          @click="onBind"
        >
          {{ bindBusy ? "等待扫码…" : "扫码绑定" }}
        </button>
      </template>
    </div>
    <template #foot>
      <button class="btn" @click="emit('close')">关闭</button>
    </template>
  </ModalDialog>
</template>
