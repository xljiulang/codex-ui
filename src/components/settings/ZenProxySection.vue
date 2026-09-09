<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import {
  applyZenProxy,
  readZenProxyStatus,
  setToast,
  store,
  toggleZenProxy,
  toastError,
  type ZenProxyStatus,
} from "../../composables/useCodex";
import { ICON_WINDOW } from "../../lib/icons";

defineProps<{ active: boolean }>();

const DEFAULT_PORT = 18080;

/** 本地代理运行状态（由 zen_proxy_status 驱动；未获取时默认停止） */
const status = ref<ZenProxyStatus>({ running: false, port: DEFAULT_PORT });
/** 端口输入（本地缓冲，保存时写回） */
const portInput = ref<string>(String(store.settings.zen_proxy_port ?? DEFAULT_PORT));
/** 输入校验错误信息 */
const portError = ref<string>("");

function validatePort(v: string): number | null {
  const n = Number(v.trim());
  if (!Number.isInteger(n) || n < 1024 || n > 65535) {
    portError.value = "端口需为 1024–65535 的整数";
    return null;
  }
  portError.value = "";
  return n;
}

async function refresh() {
  try {
    const s = await readZenProxyStatus();
    status.value = s ?? { running: false, port: DEFAULT_PORT };
  } catch {
    status.value = { running: false, port: DEFAULT_PORT };
  }
}

/** 切换开关：开启/关闭代理（写 settings.json 并启停服务） */
async function onToggle(checked: boolean) {
  const port = validatePort(portInput.value);
  if (!port) {
    setToast("端口无效，请检查输入");
    return;
  }
  const ok = await toggleZenProxy(checked, port);
  if (ok !== checked) {
    status.value = await readZenProxyStatus().catch(() => status.value);
  } else {
    status.value = { running: checked, port, error: null };
  }
  await refresh();
}

/** 应用端口变更（仅影响运行时，保存设置即重启代理） */
async function onApplyPort() {
  const port = validatePort(portInput.value);
  if (!port) return;
  const enabled = store.settings.zen_proxy_enabled ?? false;
  try {
    const st = await applyZenProxy(enabled, port, true);
    if (enabled) {
      setToast(st.error ?? `代理已重启（端口 ${st.port}）`);
    } else {
      setToast(`端口已设为 ${port}`);
    }
  } catch (e) {
    setToast(toastError(e));
  }
  await refresh();
}

onMounted(refresh);
onBeforeUnmount(refresh);
</script>

<template>
  <section v-show="active" class="settings-section settings-section-zen-proxy">
    <h2 class="settings-section-title">Zen 本地代理</h2>
    <p class="settings-section-desc">
      在本地开放一个 Responses API 端点，把请求翻译为 Chat Completions 转发到 OpenCode
      Zen 免费模型，供 codex 使用（无需 wire_api="chat"）
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>Zen 代理服务</h3>
        <div class="model-config-head-actions">
          <span
            class="zen-proxy-status"
            :class="status.running ? 'is-running' : 'is-stopped'"
          >
            {{ status.running ? `运行中（端口 ${status.port}）` : "已停止" }}
          </span>
        </div>
      </div>

      <div class="zen-proxy-row">
        <label class="switch" aria-label="Zen 本地代理开关">
          <input
            type="checkbox"
            :checked="store.settings.zen_proxy_enabled ?? false"
            @change="onToggle(($event.target as HTMLInputElement).checked)"
          />
          <span class="switch-track"></span>
        </label>
        <span class="zen-proxy-row-label">启用 Zen 本地代理</span>
      </div>

      <div class="zen-proxy-row">
        <span class="zen-proxy-row-label">监听端口</span>
        <input
          type="number"
          class="zen-proxy-port-input"
          :value="portInput"
          min="1024"
          max="65535"
          @input="portInput = ($event.target as HTMLInputElement).value"
        />
        <button class="btn zen-proxy-apply-btn" @click="onApplyPort">
          应用端口
        </button>
      </div>
      <p v-if="portError" class="zen-proxy-error">{{ portError }}</p>

      <div class="zen-proxy-hint">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path :d="ICON_WINDOW" fill="currentColor" />
        </svg>
        <span class="zen-proxy-hint-body">
          在「模型配置」页手动添加 provider，按下面填写（key 可用
          <code>public</code>）：
          <br />
          <code>base_url = http://127.0.0.1:{{
            store.settings.zen_proxy_port ?? DEFAULT_PORT
          }}/v1</code>
          <br />
          <code>wire_api = "responses"</code>
          <br />
          <code>experimental_bearer_token = "public"</code>
          <br />
          <span class="zen-proxy-hint-note">
            模型名填 Zen 支持的 ID，代理会原样透传。
          </span>
        </span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.zen-proxy-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) 0;
  border-bottom: 1px solid var(--border);
}
.zen-proxy-row:last-of-type {
  border-bottom: none;
}
.zen-proxy-row-label {
  font-size: var(--font-md);
  color: var(--text-bright);
  flex-shrink: 0;
}
.zen-proxy-port-input {
  width: 120px;
  height: var(--ctrl-h-md);
  padding: var(--space-2) var(--space-4);
}
.zen-proxy-apply-btn {
  margin-left: auto;
}
.zen-proxy-status {
  font-size: var(--font-xs);
  font-weight: 600;
  padding: 2px var(--space-3);
  border-radius: 999px;
}
.zen-proxy-status.is-running {
  color: var(--ok);
  background: rgba(var(--ok-rgb, 76, 175, 80), 0.12);
}
.zen-proxy-status.is-stopped {
  color: var(--text-faint);
  background: var(--bg-2);
}
.zen-proxy-error {
  color: var(--danger);
  font-size: var(--font-xs);
  margin: 0 0 var(--space-2);
}
.zen-proxy-hint {
  display: flex;
  gap: var(--space-2);
  align-items: flex-start;
  font-size: var(--font-xs);
  color: var(--text-faint);
  background: var(--bg-2);
  border-radius: var(--radius-md);
  padding: var(--space-3) var(--space-4);
  line-height: 1.6;
}
.zen-proxy-hint svg {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  margin-top: 2px;
}
.zen-proxy-hint code {
  background: var(--float-bg);
  border-radius: 4px;
  padding: 0 4px;
}
</style>
