
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { store } from "../../composables/useCodex";
import type { WeChatProtocolInfo } from "../../lib/types";

defineProps<{ active: boolean }>();

/** 应用版本（`tauri.conf.json` / `Cargo.toml` 的 version；非 Tauri 环境或失败回退） */
const appVersion = ref<string>("");
/** 后端已探测的 codex CLI 版本（`store.server.codexVersion`，未探测显示占位） */
const codexVersion = ref<string>("未知版本");
/** 微信接入协议（协议名 + 协议版本，来自后端 `wechat_protocol_info`；取不到显示占位） */
const wechatProtocol = ref<string>("未知");

/** 协议展示文案：协议名 · 协议版本 <channelVersion>（ClawBot 通道）；字段缺失返回占位 */
function formatWechatProtocol(
  info: WeChatProtocolInfo | null | undefined,
): string {
  const protocol = info?.protocol?.trim();
  const version = info?.channelVersion?.trim();
  if (!protocol || !version) return "未知";
  return `${protocol} · 协议版本 ${version}（ClawBot 通道）`;
}

onMounted(async () => {
  // 与 boot.ts 取标题版本同源：getVersion 失败静默回退「未知版本」
  try {
    const v = await getVersion();
    appVersion.value = v || "未知版本";
  } catch {
    appVersion.value = "未知版本";
  }
  codexVersion.value = store.server.codexVersion ?? "未知版本";
  // 协议版本只有后端一处事实源（每次微信请求的 base_info.channel_version）
  try {
    wechatProtocol.value = formatWechatProtocol(
      await invoke<WeChatProtocolInfo>("wechat_protocol_info"),
    );
  } catch {
    wechatProtocol.value = "未知";
  }
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
          <label>微信接入</label>
          <div class="about-value">{{ wechatProtocol }}</div>
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
