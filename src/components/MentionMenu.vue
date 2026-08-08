<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { UserInput } from "../lib/types";
import {
  toUserAttachment,
  type FuzzyFileResult,
} from "../lib/mention";

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
interface SkillItem {
  name: string;
  key: string;
  path: string;
  desc: string;
}

const skills = ref<SkillItem[]>([]);
const loadingSkills = ref(true);

async function loadSkills() {
  loadingSkills.value = true;
  try {
    const res = await invoke<{
      data?: { skills?: SkillItem[] }[];
    }>("codex_rpc", { method: "skills/list", params: {} });
    skills.value = (res?.data ?? [])
      .flatMap((d) => d.skills ?? [])
      .filter((s) => (s as { enabled?: boolean }).enabled !== false)
      .map<SkillItem>((s) => ({
        name: s.name,
        key: s.name,
        path: s.path ?? "",
        desc: s.desc ?? "",
      }));
  } catch {
    // ignore
  } finally {
    loadingSkills.value = false;
  }
}

onMounted(loadSkills);

const tokenLower = computed(() => props.token.toLowerCase());
const filteredSkills = computed(() => {
  if (!props.token) return skills.value;
  return skills.value.filter(
    (s) =>
      s.name.toLowerCase().includes(tokenLower.value) ||
      s.key.toLowerCase().includes(tokenLower.value),
  );
});

// ---------------- @ 分支：文件引用（键盘可导航的扁平行列表） ----------------
type Row =
  | { kind: "file"; item: FuzzyFileResult }
  | { kind: "native-file" }
  | { kind: "native-dir" };

const rows = computed<Row[]>(() => {
  const list: Row[] = [];
  if (props.token) {
    for (const r of props.results) list.push({ kind: "file", item: r });
  }
  list.push({ kind: "native-file" });
  list.push({ kind: "native-dir" });
  return list;
});

const highlight = ref(0);
watch(
  [() => props.token, () => props.results],
  () => {
    highlight.value = 0;
  },
);

function move(dir: -1 | 1) {
  const len = rows.value.length;
  if (!len) return;
  highlight.value = (highlight.value + dir + len) % len;
}

function selectHighlighted() {
  const row = rows.value[highlight.value];
  if (!row) return;
  selectRow(row);
}

function selectRow(row: Row) {
  if (row.kind === "file") {
    const { root, path, file_name } = row.item;
    const full = root.endsWith("\\") || root.endsWith("/")
      ? root + path
      : root + "\\" + path;
    emit("select-attachment", toUserAttachment(file_name, full));
  } else if (row.kind === "native-file") {
    emit("pick-files");
  } else {
    emit("pick-dir");
  }
}

defineExpose({ move, selectHighlighted });

function rowIcon(row: Row): string {
  if (row.kind === "file") return row.item.match_type === "directory" ? "D" : "F";
  return row.kind === "native-file" ? "F" : "D";
}
function rowLabel(row: Row): string {
  if (row.kind === "file") return row.item.file_name;
  return row.kind === "native-file" ? "选择文件…" : "选择文件夹…";
}
function rowDesc(row: Row): string {
  if (row.kind === "file") return row.item.path;
  return "";
}
function rowKey(row: Row): string {
  if (row.kind === "file") return "f:" + row.item.path;
  return row.kind;
}
</script>

<template>
  <div class="popup-menu mention-menu" @click.stop>
    <template v-if="kind === '@'">
      <div class="menu-group-title">引用文件</div>
      <div v-if="!token" class="menu-note mention-hint">
        输入文件名搜索，或从本地选择
      </div>
      <div v-else class="menu-note mention-hint">搜索“{{ token }}”…</div>

      <div class="menu-results">
        <button
          v-for="(row, i) in rows"
          :key="rowKey(row)"
          class="menu-item"
          :class="{ active: highlight === i }"
          @mouseenter="highlight = i"
          @click="selectRow(row)"
        >
          <span class="menu-item-icon">
            {{ rowIcon(row) }}
          </span>
          <span>
            <div class="menu-item-label">{{ rowLabel(row) }}</div>
            <div v-if="rowDesc(row)" class="menu-item-desc">{{ rowDesc(row) }}</div>
          </span>
        </button>
        <div v-if="token && searching" class="menu-note">搜索中…</div>
        <div v-else-if="token && !results.length" class="menu-note">无匹配文件</div>
      </div>
    </template>

    <template v-else>
      <div class="menu-group">
        <div class="menu-group-title">技能（使用 $ 调用 Skills）</div>
        <button
          v-for="s in filteredSkills"
          :key="s.key"
          class="menu-item"
          @click="emit('select-attachment', { type: 'skill', name: s.key, path: s.path })"
        >
          <span class="menu-item-icon">S</span>
          <span>
            <div class="menu-item-label">{{ s.name }}</div>
            <div class="menu-item-desc">{{ s.desc }}</div>
          </span>
        </button>
        <div v-if="loadingSkills" class="menu-note">加载中…</div>
        <div v-else-if="!filteredSkills.length" class="menu-note">无匹配技能</div>
      </div>
    </template>
  </div>
</template>
