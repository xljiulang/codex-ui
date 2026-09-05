<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { PreviewEditorTab } from "../composables/useEditorTabs";

const props = defineProps<{ tab: PreviewEditorTab }>();

const loading = ref(true);
const error = ref("");
/** 滚动容器兼渲染宿主：docx-preview 把分页 DOM 注入其中 */
const stage = ref<HTMLDivElement | null>(null);
/** 渲染序号：外部刷新连发时丢弃过期渲染结果 */
let renderSeq = 0;

async function load() {
  const seq = ++renderSeq;
  loading.value = true;
  error.value = "";
  const data = props.tab.docxData;
  if (!data || !data.length) {
    error.value = "文档内容为空";
    loading.value = false;
    return;
  }
  // 外部刷新替换字节时保留滚动位置（与文本/PDF 刷新的恢复策略一致）
  const prevScroll = stage.value?.scrollTop ?? 0;
  try {
    // 传副本给 docx-preview：隔离标签字节与渲染库的潜在改动，
    // 保证切标签重挂载后 load() 仍能从 tab.docxData 完整重载
    const docx = await import("docx-preview");
    if (seq !== renderSeq || !stage.value) return;
    stage.value.innerHTML = "";
    await docx.renderAsync(
      data.slice(),
      stage.value,
      undefined,
      {
        inWrapper: true,
        breakPages: true,
        renderHeaders: true,
        renderFooters: true,
        ignoreLastRenderedPageBreak: false,
      },
    );
    if (seq !== renderSeq) return;
    await nextTick();
    if (stage.value) stage.value.scrollTop = prevScroll;
  } catch (e) {
    if (seq !== renderSeq) return;
    error.value = String(e);
  } finally {
    if (seq === renderSeq) loading.value = false;
  }
}

// 挂载后首次渲染（此时 stage 已挂载、docxData 已就绪）；
// 切换预览标签或外部刷新替换 docxData：重新渲染文档
onMounted(() => void load());
watch(
  [() => props.tab, () => props.tab.docxData],
  () => void load(),
);

onBeforeUnmount(() => {
  renderSeq++;
});
</script>

<template>
  <div class="docx-preview">
    <div v-if="loading" class="preview-note">正在渲染文档…</div>
    <div v-else-if="error" class="preview-note preview-error">
      无法预览该文档（{{ error }}）
    </div>
    <div v-show="!loading && !error" ref="stage" class="docx-preview-stage"></div>
  </div>
</template>
