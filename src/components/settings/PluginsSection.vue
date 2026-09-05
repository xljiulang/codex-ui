<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from "vue";
import {
  addMarketplace,
  askConfirm,
  checkBrowserBridge,
  installPlugin,
  isAuthRequiredError,
  isChromeBridgePlugin,
  loadPluginCatalog,
  removeMarketplace,
  repairBrowserBridge,
  setToast,
  toastError,
  uninstallPlugin,
} from "../../composables/useCodex";
import type {
  PluginCatalogItem,
  PluginMarketplaceInfo,
  PluginMarketplaceLoadError,
} from "../../composables/useCodex";
import { assetUrl } from "../../lib/asset";
import {
  ICON_ARROW_DOWN,
  ICON_ARROW_RIGHT,
  ICON_DELETE,
  ICON_DOWNLOAD,
  ICON_EXTENSION,
  ICON_PLUS,
  ICON_REFRESH,
} from "../../lib/icons";

const props = defineProps<{ active: boolean }>();

const pluginState = reactive({
  loading: false,
  marketplaces: [] as PluginMarketplaceInfo[],
  loadErrors: [] as PluginMarketplaceLoadError[],
  busy: {} as Record<string, boolean>,
  /** 市场折叠状态（按市场 name；默认折叠，重开设置页重置） */
  collapsed: {} as Record<string, boolean>,
  adding: false,
  source: "",
});

onMounted(() => {
  void refreshPlugins();
});

async function refreshPlugins(force = false) {
  pluginState.loading = true;
  try {
    const res = await loadPluginCatalog(force);
    pluginState.marketplaces = res.marketplaces;
    pluginState.loadErrors = res.marketplaceLoadErrors;
    // 插件市场默认折叠；用 ??= 保留用户本次会话内已手动展开/折叠的选择
    for (const mp of res.marketplaces) {
      pluginState.collapsed[mp.name] ??= true;
    }
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pluginState.loading = false;
  }
}

/** chrome 插件安装后的浏览器桥接自愈提示：ok → 已就绪，no-registration → 无需处理，error → 失败原因 */
async function chromeBridgeToastSuffix(): Promise<string> {
  try {
    const report = await repairBrowserBridge();
    if (report.status === "ok") return "，浏览器桥接已就绪";
    if (report.status === "no-registration") return "";
    return `，浏览器桥接自愈失败：${report.message}`;
  } catch (e) {
    return `，浏览器桥接自愈失败：${toastError(e)}`;
  }
}

async function doInstall(
  mp: PluginMarketplaceInfo,
  plugin: PluginCatalogItem,
) {
  if (pluginState.busy[plugin.id]) return;
  pluginState.busy[plugin.id] = true;
  try {
    const res = await installPlugin(mp, plugin);
    const needsAuth =
      (res.appsNeedingAuth?.length ?? 0) > 0 ||
      /needs auth|requires auth|on install/i.test(res.authPolicy ?? "");
    // chrome 插件重装只恢复版本目录、不重建 latest junction，装完自动自愈浏览器桥接
    const bridgeSuffix = isChromeBridgePlugin(plugin)
      ? await chromeBridgeToastSuffix()
      : "";
    if (needsAuth) {
      setToast(
        `已安装 ${plugin.displayName}，但部分能力需要账号登录（当前 API key 不可用）${bridgeSuffix}`,
      );
    } else {
      setToast(`已安装 ${plugin.displayName}${bridgeSuffix}`);
    }
    await refreshPlugins();
  } catch (e) {
    if (isAuthRequiredError(e)) {
      setToast(
        `${plugin.displayName} 需要账号登录，当前 API key 不可用：${toastError(e)}`,
      );
    } else {
      setToast(toastError(e));
    }
  } finally {
    pluginState.busy[plugin.id] = false;
  }
}

async function doUninstall(plugin: PluginCatalogItem) {
  if (pluginState.busy[plugin.id]) return;
  if (isChromeBridgePlugin(plugin)) {
    // 桥接进程运行中会锁住插件缓存目录（卸载报 os error 5），提前提示先关 Chrome
    try {
      const bridge = await checkBrowserBridge();
      if (bridge.extensionHostRunning || bridge.nodeReplRunning) {
        const proceed = await askConfirm({
          title: "浏览器桥接进程运行中",
          message:
            "检测到浏览器桥接进程（extension-host / node_repl）正在运行，直接卸载可能因缓存文件被占用而失败。建议先完全退出 Chrome 再重试，仍要继续卸载吗？",
          confirmLabel: "继续卸载",
          cancelLabel: "取消",
        });
        if (!proceed) return;
      }
    } catch {
      // 预检失败不阻断卸载
    }
  }
  const ok = await askConfirm({
    title: "卸载插件",
    message: `确定卸载「${plugin.displayName}」吗？`,
    confirmLabel: "卸载",
    cancelLabel: "取消",
  });
  if (!ok) return;
  pluginState.busy[plugin.id] = true;
  try {
    await uninstallPlugin(plugin.id);
    setToast(`已卸载 ${plugin.displayName}`);
    await refreshPlugins();
  } catch (e) {
    const msg = toastError(e);
    if (/os error 5|拒绝访问/i.test(msg)) {
      setToast(
        "卸载失败：插件缓存文件被浏览器桥接进程占用，请先完全退出 Chrome 后重试",
      );
    } else {
      setToast(msg);
    }
  } finally {
    pluginState.busy[plugin.id] = false;
  }
}

async function doAddMarketplace() {
  const source = pluginState.source.trim();
  if (!source || pluginState.adding) return;
  pluginState.adding = true;
  try {
    await addMarketplace(source);
    pluginState.source = "";
    setToast("市场已添加");
    await refreshPlugins(true);
  } catch (e) {
    setToast(toastError(e));
  } finally {
    pluginState.adding = false;
  }
}

async function doRemoveMarketplace(mp: PluginMarketplaceInfo) {
  const ok = await askConfirm({
    title: "移除市场",
    message: `确定移除市场「${mp.displayName}」并删除其安装根吗？`,
    confirmLabel: "移除",
    cancelLabel: "取消",
  });
  if (!ok) return;
  try {
    await removeMarketplace(mp.name);
    setToast(`已移除 ${mp.displayName}`);
    await refreshPlugins();
  } catch (e) {
    setToast(toastError(e));
  }
}

function toggleMarketplace(mp: PluginMarketplaceInfo) {
  pluginState.collapsed[mp.name] = !pluginState.collapsed[mp.name];
}

function statusLabel(p: PluginCatalogItem): string {
  if (p.availability === "DisabledByAdmin") return "管理员已禁用";
  if (p.installed && p.enabled) return "已启用";
  if (p.installed) return "已安装（未启用）";
  return "未安装";
}

/** 已安装插件（跨市场汇总，携带来源市场用于展示；卸载只需插件 id） */
const installedPlugins = computed(() =>
  pluginState.marketplaces.flatMap((mp) =>
    mp.plugins.filter((p) => p.installed).map((plugin) => ({ mp, plugin })),
  ),
);

// 已安装列表最多显示 5 行、超出滚动：行高随描述/徽章可变，按实际行高计算容器 max-height
const INSTALLED_LIST_MAX_ROWS = 5;
const installedListEl = ref<HTMLElement | null>(null);
const installedListMaxHeight = ref("");

async function syncInstalledListHeight() {
  await nextTick();
  const rows = installedListEl.value
    ? Array.from(
        installedListEl.value.querySelectorAll<HTMLElement>(".plugin-row"),
      )
    : [];
  if (rows.length <= INSTALLED_LIST_MAX_ROWS) {
    installedListMaxHeight.value = "";
    return;
  }
  const h = rows
    .slice(0, INSTALLED_LIST_MAX_ROWS)
    .reduce((sum, row) => sum + row.offsetHeight, 0);
  // 分区处于 v-show 隐藏时 offsetHeight 全为 0，测量无效：保留现值，
  // 待切入插件管理分区（分区可见）后再由 active watch 重算
  if (h <= 0) return;
  installedListMaxHeight.value = `${h}px`;
}

watch(
  () => installedPlugins.value.map((x) => x.plugin.id).join(","),
  syncInstalledListHeight,
);

// 分区隐藏期测量无效（offsetHeight 恒为 0），切入时补算已安装列表限高
watch(
  () => props.active,
  (active) => {
    if (active) void syncInstalledListHeight();
  },
);

function canInstall(p: PluginCatalogItem): boolean {
  return p.availability !== "DisabledByAdmin";
}

// ---------- 插件行图标（仅用 plugin/list 接口字段，远程 URL 优先，其次本地路径，最后品牌色首字母回退） ----------
const brokenPluginIcons = ref(new Set<string>());

function pluginIconSrc(p: PluginCatalogItem): string {
  if (brokenPluginIcons.value.has(p.id)) return "";
  if (p.iconUrl) return p.iconUrl;
  if (p.iconPath) return assetUrl(p.iconPath);
  return "";
}

function markIconError(id: string) {
  const s = new Set(brokenPluginIcons.value);
  s.add(id);
  brokenPluginIcons.value = s;
}

function pluginInitial(p: PluginCatalogItem): string {
  return (p.displayName.trim()[0] ?? "P").toUpperCase();
}
</script>

<template>
  <section v-show="active" class="settings-section settings-section-plugins">
    <h2 class="settings-section-title">插件管理</h2>
    <p class="settings-section-desc">
      插件市场目录与本地安装管理
    </p>
    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>已安装插件</h3>
      </div>
      <div
        v-if="pluginState.loading && !installedPlugins.length"
        class="plugin-empty"
      >
        正在加载已安装插件…
      </div>
      <div v-else-if="!installedPlugins.length" class="plugin-empty">
        还没有已安装的插件，可在下方插件市场安装。
      </div>
      <div
        v-else
        ref="installedListEl"
        class="plugin-list installed-plugin-list"
        :style="{ maxHeight: installedListMaxHeight }"
      >
        <div
          v-for="{ mp, plugin: p } in installedPlugins"
          :key="p.id"
          class="plugin-row"
        >
          <span class="plugin-row-icon" aria-hidden="true">
            <img
              v-if="pluginIconSrc(p)"
              :src="pluginIconSrc(p)"
              alt=""
              loading="lazy"
              @error="markIconError(p.id)"
            />
            <span
              v-else
              class="plugin-icon-fallback"
              :style="
                p.brandColor ? { background: p.brandColor } : undefined
              "
            >
              {{ pluginInitial(p) }}
            </span>
          </span>
          <div class="plugin-info">
            <div class="plugin-name">
              {{ p.displayName }}
              <span v-if="p.version" class="plugin-version">
                {{ p.version }}
              </span>
            </div>
            <div v-if="p.description" class="plugin-desc">
              {{ p.description }}
            </div>
            <div class="plugin-meta">
              <span class="plugin-status">{{ statusLabel(p) }}</span>
              <span class="plugin-source">来源：{{ mp.displayName }}</span>
              <span
                v-if="p.disabledReason"
                class="plugin-disabled-reason"
              >
                {{ p.disabledReason }}
              </span>
            </div>
          </div>
          <button
            class="btn btn-icon danger plugin-uninstall-btn"
            :class="{ loading: !!pluginState.busy[p.id] }"
            v-tooltip="pluginState.busy[p.id] ? '卸载中…' : '卸载'"
            :aria-label="pluginState.busy[p.id] ? '卸载中' : '卸载'"
            :disabled="!!pluginState.busy[p.id]"
            @click="doUninstall(p)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_DELETE" />
            </svg>
          </button>
        </div>
      </div>
    </div>

    <div class="model-config-card">
      <div class="model-config-card-head">
        <h3>插件市场</h3>
        <div class="model-config-head-actions">
          <button
            class="btn btn-icon plugin-refresh-btn"
            :class="{ loading: pluginState.loading }"
            v-tooltip="'刷新目录'"
            aria-label="刷新目录"
            :disabled="pluginState.loading"
            @click="refreshPlugins(true)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_REFRESH" />
            </svg>
          </button>
        </div>
      </div>
      <div class="plugin-manage-toolbar">
        <div class="plugin-market-add">
          <input
            v-model="pluginState.source"
            placeholder="Git URL 或本地绝对路径"
            @keydown.enter="doAddMarketplace"
          />
          <button
            class="btn btn-icon primary plugin-market-add-btn"
            :class="{ loading: pluginState.adding }"
            v-tooltip="'添加市场'"
            aria-label="添加市场"
            :disabled="pluginState.adding || !pluginState.source.trim()"
            @click="doAddMarketplace"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path :d="ICON_PLUS" />
            </svg>
          </button>
        </div>
      </div>

      <div v-if="pluginState.loadErrors.length" class="plugin-load-errors">
        <p
          v-for="(err, i) in pluginState.loadErrors"
          :key="i"
          class="plugin-load-error"
        >
          市场「{{ err.name || "未知" }}」加载失败：{{ err.error }}
        </p>
      </div>

      <div
        v-if="pluginState.loading && !pluginState.marketplaces.length"
        class="plugin-empty"
      >
        正在加载插件目录…
      </div>
      <div v-else-if="!pluginState.marketplaces.length" class="plugin-empty">
        暂无可用市场
      </div>
      <div v-else class="plugin-marketplaces">
        <div
          v-for="mp in pluginState.marketplaces"
          :key="mp.name"
          class="plugin-marketplace"
        >
          <div
            class="plugin-marketplace-head"
            :class="{ collapsed: pluginState.collapsed[mp.name] }"
            @click="toggleMarketplace(mp)"
          >
            <span class="plugin-marketplace-arrow" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path
                  :d="
                    pluginState.collapsed[mp.name]
                      ? ICON_ARROW_RIGHT
                      : ICON_ARROW_DOWN
                  "
                />
              </svg>
            </span>
            <span class="plugin-marketplace-icon" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path :d="ICON_EXTENSION" />
              </svg>
            </span>
            <span class="plugin-marketplace-name">{{ mp.displayName }}</span>
            <span
              class="plugin-marketplace-count"
              v-tooltip="`${mp.plugins.length} 个插件`"
            >
              {{ mp.plugins.length }}
            </span>
            <button
              v-if="!mp.isRemote"
              class="btn btn-icon danger plugin-market-remove"
              aria-label="移除市场"
              v-tooltip="'移除市场'"
              @click.stop="doRemoveMarketplace(mp)"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path :d="ICON_DELETE" />
              </svg>
            </button>
          </div>
          <template v-if="!pluginState.collapsed[mp.name]">
            <div v-if="!mp.plugins.length" class="plugin-empty small">
              该市场暂无插件
            </div>
            <div v-else class="plugin-list">
              <div v-for="p in mp.plugins" :key="p.id" class="plugin-row">
                <span class="plugin-row-icon" aria-hidden="true">
                  <img
                    v-if="pluginIconSrc(p)"
                    :src="pluginIconSrc(p)"
                    alt=""
                    loading="lazy"
                    @error="markIconError(p.id)"
                  />
                  <span
                    v-else
                    class="plugin-icon-fallback"
                    :style="
                      p.brandColor ? { background: p.brandColor } : undefined
                    "
                  >
                    {{ pluginInitial(p) }}
                  </span>
                </span>
                <div class="plugin-info">
                  <div class="plugin-name">
                    {{ p.displayName }}
                    <span v-if="p.version" class="plugin-version">
                      {{ p.version }}
                    </span>
                  </div>
                  <div v-if="p.description" class="plugin-desc">
                    {{ p.description }}
                  </div>
                  <div class="plugin-meta">
                    <span class="plugin-status">{{ statusLabel(p) }}</span>
                    <span
                      v-if="p.disabledReason"
                      class="plugin-disabled-reason"
                    >
                      {{ p.disabledReason }}
                    </span>
                  </div>
                </div>
                <button
                  v-if="!p.installed"
                  class="btn btn-icon primary plugin-install-btn"
                  :class="{ loading: !!pluginState.busy[p.id] }"
                  v-tooltip="pluginState.busy[p.id] ? '安装中…' : '安装'"
                  :aria-label="pluginState.busy[p.id] ? '安装中' : '安装'"
                  :disabled="!canInstall(p) || !!pluginState.busy[p.id]"
                  @click="doInstall(mp, p)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_DOWNLOAD" />
                  </svg>
                </button>
                <button
                  v-else
                  class="btn btn-icon danger plugin-uninstall-btn"
                  :class="{ loading: !!pluginState.busy[p.id] }"
                  v-tooltip="pluginState.busy[p.id] ? '卸载中…' : '卸载'"
                  :aria-label="pluginState.busy[p.id] ? '卸载中' : '卸载'"
                  :disabled="!!pluginState.busy[p.id]"
                  @click="doUninstall(p)"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path :d="ICON_DELETE" />
                  </svg>
                </button>
              </div>
            </div>
          </template>
        </div>
      </div>
    </div>
  </section>
</template>
