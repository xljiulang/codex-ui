<script setup lang="ts">
import { onBeforeUnmount, reactive, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import ModalDialog from "../ModalDialog.vue";
import {
  loadMcpServerStatus,
  setToast,
  toastError,
} from "../../composables/useCodex";
import { copyText } from "../../lib/clipboard";
import type {
  McpAuthStatus,
  McpServerDetail,
  McpServerInfo,
  McpToolDetail,
} from "../../lib/types";

/** 详情弹窗目标服务器：null 表示关闭（父层 v-if 控制挂载/卸载） */
const props = defineProps<{ server: McpServerInfo | null }>();
const emit = defineEmits<{ close: [] }>();

const MCP_DETAIL_TABS = [
  { id: "info", label: "服务器信息" },
  { id: "tools", label: "工具" },
  { id: "resources", label: "资源" },
] as const;
type McpDetailTabId = (typeof MCP_DETAIL_TABS)[number]["id"];

const MCP_AUTH_LABELS: Record<McpAuthStatus, string> = {
  unknown: "认证状态未知",
  unsupported: "未支持认证",
  notLoggedIn: "未登录",
  bearerToken: "Bearer Token",
  oAuth: "OAuth",
};

/** MCP 详情弹窗状态：不落全局 store，随弹窗关闭一并销毁 */
const mcpDetail = reactive({
  name: "",
  transport: "" as "" | "stdio" | "http",
  command: "",
  argsText: "",
  cwd: "",
  url: "",
  loading: false,
  notFound: false,
  detail: null as McpServerDetail | null,
  startup: "" as "" | "starting" | "ready" | "failed" | "cancelled",
  activeTab: "info" as McpDetailTabId,
});

/** mcpServer/startupStatus/updated 的注销函数（弹窗打开期间仅保留一个） */
let mcpDetailUnlisten: (() => void) | null = null;

async function stopMcpDetailListen() {
  const unlisten = mcpDetailUnlisten;
  mcpDetailUnlisten = null;
  if (!unlisten) return;
  try {
    await unlisten();
  } catch {
    // 注销失败可忽略（下次打开会重新监听）
  }
}

/** 重新拉取当前详情；加载/打开失败时保持弹窗并 toast。 */
async function refreshMcpDetail() {
  if (!props.server || mcpDetail.loading) return;
  mcpDetail.loading = true;
  mcpDetail.notFound = false;
  try {
    const detail = await loadMcpServerStatus(mcpDetail.name);
    mcpDetail.detail = detail;
    mcpDetail.notFound = detail === null;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    mcpDetail.loading = false;
  }
}

/** 处理服务器启动状态通知：仅用于 ready 后自动重拉一次能力清单 */
function onMcpStartupUpdated(payload: {
  name?: string;
  status?: string;
}) {
  if (!payload || payload.name !== mcpDetail.name) return;
  const status = payload.status;
  if (
    status !== "starting" &&
    status !== "ready" &&
    status !== "failed" &&
    status !== "cancelled"
  ) {
    return;
  }
  mcpDetail.startup = status;
  if (status === "ready") {
    void refreshMcpDetail();
  }
}

/** 打开指定服务器的详情弹窗并订阅启动状态通知 */
async function openMcpDetail(s: McpServerInfo) {
  await stopMcpDetailListen();
  mcpDetail.name = s.name;
  mcpDetail.transport = s.url.trim() ? "http" : "stdio";
  mcpDetail.command = s.command;
  mcpDetail.argsText = (s.args ?? []).join(" ");
  mcpDetail.cwd = s.cwd ?? "";
  mcpDetail.url = s.url;
  mcpDetail.detail = null;
  mcpDetail.notFound = false;
  mcpDetail.startup = "";
  mcpDetail.activeTab = "info";
  try {
    const unlisten = await listen<{
      name?: string;
      status?: string;
    }>("mcpServer/startupStatus/updated", (e) => {
      onMcpStartupUpdated(e.payload ?? {});
    });
    mcpDetailUnlisten = unlisten;
  } catch {
    // 通知监听失败不阻断查看
  }
  await refreshMcpDetail();
}

watch(
  () => props.server,
  (s) => {
    if (s) void openMcpDetail(s);
  },
  // 父层以 v-if 挂载本组件：server 挂载时即非空，必须立即初始化
  { immediate: true },
);

onBeforeUnmount(() => {
  void stopMcpDetailListen();
});

async function copyMcpDetailUri(uri: string) {
  const ok = await copyText(uri);
  setToast(ok ? "URI 已复制" : "复制失败，请手动选择复制");
}

function mcpDetailTabCount(id: McpDetailTabId): number {
  const detail = mcpDetail.detail;
  if (!detail) return 0;
  if (id === "tools") return detail.tools.length;
  if (id === "resources") {
    return detail.resources.length + detail.resourceTemplates.length;
  }
  return 0;
}

function mcpDetailSchemaText(tool: McpToolDetail): string {
  if (tool.inputSchema === undefined) return "";
  try {
    const text = JSON.stringify(tool.inputSchema, null, 2);
    return typeof text === "string" ? text : "";
  } catch {
    return "";
  }
}

function mcpDetailAuthLabel(): string {
  return mcpDetail.detail
    ? MCP_AUTH_LABELS[mcpDetail.detail.authStatus]
    : "";
}
</script>

<template>
  <ModalDialog
    :title="mcpDetail.name"
    closable
    bodyClass="mcp-detail-body"
    @close="emit('close')"
  >
    <div class="mcp-detail">
      <div
        class="mcp-detail-tabs"
        role="tablist"
        aria-label="MCP 详情分类"
      >
        <button
          v-for="t in MCP_DETAIL_TABS"
          :key="t.id"
          type="button"
          class="mcp-detail-tab"
          :class="{ active: mcpDetail.activeTab === t.id }"
          role="tab"
          :aria-selected="mcpDetail.activeTab === t.id"
          @click="mcpDetail.activeTab = t.id"
        >
          {{ t.label }}
          <span
            v-if="
              (t.id === 'tools' || t.id === 'resources') &&
              mcpDetail.detail
            "
            class="mcp-detail-tab-count"
          >
            {{ mcpDetailTabCount(t.id) }}
          </span>
        </button>
      </div>

      <div class="mcp-detail-body-inner">
        <div
          v-if="mcpDetail.loading && !mcpDetail.detail"
          class="mcp-detail-empty"
        >
          正在获取服务器状态…
        </div>
        <div
          v-else-if="mcpDetail.notFound"
          class="mcp-detail-empty"
        >
          <p>
            未查询到「{{ mcpDetail.name }}」的状态，服务器可能尚未被
            codex 加载。
          </p>
          <button
            class="btn"
            :disabled="mcpDetail.loading"
            @click="refreshMcpDetail"
          >
            重新查询
          </button>
        </div>
        <template v-else-if="mcpDetail.detail">
          <section
            v-show="mcpDetail.activeTab === 'info'"
            class="mcp-detail-pane"
          >
            <div class="mcp-detail-fields">
              <div class="mcp-detail-field">
                <span class="mcp-detail-field-label">名称</span>
                <span class="mcp-detail-field-value mono">
                  {{ mcpDetail.detail.name }}
                </span>
              </div>
              <div class="mcp-detail-field">
                <span class="mcp-detail-field-label">传输类型</span>
                <span class="mcp-detail-field-value">
                  {{ mcpDetail.transport === "http" ? "Streamable HTTP" : "stdio" }}
                </span>
              </div>
              <template v-if="mcpDetail.transport === 'http'">
                <div class="mcp-detail-field">
                  <span class="mcp-detail-field-label">url</span>
                  <span class="mcp-detail-field-value mono">
                    {{ mcpDetail.url }}
                  </span>
                </div>
              </template>
              <template v-else>
                <div class="mcp-detail-field">
                  <span class="mcp-detail-field-label">command</span>
                  <span class="mcp-detail-field-value mono">
                    {{ mcpDetail.command }}
                  </span>
                </div>
                <div
                  v-if="mcpDetail.argsText"
                  class="mcp-detail-field"
                >
                  <span class="mcp-detail-field-label">args</span>
                  <span class="mcp-detail-field-value mono">
                    {{ mcpDetail.argsText }}
                  </span>
                </div>
                <div v-if="mcpDetail.cwd" class="mcp-detail-field">
                  <span class="mcp-detail-field-label">cwd</span>
                  <span class="mcp-detail-field-value mono">
                    {{ mcpDetail.cwd }}
                  </span>
                </div>
              </template>
              <template
                v-if="mcpDetail.detail.serverInfo"
              >
                <div
                  v-if="mcpDetail.detail.serverInfo.title"
                  class="mcp-detail-field"
                >
                  <span class="mcp-detail-field-label">标题</span>
                  <span class="mcp-detail-field-value">
                    {{ mcpDetail.detail.serverInfo.title }}
                  </span>
                </div>
                <div
                  v-if="mcpDetail.detail.serverInfo.version"
                  class="mcp-detail-field"
                >
                  <span class="mcp-detail-field-label">版本</span>
                  <span class="mcp-detail-field-value">
                    {{ mcpDetail.detail.serverInfo.version }}
                  </span>
                </div>
                <div
                  v-if="mcpDetail.detail.serverInfo.description"
                  class="mcp-detail-field"
                >
                  <span class="mcp-detail-field-label">描述</span>
                  <span class="mcp-detail-field-value">
                    {{ mcpDetail.detail.serverInfo.description }}
                  </span>
                </div>
                <div
                  v-if="mcpDetail.detail.serverInfo.websiteUrl"
                  class="mcp-detail-field"
                >
                  <span class="mcp-detail-field-label">网站</span>
                  <span class="mcp-detail-field-value mono">
                    {{ mcpDetail.detail.serverInfo.websiteUrl }}
                  </span>
                </div>
              </template>
              <div class="mcp-detail-field">
                <span class="mcp-detail-field-label">认证状态</span>
                <span class="mcp-detail-field-value">
                  {{ mcpDetailAuthLabel() }}
                </span>
              </div>
            </div>
          </section>

          <section
            v-show="mcpDetail.activeTab === 'tools'"
            class="mcp-detail-pane"
          >
            <div
              v-if="mcpDetail.detail.tools.length === 0"
              class="mcp-detail-empty"
            >
              未获取到工具（服务器可能未启动），服务器就绪后将自动更新。
            </div>
            <div
              v-else
              class="mcp-detail-tools"
            >
              <article
                v-for="tool in mcpDetail.detail.tools"
                :key="tool.name"
                class="mcp-detail-tool"
              >
                <header class="mcp-detail-tool-head">
                  <span class="mcp-detail-tool-name">
                    {{ tool.name }}
                  </span>
                  <span
                    v-if="tool.title && tool.title !== tool.name"
                    class="mcp-detail-tool-title"
                  >
                    {{ tool.title }}
                  </span>
                </header>
                <p
                  v-if="tool.description"
                  class="mcp-detail-tool-desc"
                >
                  {{ tool.description }}
                </p>
                <details
                  v-if="tool.inputSchema !== undefined"
                  class="mcp-detail-schema"
                >
                  <summary>输入参数 JSON</summary>
                  <pre>{{ mcpDetailSchemaText(tool) }}</pre>
                </details>
              </article>
            </div>
          </section>

          <section
            v-show="mcpDetail.activeTab === 'resources'"
            class="mcp-detail-pane"
          >
            <div
              v-if="
                mcpDetail.detail.resources.length === 0 &&
                mcpDetail.detail.resourceTemplates.length === 0
              "
              class="mcp-detail-empty"
            >
              该服务器未声明资源与资源模板。
            </div>
            <template v-else>
              <template
                v-if="mcpDetail.detail.resources.length"
              >
                <h4 class="mcp-detail-group-title">
                  资源（{{ mcpDetail.detail.resources.length }}）
                </h4>
                <div class="mcp-detail-resources">
                  <div
                    v-for="r in mcpDetail.detail.resources"
                    :key="r.uri"
                    class="mcp-resource-row"
                  >
                    <div class="mcp-resource-main">
                      <span class="mcp-resource-title">
                        {{ r.title || r.name || r.uri }}
                      </span>
                      <code class="mcp-resource-uri">
                        {{ r.uri }}
                      </code>
                      <span
                        v-if="r.description || r.mimeType"
                        class="mcp-resource-meta"
                      >
                        {{
                          [r.description, r.mimeType]
                            .filter(Boolean)
                            .join(" · ")
                        }}
                      </span>
                    </div>
                    <button
                      class="btn mcp-resource-copy-btn"
                      aria-label="复制 URI"
                      v-tooltip="'复制 URI'"
                      @click="copyMcpDetailUri(r.uri)"
                    >
                      复制
                    </button>
                  </div>
                </div>
              </template>
              <template
                v-if="mcpDetail.detail.resourceTemplates.length"
              >
                <h4 class="mcp-detail-group-title">
                  资源模板（{{
                    mcpDetail.detail.resourceTemplates.length
                  }}）
                </h4>
                <div class="mcp-detail-resources">
                  <div
                    v-for="rt in mcpDetail.detail.resourceTemplates"
                    :key="rt.uriTemplate"
                    class="mcp-resource-row"
                  >
                    <div class="mcp-resource-main">
                      <span class="mcp-resource-title">
                        {{ rt.title || rt.name || rt.uriTemplate }}
                      </span>
                      <code class="mcp-resource-uri">
                        {{ rt.uriTemplate }}
                      </code>
                      <span
                        v-if="rt.description || rt.mimeType"
                        class="mcp-resource-meta"
                      >
                        {{
                          [rt.description, rt.mimeType]
                            .filter(Boolean)
                            .join(" · ")
                        }}
                      </span>
                    </div>
                  </div>
                </div>
              </template>
            </template>
          </section>

        </template>
      </div>
    </div>
  </ModalDialog>
</template>

<style scoped>
.mcp-detail {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
}

.mcp-detail-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
  border-bottom: 1px solid var(--border);
}

.mcp-detail-tab {
  padding: var(--space-2) var(--space-3);
  border: 1px solid transparent;
  border-bottom: none;
  border-radius: var(--radius) var(--radius) 0 0;
  background: transparent;
  color: var(--text-dim);
  font-size: var(--font-sm);
  cursor: pointer;
  transition: color var(--ease), background var(--ease), border-color var(--ease);
}

.mcp-detail-tab:hover {
  color: var(--text-bright);
  background: rgba(var(--overlay-rgb), 0.05);
}

.mcp-detail-tab-count {
  margin-left: 4px;
  font-weight: 600;
  color: var(--accent);
}

.mcp-detail-body-inner {
  min-height: 160px;
  overflow-y: auto;
}

.mcp-detail-pane {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.mcp-detail-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-8) var(--space-4);
  color: var(--text-dim);
  font-size: var(--font-sm);
  line-height: 1.5;
  text-align: center;
}

.mcp-detail-empty p {
  margin: 0;
}

.mcp-detail-fields {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.mcp-detail-field {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  font-size: var(--font-sm);
}

.mcp-detail-field-label {
  flex: 0 0 92px;
  color: var(--text-faint);
}

.mcp-detail-field-value {
  flex: 1;
  min-width: 0;
  color: var(--text);
  word-break: break-all;
  white-space: pre-wrap;
  user-select: text;
}

.mcp-detail-field-value.mono {
  font-family: var(--mono);
}

.mcp-detail-tools {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.mcp-detail-tool {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: rgba(var(--overlay-rgb), 0.02);
}

.mcp-detail-tool-head {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.mcp-detail-tool-name {
  font-family: var(--mono);
  font-size: var(--font-sm);
  font-weight: 700;
  color: var(--text-bright);
}

.mcp-detail-tool-title {
  font-size: var(--font-xs);
  color: var(--text-faint);
}

.mcp-detail-tool-desc {
  margin: 0;
  font-size: var(--font-sm);
  line-height: 1.5;
  color: var(--text-dim);
}

.mcp-detail-schema summary {
  font-size: var(--font-xs);
  color: var(--text-faint);
  cursor: pointer;
  user-select: none;
}

.mcp-detail-schema pre {
  margin: var(--space-2) 0 0;
  max-height: 260px;
  overflow: auto;
  padding: var(--space-3);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-input);
  font-family: var(--mono);
  font-size: var(--font-xs);
  line-height: 1.5;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-all;
}

.mcp-detail-group-title {
  margin: 0;
  font-size: var(--font-sm);
  font-weight: 700;
  color: var(--text);
}

.mcp-detail-resources {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.mcp-resource-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: rgba(var(--overlay-rgb), 0.02);
}

.mcp-resource-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.mcp-resource-title {
  font-size: var(--font-sm);
  font-weight: 600;
  color: var(--text-bright);
}

.mcp-resource-uri {
  font-family: var(--mono);
  font-size: var(--font-xs);
  color: var(--text-dim);
  word-break: break-all;
  user-select: text;
}

.mcp-resource-meta {
  font-size: var(--font-xs);
  color: var(--text-faint);
  word-break: break-word;
}

.mcp-resource-copy-btn {
  flex-shrink: 0;
  min-height: var(--ctrl-h-xs);
  font-size: var(--font-xs);
}
</style>

