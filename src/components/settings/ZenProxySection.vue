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
import { ICON_SAVE } from "../../lib/icons";

defineProps<{ active: boolean }>();

const DEFAULT_PORT = 18080;
const DEFAULT_BASE_URL = "https://opencode.ai/zen/v1";

/** 本地代理运行状态（由 zen_proxy_status 驱动；未获取时默认停止） */
const status = ref<ZenProxyStatus>({ running: false, port: DEFAULT_PORT });
/** 端口输入（本地缓冲，保存时写回） */
const portInput = ref<string>(String(store.settings.zen_proxy_port ?? DEFAULT_PORT));
/** API 请求地址输入（本地缓冲，保存时写回；空回退默认） */
const apiUrlInput = ref<string>(
  store.settings.zen_proxy_base_url ?? DEFAULT_BASE_URL,
);
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
  const baseUrl = apiUrlInput.value.trim() || DEFAULT_BASE_URL;
  const ok = await toggleZenProxy(checked, port, baseUrl);
  if (ok !== checked) {
    status.value = await readZenProxyStatus().catch(() => status.value);
  } else {
    status.value = { running: checked, port, error: null };
  }
  await refresh();
}

/** 应用端口 + API 地址变更（保存设置并重启代理） */
async function onApply() {
  const port = validatePort(portInput.value);
  if (!port) return;
  const enabled = store.settings.zen_proxy_enabled ?? false;
  const baseUrl = apiUrlInput.value.trim() || DEFAULT_BASE_URL;
  try {
    const st = await applyZenProxy(enabled, port, baseUrl, true);
    if (enabled) {
      setToast(st.error ?? `代理已重启（端口 ${st.port}）`);
    } else {
      setToast(`设置已保存（端口 ${port}）`);
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
      Zen 免费模型
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <div class="zen-proxy-head-main">
          <h3>Zen 代理服务</h3>
          <p class="zen-proxy-head-desc">
            model 为 zen 免费模型，base_url 为 http://127.0.0.1:{{
              store.settings.zen_proxy_port ?? DEFAULT_PORT
            }}/v1，experimental_bearer_token 为 public 或你自己的 key
          </p>
          <div class="zen-proxy-badges">
            <span
              class="zen-proxy-status"
              :class="status.running ? 'is-running' : 'is-stopped'"
            >
              {{ status.running ? "运行中" : "已停止" }}
            </span>
            <span class="zen-proxy-port-badge">{{ status.port }}</span>
          </div>
        </div>
        <div class="model-config-head-actions">
          <label class="switch" aria-label="Zen 本地代理开关">
            <input
              type="checkbox"
              :checked="store.settings.zen_proxy_enabled ?? false"
              @change="onToggle(($event.target as HTMLInputElement).checked)"
            />
            <span class="switch-track"></span>
          </label>
        </div>
      </div>

      <div class="model-provider-form">
        <div class="setting-row">
          <label>监听回环端口</label>
          <input
            type="number"
            class="zen-proxy-port-input"
            :value="portInput"
            min="1024"
            max="65535"
            @input="portInput = ($event.target as HTMLInputElement).value"
          />
          <p v-if="portError" class="zen-proxy-error">{{ portError }}</p>
        </div>

        <div class="setting-row">
          <label>转发目标地址</label>
          <input
            type="text"
            class="zen-proxy-url-input"
            :value="apiUrlInput"
            placeholder="https://opencode.ai/zen/v1"
            @input="
              apiUrlInput = ($event.target as HTMLInputElement).value
            "
          />
        </div>
      </div>

      <div class="zen-proxy-actions">
        <button class="btn zen-proxy-apply-btn" @click="onApply">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_SAVE" />
          </svg>
          <span>应用</span>
        </button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.model-config-card-head {
  align-items: center;
}
.zen-proxy-head-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.zen-proxy-head-desc {
  margin: 0;
  font-size: var(--font-md);
  line-height: 1.5;
  color: var(--text-dim);
}
.zen-proxy-badges {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.zen-proxy-port-input {
  width: 100%;
  height: var(--ctrl-h-md);
  padding: var(--space-2) var(--space-4);
}
.zen-proxy-url-input {
  width: 100%;
  height: var(--ctrl-h-md);
  padding: var(--space-2) var(--space-4);
}
.zen-proxy-actions {
  display: flex;
  justify-content: flex-end;
  padding: var(--space-4) 0 0;
}
.zen-proxy-apply-btn {
  flex-shrink: 0;
}
.zen-proxy-status {
  font-size: var(--font-xs);
  font-weight: 600;
  line-height: 1;
  border-radius: 999px;
  flex-shrink: 0;
  padding: 1px var(--space-2);
}
.zen-proxy-status.is-running {
  color: var(--accent);
  background: var(--accent-soft);
}
.zen-proxy-status.is-stopped {
  color: var(--text-faint);
  background: var(--bg-2);
}
.zen-proxy-port-badge {
  font-size: var(--font-xs);
  font-weight: 600;
  line-height: 1;
  border-radius: 999px;
  flex-shrink: 0;
  padding: 1px var(--space-2);
  color: var(--text-dim);
  background: var(--bg-input);
  border: 1px solid var(--border);
}
.zen-proxy-error {
  color: var(--danger);
  font-size: var(--font-xs);
  margin: 0 0 var(--space-2);
}
</style>
