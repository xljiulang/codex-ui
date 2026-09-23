<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  applyCompatProxy,
  readCompatProxyStatus,
  setToast,
  store,
  toggleCompatProxy,
  toastError,
  type CompatProxyStatus,
} from "../../composables/useCodex";
import { ICON_SAVE } from "../../lib/icons";
import { copyText } from "../../lib/clipboard";
import { openDocsUrl } from "../../lib/links";

defineProps<{ active: boolean }>();

const DEFAULT_PORT = 18080;
const DEFAULT_BASE_URL = "https://opencode.ai/zen/v1";
/** 顶部说明里「Zen 免费模型」的文档地址（浏览器打开） */
const ZEN_DOCS_URL = "https://opencode.ai/zen";

/** 本地代理运行状态（由 compat_proxy_status 驱动；未获取时默认停止） */
const status = ref<CompatProxyStatus>({ running: false, port: DEFAULT_PORT });
/** 端口输入（本地缓冲，保存时写回） */
const portInput = ref<string>(
  String(store.settings.compat_proxy_port ?? DEFAULT_PORT),
);
/** API 请求地址输入（本地缓冲，保存时写回；空回退默认） */
const apiUrlInput = ref<string>(
  store.settings.compat_proxy_base_url ?? DEFAULT_BASE_URL,
);
/** 回合收尾约束和助推（本地缓冲，保存时写回；缺省开启） */
const nudgeEnabled = ref<boolean>(
  store.settings.compat_proxy_nudge_enabled ?? true,
);
/** OpenCode 客户端身份（本地缓冲，保存时写回；缺省开启） */
const identityEnabled = ref<boolean>(
  store.settings.compat_proxy_identity_enabled ?? true,
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
  return `http://127.0.0.1:${store.settings.compat_proxy_port ?? DEFAULT_PORT}${path}`;
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
    const s = await readCompatProxyStatus();
    status.value = s ?? { running: false, port: DEFAULT_PORT };
  } catch {
    status.value = { running: false, port: DEFAULT_PORT };
  }
}

/** 点击胶囊：把本机 base_url 复制到剪贴板并提示（失败按既有范式提示手动复制） */
async function copyBaseUrl() {
  const ok = await copyText(localBaseUrlHint.value);
  setToast(ok ? "已复制 base_url" : "复制失败，请手动选择复制");
}

/** 切换开关：开启/关闭代理（写 settings.json 并启停服务） */
async function onToggle(checked: boolean) {
  const port = validatePort(portInput.value);
  if (!port) {
    setToast("端口无效，请检查输入");
    return;
  }
  const ok = await toggleCompatProxy(currentOptions(checked, port));
  if (ok !== checked) {
    status.value = await readCompatProxyStatus().catch(() => status.value);
  } else {
    status.value = { running: checked, port, error: null };
  }
  await refresh();
}

/** 保存端口 + API 地址变更（保存设置并重启代理） */
async function onSave() {
  const port = validatePort(portInput.value);
  if (!port) return;
  const enabled = store.settings.compat_proxy_enabled ?? false;
  try {
    const st = await applyCompatProxy(currentOptions(enabled, port), true);
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
  <section
    v-show="active"
    class="settings-section settings-section-compat-proxy"
  >
    <h2 class="settings-section-title">兼容代理</h2>
    <p class="settings-section-desc">
      在本地开放一个 Responses API 端点，把请求翻译为 Chat Completions
      转发到任意 OpenAI 兼容上游（默认 OpenCode
      <a
        class="compat-proxy-docs-link"
        :href="ZEN_DOCS_URL"
        v-tooltip="'Zen 免费模型文档（浏览器打开）'"
        @click.prevent="openDocsUrl(ZEN_DOCS_URL)"
        >Zen 免费模型</a
      >）
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <div class="compat-proxy-head-main">
          <h3>兼容代理服务</h3>
          <div class="compat-proxy-head-status">
            <span
              class="compat-proxy-status-badge"
              :class="
                (store.settings.compat_proxy_enabled ?? false)
                  ? 'is-running'
                  : 'is-stopped'
              "
            >
              {{
                (store.settings.compat_proxy_enabled ?? false)
                  ? "已启动"
                  : "已停止"
              }}
            </span>
            <code
              v-if="store.settings.compat_proxy_enabled ?? false"
              class="compat-proxy-base-url-capsule"
              role="button"
              tabindex="0"
              :aria-label="`复制 base_url：${localBaseUrlHint}`"
              @click="copyBaseUrl()"
              @keydown.enter.prevent="copyBaseUrl()"
              @keydown.space.prevent="copyBaseUrl()"
            >
              {{ localBaseUrlHint }}
            </code>
          </div>
        </div>
        <div class="model-config-head-actions">
          <label class="switch" aria-label="兼容代理开关">
            <input
              type="checkbox"
              :checked="store.settings.compat_proxy_enabled ?? false"
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
            class="compat-proxy-port-input"
            :value="portInput"
            min="1024"
            max="65535"
            @input="portInput = ($event.target as HTMLInputElement).value"
          />
          <p v-if="portError" class="compat-proxy-error">{{ portError }}</p>
        </div>

        <div class="setting-row">
          <label>模型提供方的 base_url</label>
          <input
            type="text"
            class="compat-proxy-url-input"
            :value="apiUrlInput"
            placeholder="https://opencode.ai/zen/v1"
            @input="apiUrlInput = ($event.target as HTMLInputElement).value"
          />
        </div>

        <div class="setting-row">
          <div class="checkbox-row compat-proxy-checkbox-row">
            <input
              id="compat-proxy-nudge"
              v-model="nudgeEnabled"
              type="checkbox"
            />
            <label for="compat-proxy-nudge">
              回合收尾约束和助推（模型空转收尾时自动续跑）
            </label>
          </div>
        </div>

        <div class="setting-row">
          <div class="checkbox-row compat-proxy-checkbox-row">
            <input
              id="compat-proxy-identity"
              v-model="identityEnabled"
              type="checkbox"
            />
            <label for="compat-proxy-identity">
              OpenCode 客户端身份（补齐与 OpenCode 一致的请求头和工具集）
            </label>
          </div>
        </div>
      </div>

      <div class="compat-proxy-actions">
        <button class="btn compat-proxy-apply-btn" @click="onSave">
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
.compat-proxy-head-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
/* 卡片头状态行：状态徽章常驻，base_url 胶囊仅在启用后出现 */
.compat-proxy-head-status {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-2);
  min-width: 0;
}
.compat-proxy-status-badge {
  padding: 2px var(--space-3);
  border-radius: 999px;
  font-size: var(--font-xs);
  font-weight: 600;
  line-height: 1.4;
  border: 1px solid transparent;
  white-space: nowrap;
  flex-shrink: 0;
}
.compat-proxy-status-badge.is-running {
  color: var(--accent);
  background: var(--accent-soft);
}
.compat-proxy-status-badge.is-stopped {
  color: var(--text-faint);
  background: var(--bg-input);
  border-color: var(--border);
}
/* base_url 值胶囊：可点击复制（鼠标手型 + 悬停高亮 + 键盘可达） */
.compat-proxy-base-url-capsule {
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
  cursor: pointer;
  -webkit-user-select: text;
  user-select: text;
  transition:
    border-color var(--ease),
    color var(--ease);
}
.compat-proxy-base-url-capsule:hover {
  border-color: var(--accent-dim);
  color: var(--accent);
}
.compat-proxy-base-url-capsule:focus-visible {
  outline: none;
  border-color: var(--accent-dim);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.12);
}
.compat-proxy-docs-link {
  color: var(--accent);
  text-decoration: none;
  cursor: pointer;
  user-select: text;
}
.compat-proxy-docs-link:hover {
  color: var(--accent-dim);
  text-decoration: underline;
}
.compat-proxy-port-input {
  width: 100%;
  height: var(--ctrl-h-md);
  padding: var(--space-2) var(--space-4);
}
.compat-proxy-url-input {
  width: 100%;
  height: var(--ctrl-h-md);
  padding: var(--space-2) var(--space-4);
}
/* 行为开关行：复选框在左、说明在后（复用全局 `.checkbox-row`，毛玻璃态自动跟上）；
   文案会换行，因此复选框与首行对齐而不是整行居中 */
.compat-proxy-checkbox-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}
/* 抵消全局 `.setting-row label` 的 block/600 字重/--text-dim 与下边距 */
.compat-proxy-checkbox-row label {
  margin-bottom: 0;
  font-size: var(--font-md);
  font-weight: 400;
  line-height: 1.5;
  color: var(--text);
  user-select: text;
}
/* 18px 行高对 16px 方框补 1px，使方框与文字首行垂直居中对齐 */
.compat-proxy-checkbox-row input[type="checkbox"] {
  accent-color: var(--accent);
  width: 16px;
  height: 16px;
  margin: 1px 0 0;
  flex-shrink: 0;
  cursor: pointer;
}
.compat-proxy-actions {
  display: flex;
  justify-content: flex-end;
  padding: var(--space-4) 0 0;
}
.compat-proxy-apply-btn {
  flex-shrink: 0;
}

.compat-proxy-error {
  color: var(--danger);
  font-size: var(--font-xs);
  margin: 0 0 var(--space-2);
}
</style>
