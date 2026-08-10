<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { UserInput } from "../lib/types";
import {
  toUserAttachment,
  type FuzzyFileResult,
} from "../lib/mention";
import {
  NEW_CHAT_PLUGIN_KEY,
  ensureSkills,
  store,
  type PluginItem,
} from "../composables/useCodex";

const props = defineProps<{
  kind: "@" | "$";
  token: string;
  results: FuzzyFileResult[];
  searching: boolean;
}>();

const emit = defineEmits<{
  close: [];
  "pick-files": [];
  "pick-dir": [];
  "select-attachment": [attachment: UserInput];
}>();

// ---------------- $ 分支：技能列表（保持原行为） ----------------
/** 首次打开时确保技能缓存已加载（幂等，之后复用） */
onMounted(() => {
  void ensureSkills();
});

const loadingSkills = computed(
  () => !store.skillsLoaded && store.skills.length === 0,
);

const tokenLower = computed(() => props.token.toLowerCase());
const filteredSkills = computed(() => {
  if (!props.token) return store.skills;
  return store.skills.filter(
    (s) =>
      s.name.toLowerCase().includes(tokenLower.value) ||
      s.key.toLowerCase().includes(tokenLower.value),
  );
});

// ---------------- @ 分支：文件 + 插件（对话级缓存联合搜索） ----------------
type Row =
  | { kind: "file"; item: FuzzyFileResult }
  | { kind: "native-file" }
  | { kind: "native-dir" }
  | { kind: "plugin"; plugin: PluginItem };

/** 当前对话的插件缓存（未创建会话用 NEW_CHAT_PLUGIN_KEY） */
const currentThreadPlugins = computed(() => {
  const key = store.currentThreadId ?? NEW_CHAT_PLUGIN_KEY;
  return store.threadPlugins[key]?.plugins ?? [];
});

/** 缓存插件按 token 过滤（名称/显示名/描述），空 token 显示全部 */
const filteredPlugins = computed(() => {
  const all = currentThreadPlugins.value;
  if (!props.token) return all;
  return all.filter(
    (p) =>
      p.name.toLowerCase().includes(tokenLower.value) ||
      p.displayName.toLowerCase().includes(tokenLower.value) ||
      p.description.toLowerCase().includes(tokenLower.value),
  );
});

/**
 * 固定行始终在最前；有 token 时命中插件在前、命中文件在后（联合搜索）；
 * 无 token 时固定行后直接接全部插件。
 */
const rows = computed<Row[]>(() => {
  const list: Row[] = [{ kind: "native-file" }, { kind: "native-dir" }];
  for (const p of filteredPlugins.value) {
    list.push({ kind: "plugin", plugin: p });
  }
  if (props.token) {
    for (const r of props.results) list.push({ kind: "file", item: r });
  }
  return list;
});

/** 插件区行数（固定行之后），用于在扁平列表里插入“文件”分组标题 */
const pluginRowCount = computed(() => filteredPlugins.value.length);

const highlight = ref(0);
watch(
  [
    () => props.kind,
    () => props.token,
    () => props.results,
    filteredPlugins,
    filteredSkills,
  ],
  () => {
    highlight.value = 0;
  },
);

function move(dir: -1 | 1) {
  const len =
    props.kind === "@" ? rows.value.length : filteredSkills.value.length;
  if (!len) return;
  highlight.value = (highlight.value + dir + len) % len;
}

function selectHighlighted() {
  if (props.kind === "@") {
    const row = rows.value[highlight.value];
    if (!row) return;
    selectRow(row);
    return;
  }
  const s = filteredSkills.value[highlight.value];
  if (!s) return;
  emit("select-attachment", {
    type: "skill",
    name: s.key,
    path: s.path,
    source: "skill",
  });
}

defineExpose({ move, selectHighlighted });

function selectRow(row: Row) {
  if (row.kind === "file") {
    const { root, path, file_name } = row.item;
    const full = root.endsWith("\\") || root.endsWith("/")
      ? root + path
      : root + "\\" + path;
    emit("select-attachment", toUserAttachment(file_name, full));
  } else if (row.kind === "native-file") {
    emit("pick-files");
  } else if (row.kind === "native-dir") {
    emit("pick-dir");
  } else {
    emit("select-attachment", {
      type: "skill",
      name: row.plugin.name,
      path: row.plugin.path,
      source: "plugin",
      pluginId: row.plugin.id,
    });
  }
}

function rowIcon(row: Row): string {
  if (row.kind === "file") return row.item.match_type === "directory" ? "D" : "F";
  if (row.kind === "plugin") return "";
  return row.kind === "native-file" ? "F" : "D";
}

function rowLabel(row: Row): string {
  if (row.kind === "file") return row.item.file_name;
  if (row.kind === "plugin") return row.plugin.displayName;
  return row.kind === "native-file" ? "选择文件…" : "选择文件夹…";
}

function rowDesc(row: Row): string {
  if (row.kind === "file") return row.item.path;
  if (row.kind === "plugin") return row.plugin.description;
  if (row.kind === "native-file") return "从本地选择文件作为上下文";
  return "从本地选择文件夹作为上下文";
}

function rowKey(row: Row): string {
  if (row.kind === "file") return "f:" + row.item.path;
  if (row.kind === "plugin") return "p:" + row.plugin.id;
  return row.kind;
}

// ---------------- 插件接口图标 ----------------
const brokenIcons = ref(new Set<string>());

function pluginIconSrc(p: PluginItem): string {
  if (brokenIcons.value.has(p.id)) return "";
  if (p.iconUrl) return p.iconUrl;
  if (p.iconPath) return convertFileSrc(p.iconPath);
  return "";
}

function markIconError(id: string) {
  const s = new Set(brokenIcons.value);
  s.add(id);
  brokenIcons.value = s;
}

function pluginInitial(p: PluginItem): string {
  return (p.displayName.trim()[0] ?? "P").toUpperCase();
}
</script>

<template>
  <div class="popup-menu mention-menu" @click.stop>
    <template v-if="kind === '@'">
      <div class="menu-group-title">引用文件</div>
      <div v-if="!token" class="menu-note mention-hint">
        输入文件名或插件名搜索，或从本地选择
      </div>
      <div v-else class="menu-note mention-hint">搜索“{{ token }}”…</div>

      <div class="menu-results">
        <template v-for="(row, i) in rows" :key="rowKey(row)">
          <div v-if="i === 2 && pluginRowCount > 0" class="menu-group-title">
            插件
          </div>
          <div
            v-else-if="
              i === 2 + pluginRowCount && token && results.length > 0
            "
            class="menu-group-title"
          >
            文件
          </div>
          <button
            class="menu-item"
            :class="{ active: highlight === i }"
            @mouseenter="highlight = i"
            @click="selectRow(row)"
          >
            <span v-if="row.kind === 'plugin'" class="menu-item-icon plugin-icon">
              <img
                v-if="pluginIconSrc(row.plugin)"
                :src="pluginIconSrc(row.plugin)"
                alt=""
                @error="markIconError(row.plugin.id)"
              />
              <span
                v-else
                class="plugin-icon-fallback"
                :style="
                  row.plugin.brandColor
                    ? { background: row.plugin.brandColor }
                    : undefined
                "
              >
                {{ pluginInitial(row.plugin) }}
              </span>
            </span>
            <span v-else class="menu-item-icon">
              {{ rowIcon(row) }}
            </span>
            <span>
              <div class="menu-item-label">{{ rowLabel(row) }}</div>
              <div v-if="rowDesc(row)" class="menu-item-desc">{{ rowDesc(row) }}</div>
            </span>
          </button>
        </template>

        <div v-if="token && searching" class="menu-note">搜索中…</div>
        <div v-else-if="token && !results.length" class="menu-note">无匹配文件</div>
        <div v-if="token && !filteredPlugins.length" class="menu-note">
          无匹配插件
        </div>
        <div v-if="!token && !currentThreadPlugins.length" class="menu-note">
          暂无可用插件
        </div>
      </div>
    </template>

    <template v-else>
      <div class="menu-group">
        <div class="menu-group-title">技能（使用 $ 调用 Skills）</div>
        <button
          v-for="(s, i) in filteredSkills"
          :key="s.key"
          class="menu-item"
          :class="{ active: highlight === i }"
          @mouseenter="highlight = i"
          @click="
            emit('select-attachment', {
              type: 'skill',
              name: s.key,
              path: s.path,
              source: 'skill',
            })
          "
        >
          <span class="menu-item-icon">S</span>
          <span>
            <div class="menu-item-label">{{ s.name }}</div>
            <div class="menu-item-desc">{{ s.shortDesc }}</div>
          </span>
        </button>
        <div v-if="loadingSkills" class="menu-note">加载中…</div>
        <div v-else-if="!filteredSkills.length" class="menu-note">无匹配技能</div>
      </div>
    </template>
  </div>
</template>
