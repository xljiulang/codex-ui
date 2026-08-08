<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { store } from "../composables/useCodex";
import { baseName, toUserAttachment } from "../lib/mention";

const emit = defineEmits<{ close: [] }>();

interface SkillItem {
  name: string;
  key?: string;
  description?: string;
  path?: string;
  enabled?: boolean;
}

const skills = ref<SkillItem[]>([]);
const loading = ref(true);

async function loadPlugins() {
  loading.value = true;
  try {
    let list: SkillItem[] = [];
    try {
      const res = await invoke<{
        marketplaces?: {
          plugins?: {
            name: string;
            installed?: boolean;
            enabled?: boolean;
            source?: { path?: string };
            interface?: {
              displayName?: string;
              shortDescription?: string;
              longDescription?: string;
            };
          }[];
        }[];
      }>("codex_rpc", {
        method: "plugin/list",
        params: {},
      });
      list = (res?.marketplaces ?? [])
        .flatMap((mp) => mp.plugins ?? [])
        .filter((p) => p.installed !== false && p.enabled !== false)
        .map<SkillItem>((p) => ({
          name: p.interface?.displayName ?? p.name,
          key: p.name,
          description:
            p.interface?.shortDescription ?? p.interface?.longDescription ?? "",
          path: p.source?.path ?? "",
        }));
      // 按名称去重
      const seen = new Set<string>();
      list = list.filter((p) => {
        if (seen.has(p.key ?? p.name)) return false;
        seen.add(p.key ?? p.name);
        return true;
      });
    } catch {
      // plugin/list 失败时回退到 skills/list
    }
    if (!list.length) {
      try {
        const res = await invoke<{
          data?: { skills?: SkillItem[] }[];
        }>("codex_rpc", {
          method: "skills/list",
          params: {},
        });
        list = (res?.data ?? [])
          .flatMap((d) => d.skills ?? [])
          .filter((s) => s.enabled !== false)
          .map((s) => ({ ...s, key: s.name }));
      } catch {
        // ignore
      }
    }
    skills.value = list;
  } finally {
    loading.value = false;
  }
}

onMounted(loadPlugins);

function initialDir(): string {
  return (
    store.newChatCwd ?? store.currentThreadCwd ?? store.server.workspace ?? ""
  );
}

async function pickFiles() {
  try {
    const files = await invoke<string[]>("pick_files", {
      multiple: true,
      initialDir: initialDir(),
    });
    for (const f of files) {
      store.attachments.push(toUserAttachment(baseName(f), f));
    }
  } catch (e) {
    store.toast = String(e);
  }
  emit("close");
}

async function pickFolder() {
  try {
    const dir = await invoke<string | null>("pick_directory", {
      initialDir: initialDir(),
    });
    if (dir) {
      store.attachments.push(toUserAttachment(baseName(dir), dir));
    }
  } catch (e) {
    store.toast = String(e);
  }
  emit("close");
}

function addSkill(s: SkillItem) {
  store.attachments.push({
    type: "skill",
    name: s.key ?? s.name,
    path: s.path ?? "",
  });
  emit("close");
}
</script>

<template>
  <div class="popup-menu" @click.stop>
    <div class="menu-group">
      <div class="menu-group-title">添加</div>
      <button class="menu-item" @click="pickFiles()">
        <span class="menu-item-icon">F</span>
        <span>
          <div class="menu-item-label">添加文件…</div>
          <div class="menu-item-desc">从本地选择文件作为上下文</div>
        </span>
      </button>
      <button class="menu-item" @click="pickFolder()">
        <span class="menu-item-icon">D</span>
        <span>
          <div class="menu-item-label">添加文件夹…</div>
          <div class="menu-item-desc">从本地选择文件夹作为上下文</div>
        </span>
      </button>
    </div>

    <div class="menu-group">
      <div class="menu-group-title">插件</div>
      <button v-for="s in skills" :key="s.name" class="menu-item" @click="addSkill(s)">
        <span class="menu-item-icon">P</span>
        <span>
          <div class="menu-item-label">{{ s.name }}</div>
          <div class="menu-item-desc">{{ s.description ?? "" }}</div>
        </span>
      </button>
      <div v-if="loading" class="menu-note">加载中…</div>
      <div v-else-if="!skills.length" class="menu-note">暂无可用插件</div>
    </div>
  </div>
</template>
