<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { UserInput } from "../lib/types";

const props = defineProps<{ kind: "@" | "$"; token: string }>();
const emit = defineEmits<{
  close: [];
  "pick-files": [];
  "pick-dir": [];
  "select-attachment": [attachment: UserInput];
}>();

interface Item {
  name: string;
  key: string;
  path: string;
  desc: string;
}

const plugins = ref<Item[]>([]);
const skills = ref<Item[]>([]);
const loading = ref(true);

async function load() {
  loading.value = true;
  try {
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
      }>("codex_rpc", { method: "plugin/list", params: {} });
      const list = (res?.marketplaces ?? [])
        .flatMap((mp) => mp.plugins ?? [])
        .filter((p) => p.installed !== false && p.enabled !== false)
        .map<Item>((p) => ({
          name: p.interface?.displayName ?? p.name,
          key: p.name,
          path: p.source?.path ?? "",
          desc:
            p.interface?.shortDescription ?? p.interface?.longDescription ?? "",
        }));
      const seen = new Set<string>();
      plugins.value = list.filter((p) => {
        if (seen.has(p.key)) return false;
        seen.add(p.key);
        return true;
      });
    } catch {
      // ignore
    }
    try {
      const res = await invoke<{
        data?: {
          skills?: {
            name: string;
            description?: string;
            path?: string;
            enabled?: boolean;
          }[];
        }[];
      }>("codex_rpc", { method: "skills/list", params: {} });
      skills.value = (res?.data ?? [])
        .flatMap((d) => d.skills ?? [])
        .filter((s) => s.enabled !== false)
        .map<Item>((s) => ({
          name: s.name,
          key: s.name,
          path: s.path ?? "",
          desc: s.description ?? "",
        }));
    } catch {
      // ignore
    }
  } finally {
    loading.value = false;
  }
}

onMounted(load);

const tokenLower = computed(() => props.token.toLowerCase());
function match(item: Item): boolean {
  if (!props.token) return true;
  return (
    item.name.toLowerCase().includes(tokenLower.value) ||
    item.key.toLowerCase().includes(tokenLower.value)
  );
}
const filteredPlugins = computed(() => plugins.value.filter(match));
const filteredSkills = computed(() => skills.value.filter(match));
</script>

<template>
  <div class="popup-menu mention-menu" @click.stop>
    <template v-if="kind === '@'">
      <div class="menu-group">
        <div class="menu-group-title">文件</div>
        <button class="menu-item" @click="emit('pick-files')">
          <span class="menu-item-icon">F</span>
          <span>
            <div class="menu-item-label">选择文件…</div>
            <div class="menu-item-desc">从本地选择文件作为上下文</div>
          </span>
        </button>
        <button class="menu-item" @click="emit('pick-dir')">
          <span class="menu-item-icon">D</span>
          <span>
            <div class="menu-item-label">选择文件夹…</div>
            <div class="menu-item-desc">从本地选择文件夹作为上下文</div>
          </span>
        </button>
      </div>
      <div class="menu-group">
        <div class="menu-group-title">插件</div>
        <button
          v-for="p in filteredPlugins"
          :key="p.key"
          class="menu-item"
          @click="emit('select-attachment', { type: 'skill', name: p.key, path: p.path })"
        >
          <span class="menu-item-icon">P</span>
          <span>
            <div class="menu-item-label">{{ p.name }}</div>
            <div class="menu-item-desc">{{ p.desc }}</div>
          </span>
        </button>
        <div v-if="loading" class="menu-note">加载中…</div>
        <div v-else-if="!filteredPlugins.length" class="menu-note">无匹配插件</div>
      </div>
      <div class="menu-group">
        <div class="menu-group-title">技能</div>
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
        <div v-if="loading" class="menu-note">加载中…</div>
        <div v-else-if="!filteredSkills.length" class="menu-note">无匹配技能</div>
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
        <div v-if="loading" class="menu-note">加载中…</div>
        <div v-else-if="!filteredSkills.length" class="menu-note">无匹配技能</div>
      </div>
    </template>
  </div>
</template>
