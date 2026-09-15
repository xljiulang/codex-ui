
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { store } from "../../composables/useCodex";

defineProps<{ active: boolean }>();

/** 应用版本（`tauri.conf.json` / `Cargo.toml` 的 version；非 Tauri 环境或失败回退） */
const appVersion = ref<string>("");
/** 后端已探测的 codex CLI 版本（`store.server.codexVersion`，未探测显示占位） */
const codexVersion = ref<string>("未知版本");

onMounted(async () => {
  // 与 boot.ts 取标题版本同源：getVersion 失败静默回退「未知版本」
  try {
    const v = await getVersion();
    appVersion.value = v || "未知版本";
  } catch {
    appVersion.value = "未知版本";
  }
  codexVersion.value = store.server.codexVersion ?? "未知版本";
});
</script>

<template>
  <section v-show="active" class="settings-section settings-section-about">
    <h2 class="settings-section-title">关于</h2>
    <p class="settings-section-desc">Codex UI 版本与应用信息</p>

    <div class="settings-card about-card">
      <div class="model-config-card-head">
        <div class="about-head-main">
          <h3>Codex UI</h3>
          <p class="about-tagline">由 xljiulang 100% vibe coding 而成</p>
        </div>
      </div>
      <div class="settings about-list">
        <div class="setting-row about-row">
          <label>应用版本</label>
          <div class="about-value">{{ appVersion }}</div>
        </div>
        <div class="setting-row about-row">
          <label>codex CLI 版本</label>
          <div class="about-value">{{ codexVersion }}</div>
        </div>
        <div class="setting-row about-row">
          <label>技术栈</label>
          <div class="about-value">Tauri 2 · Rust · Vue 3 · TypeScript</div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.about-card {
  gap: var(--space-3);
}
/* 卡片头：标题与副标题竖排（与 Zen 代理分区的 head-main / head-desc 同款取值） */
.about-head-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}
.about-tagline {
  margin: 0;
  font-size: var(--font-md);
  line-height: 1.5;
  color: var(--text-dim);
  -webkit-user-select: text;
  user-select: text;
}
.about-list {
  width: 100%;
  gap: 0;
  margin-top: var(--space-4);
}
.about-row {
  text-align: left;
}
.about-row label {
  margin-bottom: var(--space-1);
}
.about-value {
  font-size: var(--font-md);
  color: var(--text-bright);
  word-break: break-all;
  user-select: text;
}
</style>
