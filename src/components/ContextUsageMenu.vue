<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { type SessionTab } from "../composables/useCodex";
import { useContextUsage } from "../composables/useContextUsage";
import { formatTokens } from "../lib/format";
import { ICON_ARROW_DOWN, ICON_ARROW_UP, ICON_COMPRESS } from "../lib/icons";

const props = defineProps<{ tab: SessionTab }>();

const { ctxUsage, compacting, compactNow } = useContextUsage(props.tab);

const usage = computed(() => props.tab.threadTokenUsage);
// 无任何用量数据时整组件不渲染（与原导航底栏「缺数据不显示」一致）
const hasUsage = computed(() => usage.value != null);

// ---------- 圆环：进度 = 上下文占用百分比（used / window） ----------
const RING_CIRCUMFERENCE = 2 * Math.PI * 10;
const ringPct = computed(() => ctxUsage.value?.pct ?? 0);
/** 环心数字：一位数补 %（如 5%），两位及以上保持纯数字（如 50） */
const ringPctText = computed(() =>
  ringPct.value < 10 ? `${ringPct.value}%` : String(ringPct.value),
);
const ringDash = computed(
  () => `${(RING_CIRCUMFERENCE * ringPct.value) / 100} ${RING_CIRCUMFERENCE}`,
);
const ringTooltip = computed(() => {
  const parts: string[] = [];
  const u = ctxUsage.value;
  if (u) {
    parts.push(
      `上下文已用 ${formatTokens(u.used)} / ${formatTokens(u.window)}（${u.pct}%）`,
    );
  }
  const t = usage.value;
  if (typeof t?.input === "number" && typeof t?.output === "number") {
    parts.push(
      `Token消耗： 输入 ${formatTokens(t.input)}，输出 ${formatTokens(t.output)}`,
    );
  }
  return parts.join(" · ");
});

// ---------- 悬浮菜单：镜像回合导航按钮的纯悬停开关（容器级 enter/leave + 延迟关闭） ----------
const menuOpen = ref(false);
const rootEl = ref<HTMLElement | null>(null);
let closeTimer: number | undefined;
function cancelClose() {
  if (closeTimer !== undefined) {
    window.clearTimeout(closeTimer);
    closeTimer = undefined;
  }
}
function openMenu() {
  cancelClose();
  menuOpen.value = true;
}
function scheduleClose() {
  if (!menuOpen.value) return;
  cancelClose();
  closeTimer = window.setTimeout(() => {
    closeTimer = undefined;
    menuOpen.value = false;
  }, 150);
}
function toggleMenu() {
  if (menuOpen.value) {
    cancelClose();
    menuOpen.value = false;
  } else {
    openMenu();
  }
}
/** 键盘 Enter/Space 开合（与回合导航按钮一致；preventDefault 阻止按钮 Space 默认点击） */
function onKeydown(e: KeyboardEvent) {
  if (e.key !== "Enter" && e.key !== " ") return;
  e.preventDefault();
  toggleMenu();
}
function onWindowMouseDown(e: MouseEvent) {
  if (!menuOpen.value) return;
  if (rootEl.value?.contains(e.target as Node)) return;
  menuOpen.value = false;
}
function onWindowKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") menuOpen.value = false;
}
onMounted(() => {
  window.addEventListener("mousedown", onWindowMouseDown);
  window.addEventListener("keydown", onWindowKeydown);
});
onBeforeUnmount(() => {
  window.removeEventListener("mousedown", onWindowMouseDown);
  window.removeEventListener("keydown", onWindowKeydown);
  cancelClose();
});

// ---------- 菜单内容 ----------
const remaining = computed(() => {
  const u = ctxUsage.value;
  if (!u) return "";
  return formatTokens(Math.max(0, u.window - u.used));
});

interface UsageRow {
  key: string;
  label: string;
  value: string;
  icon?: string;
  sub?: boolean;
}
/** 会话累计 token 明细行；事件缺字段时整行隐藏 */
const totalRows = computed<UsageRow[]>(() => {
  const t = usage.value;
  if (!t) return [];
  const rows: UsageRow[] = [];
  if (typeof t.input === "number") {
    rows.push({ key: "input", label: "输入", value: formatTokens(t.input), icon: ICON_ARROW_UP });
  }
  if (typeof t.output === "number") {
    rows.push({ key: "output", label: "输出", value: formatTokens(t.output), icon: ICON_ARROW_DOWN });
  }
  if (typeof t.totalTokens === "number") {
    rows.push({ key: "total", label: "合计", value: formatTokens(t.totalTokens) });
  }
  if (typeof t.cachedInput === "number") {
    rows.push({ key: "cachedInput", label: "缓存读取", value: formatTokens(t.cachedInput), sub: true });
  }
  if (typeof t.cacheWriteInput === "number") {
    rows.push({ key: "cacheWriteInput", label: "缓存写入", value: formatTokens(t.cacheWriteInput), sub: true });
  }
  if (typeof t.reasoningOutput === "number") {
    rows.push({ key: "reasoningOutput", label: "推理输出", value: formatTokens(t.reasoningOutput), sub: true });
  }
  return rows;
});
</script>

<template>
  <!-- 悬停开关绑在容器上（含按钮与菜单），与回合导航按钮同模式 -->
  <div
    v-if="hasUsage"
    ref="rootEl"
    class="menu-anchor ctx-ring-anchor"
    @mouseenter="openMenu()"
    @mouseleave="scheduleClose()"
  >
    <button
      type="button"
      class="ctx-ring"
      :aria-label="ringTooltip || '上下文用量'"
      aria-haspopup="true"
      :aria-expanded="menuOpen ? 'true' : 'false'"
      @keydown="onKeydown"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle class="ctx-ring-track" cx="12" cy="12" r="10" />
        <circle
          v-if="ctxUsage"
          class="ctx-ring-bar"
          cx="12"
          cy="12"
          r="10"
          :stroke-dasharray="ringDash"
          transform="rotate(-90 12 12)"
        />
      </svg>
      <span v-if="ctxUsage" class="ctx-ring-pct">{{ ringPctText }}</span>
    </button>
    <div v-if="menuOpen" class="popup-menu usage-menu" role="menu" @click.stop>
      <div class="usage-menu-head">
        <span class="usage-menu-title">上下文与 Token</span>
      </div>

      <div v-if="ctxUsage" class="usage-menu-section">
        <div class="usage-menu-bar-row">
          <div
            class="usage-menu-bar"
            role="progressbar"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-valuenow="ringPct"
          >
            <div class="usage-menu-bar-fill" :style="{ width: ringPct + '%' }"></div>
          </div>
          <!-- 百分比 + 压缩图标合成胶囊（复刻旧导航底栏合并胶囊风格） -->
          <span class="usage-menu-capsule">
            <span class="usage-menu-capsule-pct">{{ ringPct }}%</span>
            <button
              type="button"
              class="usage-menu-capsule-btn"
              :aria-label="compacting ? '正在压缩上下文' : '压缩上下文'"
              v-tooltip="'压缩上下文'"
              :disabled="compacting"
              @click="compactNow()"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_COMPRESS" />
              </svg>
            </button>
          </span>
        </div>
        <div class="usage-menu-line">
          已用 {{ formatTokens(ctxUsage.used) }} / 窗口
          {{ formatTokens(ctxUsage.window) }} · 剩余 {{ remaining }}
        </div>
      </div>
      <div v-else class="usage-menu-section">
        <div class="usage-menu-line dim">窗口大小未知，暂无上下文占用信息</div>
      </div>

      <div v-if="totalRows.length" class="usage-menu-section">
        <div class="usage-menu-subtitle">会话累计 TOKEN</div>
        <div
          v-for="row in totalRows"
          :key="row.key"
          class="usage-menu-row"
          :class="{ sub: row.sub }"
        >
          <span class="usage-menu-label">
            <svg v-if="row.icon" class="usage-menu-ico" viewBox="0 0 24 24" aria-hidden="true">
              <path :d="row.icon" />
            </svg>
            {{ row.label }}
          </span>
          <span class="usage-menu-value">{{ row.value }}</span>
        </div>
      </div>
    </div>
  </div>
</template>
