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
/** 回合收尾强制约束（本地缓冲，保存时写回；缺省开启） */
const nudgeEnabled = ref<boolean>(store.settings.zen_proxy_nudge_enabled ?? true);
/** OpenCode 客户端身份（本地缓冲，保存时写回；缺省开启） */
const identityEnabled = ref<boolean>(
  store.settings.zen_proxy_identity_enabled ?? true,
);
/** 输入校验错误信息 */
const portError = ref<string>("");

/** 本次要应用的选项（端口 / 上游 / 两个行为开关的当前缓冲值） */
function currentOptions(enabled: boolean, port: number) {
  return {
    enabled,
    port,
    baseUrl: apiUrlInput.value.trim() || DEFAULT_BASE_URL,
    nudgeEnabled: nudgeEnabled.value,
    identityEnabled: identityEnabled.value,
  };
}

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
  const ok = await toggleZenProxy(currentOptions(checked, port));
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
  try {
    const st = await applyZenProxy(currentOptions(enabled, port), true);
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
            在「模型配置」中添加模型提供方，字段按下图填写
          </p>

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

      <div class="zen-proxy-config-hint">
        <div class="zen-proxy-config-row">
          <span class="zen-proxy-config-field">base_url</span>
          <span class="zen-proxy-config-value">
            <code class="zen-proxy-config-code zen-proxy-config-base-url">
              {{ localBaseUrlHint }}
            </code>
          </span>
        </div>
        <div class="zen-proxy-config-row">
          <span class="zen-proxy-config-field">experimental_bearer_token</span>
          <span class="zen-proxy-config-value">
            <a
              class="zen-proxy-docs-link"
              :href="ZEN_KEY_DOCS_URL"
              v-tooltip="'获取你的 Zen API Key（浏览器打开）'"
              @click.prevent="openDocsUrl(ZEN_KEY_DOCS_URL)"
            >ApiKey</a>
            或
            <code class="zen-proxy-config-code">public</code>
          </span>
        </div>
        <div class="zen-proxy-config-row">
          <span class="zen-proxy-config-field">model</span>
          <span class="zen-proxy-config-value">
            <a
              class="zen-proxy-docs-link"
              :href="ZEN_PRICING_DOCS_URL"
              v-tooltip="'Zen 定价文档（浏览器打开）'"
              @click.prevent="openDocsUrl(ZEN_PRICING_DOCS_URL)"
            >Zen 免费模型</a>
          </span>
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

        <div class="setting-row zen-proxy-switch-row">
          <label class="zen-proxy-switch-text" for="zen-proxy-nudge">
            回合收尾强制约束（模型空转收尾时自动续跑，直到真的动手做完或干净收尾）
          </label>
          <label class="switch">
            <input id="zen-proxy-nudge" v-model="nudgeEnabled" type="checkbox" />
            <span class="switch-track"></span>
          </label>
        </div>

        <div class="setting-row zen-proxy-switch-row">
          <label class="zen-proxy-switch-text" for="zen-proxy-identity">
            OpenCode 客户端身份（按 OpenCode 客户端形状发送请求头，并对 opencode
            上游补齐免费层门禁字段）
          </label>
          <label class="switch">
            <input id="zen-proxy-identity" v-model="identityEnabled" type="checkbox" />
            <span class="switch-track"></span>
          </label>
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
/* 行为开关行：文字在左、开关在右（与「个性化」分区的开关行一致） */
.zen-proxy-switch-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}
/* 抵消全局 `.setting-row label` 的 block/600 字重与下边距，让文字与开关垂直居中 */
.zen-proxy-switch-row label {
  margin-bottom: 0;
}
.zen-proxy-switch-text {
  flex: 1;
  min-width: 0;
  font-size: var(--font-md);
  font-weight: 400;
  line-height: 1.5;
  color: var(--text);
  user-select: text;
}
.zen-proxy-switch-row .switch {
  flex-shrink: 0;
}
.zen-proxy-actions {
  display: flex;
  justify-content: flex-end;
  padding: var(--space-4) 0 0;
}
.zen-proxy-apply-btn {
  flex-shrink: 0;
}

.zen-proxy-error {
  color: var(--danger);
  font-size: var(--font-xs);
  margin: 0 0 var(--space-2);
}
.zen-proxy-config-hint {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: rgba(var(--overlay-rgb), 0.02);
  padding: var(--space-5) var(--space-8);
  margin-top: var(--space-5);
}

.zen-proxy-config-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}

.zen-proxy-config-field {
  flex: 0 0 168px;
  font-family: var(--mono);
  font-size: var(--font-xs);
  color: var(--text-faint);
  user-select: text;
}

.zen-proxy-config-value {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-2);
  font-size: var(--font-md);
  color: var(--text);
  user-select: text;
}

.zen-proxy-config-code {
  font-family: var(--mono);
  font-size: var(--font-xs);
  line-height: 1.5;
  color: var(--text-bright);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1px var(--space-3);
  word-break: break-all;
  min-width: 0;
}
</style>
