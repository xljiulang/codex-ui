<script setup lang="ts">
import { ref } from "vue";
import { saveSettings, setToast, store, toastError } from "../../composables/useCodex";
import { THEMES, previewTheme, type ThemeId } from "../../composables/useTheme";
import { ICON_CHECK } from "../../lib/icons";
import type { AppSettings } from "../../lib/types";

defineProps<{ active: boolean }>();

const sound = ref(store.settings.sound_enabled);
const errorNotify = ref(store.settings.error_notify_enabled ?? true);
const enterToSend = ref(store.settings.enter_to_send);
const theme = ref<ThemeId>(store.settings.theme as ThemeId);
const glass = ref(store.settings.glass_effect);

/** 即时保存：任何设置项变更立即持久化（成功静默，失败 toast） */
async function persist(patch: Partial<AppSettings>) {
  try {
    await saveSettings(patch);
  } catch (e) {
    setToast(toastError(e));
  }
}

function selectTheme(id: ThemeId) {
  // 即时预览 + 立即持久化（saveSettings 内部 applyTheme 兜底一致）
  theme.value = id;
  previewTheme(id);
  void persist({ theme: id });
}
</script>

<template>
  <section
    v-show="active"
    class="settings-section settings-section-personalization"
  >
    <h2 class="settings-section-title">个性化</h2>
    <p class="settings-section-desc">
      主题、音效与消息发送等个性化偏好
    </p>
    <div class="settings-card">
      <div class="settings">
        <div class="setting-row switch-row">
          <label class="switch">
            <input
              id="sound"
              v-model="sound"
              type="checkbox"
              @change="persist({ sound_enabled: sound })"
            />
            <span class="switch-track"></span>
          </label>
          <label class="switch-text" for="sound">
            提权/交互时播放提示音
          </label>
        </div>

        <div class="setting-row switch-row">
          <label class="switch">
            <input
              id="error-notify"
              v-model="errorNotify"
              type="checkbox"
              @change="persist({ error_notify_enabled: errorNotify })"
            />
            <span class="switch-track"></span>
          </label>
          <label class="switch-text" for="error-notify">
            codex 错误发系统通知（窗口没有前台焦点时）
          </label>
        </div>

        <div class="setting-row switch-row">
          <label class="switch">
            <input
              id="enter"
              v-model="enterToSend"
              type="checkbox"
              @change="persist({ enter_to_send: enterToSend })"
            />
            <span class="switch-track"></span>
          </label>
          <label class="switch-text" for="enter">
            Enter 快捷发送（开启时 Ctrl+Enter 换行；关闭后 Enter 换行，Ctrl+Enter 发送）
          </label>
        </div>

        <div class="setting-row">
          <div class="theme-row-head">
            <label class="switch">
              <input
                id="glass"
                v-model="glass"
                type="checkbox"
                @change="persist({ glass_effect: glass })"
              />
              <span class="switch-track"></span>
            </label>
            <label class="switch-text" for="glass">毛玻璃主题外观</label>
          </div>
          <div class="theme-picker">
            <button
              v-for="t in THEMES"
              :key="t.id"
              class="theme-card"
              :class="{ selected: theme === t.id }"
              :data-theme-id="t.id"
              :aria-pressed="theme === t.id"
              @click="selectTheme(t.id)"
            >
              <span class="theme-swatch"></span>
              <span class="theme-name">{{ t.name }}</span>
              <span class="theme-desc">{{ t.desc }}</span>
              <span v-if="theme === t.id" class="theme-check" aria-hidden="true">
                <svg viewBox="0 0 24 24">
                  <path :d="ICON_CHECK" />
                </svg>
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.theme-picker {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
}
.theme-card {
  flex: 1 1 150px;
  min-width: 120px;
  text-align: left;
  padding: var(--space-3);
  border-radius: var(--radius-lg);
  border: 1px solid var(--border);
  background: var(--bg-input);
  box-shadow: var(--shadow-sm), var(--inset-shadow);
  transition: border-color var(--ease), box-shadow var(--ease),
    transform var(--ease);
}
.theme-card:hover {
  border-color: var(--border-light);
  transform: translateY(-1px);
  box-shadow: var(--shadow-hover), var(--inset-shadow);
}
.theme-card.selected {
  border-color: var(--accent);
  box-shadow: var(--shadow-accent), var(--inset-shadow);
}
.theme-swatch {
  display: block;
  position: relative;
  height: 46px;
  border-radius: var(--radius);
  border: 1px solid var(--border-light);
  margin-bottom: var(--space-3);
  overflow: hidden;
}
.theme-swatch::after {
  content: "";
  position: absolute;
  left: 10%;
  bottom: 9px;
  width: 38%;
  height: 7px;
  border-radius: 999px;
}
.theme-card[data-theme-id="blue"] .theme-swatch {
  background: #0e1116;
}
.theme-card[data-theme-id="blue"] .theme-swatch::after {
  background: #4da6ff;
}
.theme-card[data-theme-id="dark"] .theme-swatch {
  background: #0a0b10;
}
.theme-card[data-theme-id="dark"] .theme-swatch::after {
  background: #a78bfa;
}
.theme-card[data-theme-id="light"] .theme-swatch {
  background: #f4f6fb;
}
.theme-card[data-theme-id="light"] .theme-swatch::after {
  background: #2563eb;
}
.theme-name {
  display: block;
  font-size: var(--font-md);
  font-weight: 600;
  color: var(--text-bright);
}
.theme-desc {
  display: block;
  margin-top: 2px;
  font-size: var(--font-sm);
  color: var(--text-faint);
}
/* 开关在左、文字在右（仅本 Tab 的排布；其它分区仍是文字在左、开关在右） */
.switch-row {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

/* 抵消全局 .setting-row label 的 margin-bottom（文字标签与开关标签） */
.switch-row .switch,
.switch-text {
  margin-bottom: 0;
}

/* 抵消全局 .setting-row label 的 600 字重 / --text-dim，沿用原 checkbox 行观感 */
.switch-text {
  font-size: var(--font-md);
  font-weight: 400;
  line-height: 1.5;
  color: var(--text);
}

/* 主题行头部：毛玻璃开关在左、标题在右，同行并列 */
.theme-row-head {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: var(--space-4);
  margin-bottom: var(--space-4);
}
</style>
