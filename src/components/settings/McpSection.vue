<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import {
  askConfirm,
  loadMcpServers,
  saveMcpServers,
  setToast,
  toastError,
} from "../../composables/useCodex";
import {
  ICON_DELETE,
  ICON_EDIT,
  ICON_INFO,
  ICON_MCP,
  ICON_PLUS,
  ICON_REFRESH,
} from "../../lib/icons";
import type { McpEnvEntry, McpServerInfo } from "../../lib/types";
import AppSelect, { type AppSelectOption } from "../AppSelect.vue";
import ModalDialog from "../ModalDialog.vue";
import McpDetailDialog from "./McpDetailDialog.vue";

const props = defineProps<{ active: boolean }>();

/** MCP 传输方式选项 */
const transportOptions: AppSelectOption[] = [
  { value: "stdio", label: "stdio" },
  { value: "http", label: "Streamable HTTP" },
];

const mcpState = reactive({
  loading: false,
  saving: false,
  servers: [] as McpServerInfo[],
  /** config/read 用户层原始 [mcp_servers.*]，用于保存时保留未知字段 */
  raw: {} as Record<string, unknown>,
});

/** omit_tools_from 可选暴露面（勾选=从该面排除），UI 展示用中文备注 */
const MCP_OMIT_OPTIONS = [
  { value: "direct", label: "直接暴露" },
  { value: "deferred", label: "延迟暴露" },
  { value: "code_mode", label: "代码模式" },
] as const;

/** MCP 新增/编辑表单状态（editingIndex < 0 表示新增） */
const mcpForm = reactive({
  open: false,
  editingIndex: -1,
  /** 传输方式："stdio" | "http" */
  transport: "stdio" as "stdio" | "http",
  name: "",
  command: "",
  cwd: "",
  argsText: "",
  env: [] as McpEnvEntry[],
  url: "",
  headers: [] as McpEnvEntry[],
  bearer_token_env_var: "",
  omit_tools_from: [] as string[],
});

const mcpFormErrors = reactive({
  name: "",
  command: "",
  url: "",
  env: "",
});

function clearMcpFormErrors() {
  mcpFormErrors.name = "";
  mcpFormErrors.command = "";
  mcpFormErrors.url = "";
  mcpFormErrors.env = "";
}

onMounted(() => {
  void loadMcp();
});

async function loadMcp() {
  if (mcpState.loading) return;
  mcpState.loading = true;
  try {
    const res = await loadMcpServers();
    mcpState.servers = res.servers.map((s) => ({ ...s }));
    mcpState.raw = res.raw;
  } catch (e) {
    setToast(toastError(e));
  } finally {
    mcpState.loading = false;
  }
}

/** 打开「添加」MCP 服务器表单 */
function openAddMcp() {
  mcpForm.open = true;
  mcpForm.editingIndex = -1;
  mcpForm.transport = "stdio";
  mcpForm.name = "";
  mcpForm.command = "";
  mcpForm.cwd = "";
  mcpForm.argsText = "";
  mcpForm.env = [];
  mcpForm.url = "";
  mcpForm.headers = [];
  mcpForm.bearer_token_env_var = "";
  mcpForm.omit_tools_from = ["deferred"];
  clearMcpFormErrors();
}

/** 打开「编辑」MCP 服务器表单（标识 name 只读） */
function openEditMcp(index: number) {
  const s = mcpState.servers[index];
  if (!s) return;
  mcpForm.open = true;
  mcpForm.editingIndex = index;
  mcpForm.name = s.name;
  mcpForm.transport = s.url.trim() ? "http" : "stdio";
  mcpForm.command = s.command;
  mcpForm.cwd = s.cwd ?? "";
  mcpForm.argsText = (s.args ?? []).join(" ");
  mcpForm.env = (s.env ?? []).map((e) => ({ ...e }));
  mcpForm.url = s.url ?? "";
  mcpForm.headers = (s.headers ?? []).map((e) => ({ ...e }));
  mcpForm.bearer_token_env_var = s.bearer_token_env_var ?? "";
  mcpForm.omit_tools_from = [...(s.omit_tools_from ?? [])];
  clearMcpFormErrors();
}

function closeMcpForm() {
  mcpForm.open = false;
}

function addMcpEnvRow() {
  mcpForm.env.push({ key: "", value: "" });
}

function removeMcpEnvRow(index: number) {
  mcpForm.env.splice(index, 1);
}

function addMcpHeaderRow() {
  mcpForm.headers.push({ key: "", value: "" });
}

function removeMcpHeaderRow(index: number) {
  mcpForm.headers.splice(index, 1);
}

/** 提交 MCP 表单：校验后更新本地列表并立即落盘 */
async function confirmMcpForm() {
  if (mcpState.saving || mcpState.loading) return;
  clearMcpFormErrors();
  const name = mcpForm.name.trim();
  if (!name) {
    mcpFormErrors.name = "请填写服务器名称";
    return;
  }
  if (!/^[A-Za-z0-9_-]+$/.test(name)) {
    mcpFormErrors.name = "只能包含字母、数字、下划线与连字符";
    return;
  }
  if (mcpForm.editingIndex < 0) {
    if (mcpState.servers.some((s) => s.name === name)) {
      mcpFormErrors.name = "服务器名称已存在";
      return;
    }
  }
  const isHttp = mcpForm.transport === "http";
  if (isHttp) {
    if (!mcpForm.url.trim()) {
      mcpFormErrors.url = "请填写 url";
      return;
    }
    if (!/^https?:\/\//i.test(mcpForm.url.trim())) {
      mcpFormErrors.url = "url 必须以 http:// 或 https:// 开头";
      return;
    }
  } else if (!mcpForm.command.trim()) {
    mcpFormErrors.command = "请填写 command";
    return;
  }
  const kvRows = isHttp ? mcpForm.headers : mcpForm.env;
  const kvLabel = isHttp ? "请求头" : "env";
  const envKeys = new Set<string>();
  for (const e of kvRows) {
    const key = e.key.trim();
    if (!key) {
      mcpFormErrors.env = `存在空的 ${kvLabel} 键`;
      return;
    }
    if (envKeys.has(key)) {
      mcpFormErrors.env = `${kvLabel}键「${key}」重复`;
      return;
    }
    envKeys.add(key);
  }
  const args = mcpForm.argsText
    .split(/\s+/)
    .map((a) => a.trim())
    .filter(Boolean);
  const entry: McpServerInfo = {
    name,
    command: isHttp ? "" : mcpForm.command.trim(),
    cwd: isHttp ? "" : mcpForm.cwd.trim(),
    args: isHttp ? [] : args,
    env: isHttp ? [] : mcpForm.env.map((e) => ({ key: e.key.trim(), value: e.value })),
    url: isHttp ? mcpForm.url.trim() : "",
    headers: isHttp
      ? mcpForm.headers.map((e) => ({ key: e.key.trim(), value: e.value }))
      : [],
    bearer_token_env_var: isHttp ? mcpForm.bearer_token_env_var.trim() : "",
    omit_tools_from: [...mcpForm.omit_tools_from],
  };
  const isNew = mcpForm.editingIndex < 0;
  if (isNew) {
    mcpState.servers.push(entry);
  } else {
    mcpState.servers[mcpForm.editingIndex] = entry;
  }
  mcpState.saving = true;
  try {
    await saveMcpServers(mcpState.servers, mcpState.raw);
    setToast("MCP 服务器已保存");
    mcpForm.open = false;
    await loadMcp();
  } catch (e) {
    setToast(toastError(e));
    // 回滚：重读磁盘状态，保留表单输入
    await loadMcp();
  } finally {
    mcpState.saving = false;
  }
}

/** 删除 MCP 服务器：弹确认后立即落盘 */
async function removeMcp(index: number) {
  const s = mcpState.servers[index];
  if (!s || mcpState.saving || mcpState.loading) return;
  const ok = await askConfirm({
    title: "删除 MCP 服务器",
    message: `确定删除 MCP 服务器「${s.name}」吗？`,
    confirmLabel: "删除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  mcpState.servers.splice(index, 1);
  mcpState.saving = true;
  try {
    await saveMcpServers(mcpState.servers, mcpState.raw);
    setToast(`已删除 MCP 服务器「${s.name}」`);
    await loadMcp();
  } catch (e) {
    setToast(toastError(e));
    await loadMcp(); // 回滚
  } finally {
    mcpState.saving = false;
  }
}

// ---------- MCP 详情信息 ----------

/** 详情弹窗目标服务器（null=关闭；详情状态与监听生命周期都在 McpDetailDialog 内） */
const mcpDetailServer = ref<McpServerInfo | null>(null);

/** 打开指定服务器的详情弹窗 */
function openMcpDetail(index: number) {
  const s = mcpState.servers[index];
  if (!s || mcpState.loading || mcpState.saving) return;
  mcpDetailServer.value = s;
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-mcp">
    <h2 class="settings-section-title">MCP 管理</h2>
    <p class="settings-section-desc">
      配置 MCP 服务器
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>MCP 服务器</h3>
        <div class="model-config-head-actions">
          <button
            class="btn btn-icon primary mcp-config-add-btn"
            v-tooltip="'添加'"
            aria-label="添加"
            :disabled="mcpState.loading || mcpState.saving"
            @click="openAddMcp"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_PLUS" />
            </svg>
          </button>
          <button
            class="btn btn-icon model-config-reload-btn"
            aria-label="刷新"
            v-tooltip="'刷新'"
            :disabled="mcpState.loading"
            @click="loadMcp"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_REFRESH" />
            </svg>
          </button>
        </div>
      </div>
      <div class="mcp-servers-list">
        <div
          v-if="mcpState.loading && !mcpState.servers.length"
          class="plugin-empty"
        >
          正在加载 MCP 服务器…
        </div>
        <div v-else-if="!mcpState.servers.length" class="plugin-empty">
          还没有 MCP 服务器，点击卡片头部「＋」创建。
        </div>
        <div
          v-for="(s, i) in mcpState.servers"
          :key="s.name"
          class="mcp-server-row"
        >
          <span class="row-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <!-- ICON_MCP 是面字形（cable 带子路径），按 fill 渲染才成形；
                   描边会把 1 单位宽的带子两侧各描一圈，14px 下糊成一团 -->
              <path :d="ICON_MCP" fill="currentColor" />
            </svg>
          </span>
          <div class="mcp-server-info">
            <span class="mcp-server-name">{{ s.name }}</span>
            <div class="mcp-server-meta">
              <span class="mcp-server-type">{{ s.url.trim() ? "http" : "stdio" }}</span>
              <span
                v-if="s.omit_tools_from?.length"
                class="mcp-server-omit"
                v-tooltip="`omit_tools_from: ${s.omit_tools_from.join(', ')}`"
              >
                omit: {{ s.omit_tools_from.join("/") }}
              </span>
            </div>
          </div>
          <div class="model-provider-actions">
            <button
              class="btn btn-icon mcp-row-info"
              aria-label="查看详情"
              v-tooltip="'查看详情'"
              :disabled="mcpState.loading || mcpState.saving"
              @click="openMcpDetail(i)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_INFO" />
              </svg>
            </button>
            <button
              class="btn btn-icon mcp-row-edit"
              aria-label="编辑"
              v-tooltip="'编辑'"
              :disabled="mcpState.loading || mcpState.saving"
              @click="openEditMcp(i)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_EDIT" />
              </svg>
            </button>
            <button
              class="btn btn-icon danger mcp-row-delete"
              aria-label="删除"
              v-tooltip="'删除'"
              :disabled="mcpState.loading || mcpState.saving"
              @click="removeMcp(i)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_DELETE" />
              </svg>
            </button>
          </div>
        </div>
      </div>

      <ModalDialog
        v-if="mcpForm.open"
        :title="mcpForm.editingIndex >= 0 ? '编辑 MCP 服务器' : '添加 MCP 服务器'"
        closable
        @close="closeMcpForm"
      >
        <div class="mcp-server-form">
          <div
            class="setting-row"
            :class="{ 'model-config-row-error': mcpFormErrors.name }"
          >
            <label>
              名称（name）
              <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
            </label>
            <input
              v-model="mcpForm.name"
              type="text"
              :disabled="mcpForm.editingIndex >= 0"
              placeholder="如 filesystem"
              :class="{ 'model-config-input-error': mcpFormErrors.name }"
            />
            <p v-if="mcpFormErrors.name" class="model-config-field-error">
              {{ mcpFormErrors.name }}
            </p>
          </div>
          <div class="setting-row">
            <label for="mcp-form-transport">传输方式</label>
            <AppSelect
              id="mcp-form-transport"
              v-model="mcpForm.transport"
              :options="transportOptions"
            />
          </div>
          <div class="setting-row mcp-omit-row">
            <label>omit_tools_from（工具暴露面）</label>
            <div class="mcp-omit-options">
              <p class="mcp-omit-hint">
                勾选表示「从该暴露面排除」，该服务器工具不再以对应方式暴露给模型。
              </p>
              <label
                v-for="opt in MCP_OMIT_OPTIONS"
                :key="opt.value"
                class="mcp-omit-option"
              >
                <input
                  type="checkbox"
                  :value="opt.value"
                  v-model="mcpForm.omit_tools_from"
                />
                <span class="mcp-omit-value">{{ opt.value }}</span>
                <span class="mcp-omit-desc">{{ opt.label }}</span>
              </label>
              <p class="mcp-omit-note">
                新增服务器默认勾选 deferred（直接内联，便于 DeepSeek 等模型使用）；若三个全勾选，工具将完全对模型隐藏。
              </p>
            </div>
          </div>
          <div
            v-if="mcpForm.transport === 'stdio'"
            class="setting-row"
            :class="{ 'model-config-row-error': mcpFormErrors.command }"
          >
            <label>
              command
              <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
            </label>
            <input
              v-model="mcpForm.command"
              type="text"
              placeholder="如 npx"
              :class="{ 'model-config-input-error': mcpFormErrors.command }"
            />
            <p
              v-if="mcpFormErrors.command"
              class="model-config-field-error"
            >
              {{ mcpFormErrors.command }}
            </p>
          </div>
          <div
            v-if="mcpForm.transport === 'stdio'"
            class="setting-row"
          >
            <label>cwd（工作目录）</label>
            <input
              v-model="mcpForm.cwd"
              type="text"
              placeholder="服务器进程启动目录，可留空（如 D:\\project）"
            />
          </div>
          <div
            v-if="mcpForm.transport === 'stdio'"
            class="setting-row"
          >
            <label>args（空格分隔）</label>
            <input
              class="mcp-args-input"
              v-model="mcpForm.argsText"
              type="text"
              placeholder="如 -y @modelcontextprotocol/server-filesystem ."
            />
          </div>
          <div
            v-if="mcpForm.transport === 'stdio'"
            class="setting-row mcp-env-block"
            :class="{ 'model-config-row-error': mcpFormErrors.env }"
          >
            <label>env（环境变量）</label>
            <div class="mcp-env-rows">
              <div
                v-for="(e, i) in mcpForm.env"
                :key="i"
                class="mcp-env-row"
              >
                <input
                  v-model="e.key"
                  type="text"
                  placeholder="环境变量名"
                />
                <input
                  v-model="e.value"
                  type="text"
                  placeholder="值"
                />
                <button
                  class="btn btn-icon danger"
                  aria-label="删除该环境变量"
                  v-tooltip="'删除该环境变量'"
                  @click="removeMcpEnvRow(i)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_DELETE" />
                  </svg>
                </button>
              </div>
            </div>
            <p v-if="mcpFormErrors.env" class="model-config-field-error">
              {{ mcpFormErrors.env }}
            </p>
          </div>
          <div
            v-if="mcpForm.transport === 'http'"
            class="setting-row"
            :class="{ 'model-config-row-error': mcpFormErrors.url }"
          >
            <label>
              url
              <span class="model-config-required" aria-label="必填" v-tooltip="'必填'">*</span>
            </label>
            <input
              v-model="mcpForm.url"
              type="text"
              placeholder="如 https://example.com/mcp"
              :class="{ 'model-config-input-error': mcpFormErrors.url }"
            />
            <p v-if="mcpFormErrors.url" class="model-config-field-error">
              {{ mcpFormErrors.url }}
            </p>
          </div>
          <div
            v-if="mcpForm.transport === 'http'"
            class="setting-row"
          >
            <label>bearer_token_env_var（Bearer 令牌环境变量名）</label>
            <input
              v-model="mcpForm.bearer_token_env_var"
              type="text"
              placeholder="如 MY_MCP_TOKEN"
            />
          </div>
          <div
            v-if="mcpForm.transport === 'http'"
            class="setting-row mcp-env-block"
            :class="{ 'model-config-row-error': mcpFormErrors.env }"
          >
            <label>http_headers（静态请求头）</label>
            <div class="mcp-env-rows">
              <div
                v-for="(h, i) in mcpForm.headers"
                :key="i"
                class="mcp-env-row"
              >
                <input
                  v-model="h.key"
                  type="text"
                  placeholder="请求头名"
                />
                <input
                  v-model="h.value"
                  type="text"
                  placeholder="值"
                />
                <button
                  class="btn btn-icon danger"
                  aria-label="删除该请求头"
                  v-tooltip="'删除该请求头'"
                  @click="removeMcpHeaderRow(i)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_DELETE" />
                  </svg>
                </button>
              </div>
            </div>
            <p v-if="mcpFormErrors.env" class="model-config-field-error">
              {{ mcpFormErrors.env }}
            </p>
          </div>
          <div class="model-config-actions">
            <button
              v-if="mcpForm.transport === 'stdio'"
              class="btn btn-icon mcp-kv-add-btn"
              aria-label="添加环境变量"
              v-tooltip="'添加环境变量'"
              @click="addMcpEnvRow"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_PLUS" />
              </svg>
            </button>
            <button
              v-else
              class="btn btn-icon mcp-kv-add-btn"
              aria-label="添加请求头"
              v-tooltip="'添加请求头'"
              @click="addMcpHeaderRow"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_PLUS" />
              </svg>
            </button>
          </div>
        </div>
        <template #foot>
          <button class="btn" @click="closeMcpForm">取消</button>
          <button
            class="btn primary mcp-form-submit"
            :disabled="mcpState.saving || mcpState.loading"
            @click="confirmMcpForm"
          >
            {{ mcpForm.editingIndex >= 0 ? "保存修改" : "添加" }}
          </button>
        </template>
      </ModalDialog>

      <McpDetailDialog
        v-if="mcpDetailServer"
        :server="mcpDetailServer"
        @close="mcpDetailServer = null"
      />

    </div>
  </section>
</template>

<style scoped>
.mcp-server-omit {
  white-space: nowrap;
}

/* MCP 管理 */
.mcp-servers-list {
  display: flex;
  flex-direction: column;
}

.mcp-server-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-4) 0;
  border-bottom: 1px solid var(--border);
  transition: background var(--ease);
}

.mcp-server-row:hover {
  background: rgba(var(--overlay-rgb), 0.025);
}

.mcp-server-row:last-child {
  border-bottom: none;
}

.mcp-server-info {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  flex: 1;
  min-width: 0;
}

.mcp-server-meta {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.mcp-server-name {
  font-size: var(--font-base);
  color: var(--text-bright);
  font-family: var(--mono);
  user-select: text;
}

.mcp-server-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--space-6) var(--space-8);
  background: rgba(var(--overlay-rgb), 0.02);
}

.mcp-env-block {
  flex-direction: column;
  align-items: stretch;
}

.mcp-env-rows {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.mcp-env-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.mcp-env-row input {
  flex: 1;
}

.mcp-omit-row {
  flex-direction: column;
  align-items: stretch;
}

.mcp-omit-options {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.mcp-server-form .mcp-omit-option {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin: 0;
  font-weight: 400;
  font-size: var(--font-sm);
  color: var(--text);
  cursor: pointer;
  line-height: 1.4;
}

.mcp-omit-option input[type="checkbox"] {
  appearance: none;
  -webkit-appearance: none;
  width: 16px;
  height: 16px;
  margin: 0;
  flex-shrink: 0;
  position: relative;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  cursor: pointer;
  transition: background var(--ease), border-color var(--ease), box-shadow var(--ease);
}

.mcp-omit-option input[type="checkbox"]:hover {
  border-color: var(--accent-dim);
}

.mcp-omit-option input[type="checkbox"]:focus-visible {
  outline: none;
  border-color: var(--accent-dim);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.12);
}

.mcp-omit-option input[type="checkbox"]:checked {
  background: var(--accent);
  border-color: var(--accent);
}

.mcp-omit-option input[type="checkbox"]:checked::after {
  content: "";
  position: absolute;
  left: 5px;
  top: 2px;
  width: 4px;
  height: 8px;
  border: solid var(--on-accent);
  border-width: 0 2px 2px 0;
  transform: rotate(45deg);
}

.mcp-omit-value {
  font-family: var(--mono);
  font-size: var(--font-sm);
  color: var(--text-bright);
  flex-shrink: 0;
}

.mcp-omit-desc {
  flex: 1;
  min-width: 0;
  font-size: var(--font-sm);
  color: var(--text-faint);
}

.mcp-omit-hint,
.mcp-omit-note {
  margin: 0;
  font-size: var(--font-sm);
}

.mcp-omit-hint {
  color: var(--text-faint);
}

.mcp-omit-note {
  color: var(--yellow);
}
</style>
