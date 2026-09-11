<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
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
import { openDocsUrl } from "../../lib/links";

defineProps<{ active: boolean }>();

const DEFAULT_PORT = 18080;
const DEFAULT_BASE_URL = "https://opencode.ai/zen/v1";
const ZEN_PRICING_DOCS_URL =
  "https://open-code.ai/zh/docs/zen#%E5%AE%9A%E4%BB%B7";
const ZEN_KEY_DOCS_URL = "https://opencode.ai/zen";

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

/**
 * 本地 provider 的 base_url 提示：本机回环地址 + 模型提供方 base_url 的路径
 * （codex 请求 `{base_url}/responses`，两侧路径必须一致，只换 host）。
 */
const localBaseUrlHint = computed(() => {
  const raw = apiUrlInput.value.trim() || DEFAULT_BASE_URL;
  let path = "";
  try {
    path = new URL(raw).pathname.replace(/\/+$/, "");
  } catch {
    path = "";
  }
  return `http://127.0.0.1:${store.settings.zen_proxy_port ?? DEFAULT_PORT}${path}`;
});

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

/** 保存端口 + API 地址变更（保存设置并重启代理） */
async function onSave() {
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
    <h2 class="settings-section-title">Zen 代理</h2>
    <p class="settings-section-desc">
      在本地开放一个 Responses API 端点，把请求翻译为 Chat Completions 转发到 OpenCode
      Zen 免费模型
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <div class="zen-proxy-head-main">
          <h3>Zen 本地代理服务</h3>
          <p class="zen-proxy-head-desc">
            model 为
            <a
              class="zen-proxy-docs-link"
              :href="ZEN_PRICING_DOCS_URL"
              v-tooltip="'Zen 定价文档（浏览器打开）'"
              @click.prevent="openDocsUrl(ZEN_PRICING_DOCS_URL)"
            >Zen 免费模型</a>，本地 provider 的 base_url 为
            {{ localBaseUrlHint }}，experimental_bearer_token 为 public 或 <a
            class="zen-proxy-docs-link"
            :href="ZEN_KEY_DOCS_URL"
            v-tooltip="'获取你的 Zen API Key（浏览器打开）'"
            @click.prevent="openDocsUrl(ZEN_KEY_DOCS_URL)"
          >你自己的 key</a>
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
          <label>模型提供方的 base_url</label>
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
        <button class="btn zen-proxy-apply-btn" @click="onSave">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="ICON_SAVE" />
          </svg>
          <span>保存</span>
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
  -webkit-user-select: text;
  user-select: text;
}
.zen-proxy-docs-link {
  color: var(--accent);
  text-decoration: none;
  cursor: pointer;
  user-select: text;
}
.zen-proxy-docs-link:hover {
  color: var(--accent-dim);
  text-decoration: underline;
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
  border: 1px solid transparent;
}
.zen-proxy-status.is-running {
  color: var(--accent);
  background: var(--accent-soft);
}
.zen-proxy-status.is-stopped {
  color: var(--text-dim);
  background: var(--bg-input);
  border-color: var(--border);
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
