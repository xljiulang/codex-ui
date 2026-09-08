<script setup lang="ts">
import { computed, reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { setToast } from "../composables/useCodex";
import type { PendingInteraction } from "../lib/types";
import { list, obj, str } from "../lib/interaction";
import AppSelect, { type AppSelectOption } from "./AppSelect.vue";

/** 布尔下拉选项（值沿用字符串 "true"/"false"，提交时转换） */
const booleanOptions: AppSelectOption[] = [
  { value: "true", label: "是" },
  { value: "false", label: "否" },
];

const props = defineProps<{ interaction: PendingInteraction }>();

const emit = defineEmits<{
  /** 提交表单内容（url 模式为 null） */
  submit: [content: Record<string, unknown> | null];
  cancel: [];
}>();

const params = computed(() => props.interaction.params);
const formValues = reactive<Record<string, string>>({});
/** 多选 enum（schema type=array）的选中项 */
const formMulti = reactive<Record<string, string[]>>({});

function schemaProperties(): [string, Record<string, unknown>][] {
  const schema = obj(params.value.requestedSchema);
  return Object.entries(obj(schema.properties)).map(([k, v]) => [k, obj(v)]);
}

function requiredFields(): string[] {
  return list(obj(obj(params.value.requestedSchema).required)).map(str);
}

function isRequired(key: string): boolean {
  return requiredFields().includes(key);
}

/** 单选/多选 enum schema：oneOf/anyOf（const+title）或 enum（+enumNames） */
function isEnumSchema(s: Record<string, unknown>): boolean {
  return (
    Array.isArray(s.oneOf) ||
    Array.isArray(s.anyOf) ||
    Array.isArray(s.enum)
  );
}

function isMultiEnum(s: Record<string, unknown>): boolean {
  return s.type === "array" && isEnumSchema(obj(s.items));
}

function enumOptions(s: Record<string, unknown>): { value: string; title: string }[] {
  const oneOf = list(s.oneOf).map(obj).filter((o) => o.const !== undefined);
  if (oneOf.length) {
    return oneOf.map((o) => ({
      value: str(o.const),
      title: str(o.title ?? o.const),
    }));
  }
  const anyOf = list(s.anyOf).map(obj).filter((o) => o.const !== undefined);
  if (anyOf.length) {
    return anyOf.map((o) => ({
      value: str(o.const),
      title: str(o.title ?? o.const),
    }));
  }
  const enums = list(s.enum).map(str);
  const names = list(s.enumNames).map(str);
  return enums.map((v, i) => ({ value: v, title: names[i] || v }));
}

/** schema 枚举转 AppSelect 选项（value/title → value/label） */
function enumSelectOptions(s: Record<string, unknown>): AppSelectOption[] {
  return enumOptions(s).map((o) => ({ value: o.value, label: o.title }));
}

/** 缺失的必填字段标题；全部填齐返回 null */
function missingRequired(): string | null {
  const schema = obj(params.value.requestedSchema);
  const props = obj(schema.properties);
  for (const key of requiredFields()) {
    const prop = obj(props[key]);
    if (Object.keys(prop).length === 0) return key;
    if (isMultiEnum(prop)) {
      if (!(formMulti[key] ?? []).length) return str(prop.title) || key;
      continue;
    }
    if (prop.type === "boolean") continue; // 布尔始终有值（默认 false）
    if (prop.type === "number" || prop.type === "integer") {
      if ((formValues[key] ?? "") === "") return str(prop.title) || key;
      continue;
    }
    if (!(formValues[key] ?? "").trim()) return str(prop.title) || key;
  }
  return null;
}

/** 按 schema 类型把表单值转成结构化 content：数字/布尔保留类型，可选空字段省略 */
function buildFormContent(): Record<string, unknown> {
  const content: Record<string, unknown> = {};
  for (const [key, prop] of schemaProperties()) {
    const raw = formValues[key] ?? "";
    if (isMultiEnum(prop)) {
      const sel = formMulti[key] ?? [];
      const def = list(prop.default).map(str);
      const v = sel.length ? sel : def;
      if (v.length) content[key] = v;
      continue;
    }
    if (prop.type === "boolean") {
      const def = typeof prop.default === "boolean" ? prop.default : false;
      content[key] = raw === "" ? def : raw === "true";
      continue;
    }
    if (prop.type === "number" || prop.type === "integer") {
      if (raw !== "") content[key] = Number(raw);
      else if (typeof prop.default === "number") content[key] = prop.default;
      continue;
    }
    // string / 单选 enum
    if (raw !== "") content[key] = raw;
    else if (typeof prop.default === "string") content[key] = prop.default;
  }
  return content;
}

async function handleElicitation() {
  const mode = obj(params.value).mode ?? "form";
  if (mode === "url") {
    try {
      await invoke("open_url", { url: str(params.value.url) });
    } catch {
      // ignore
    }
    // url 模式：无表单字段，接受时返回空内容
    emit("submit", null);
    return;
  }
  const missing = missingRequired();
  if (missing) {
    setToast(`请填写必填字段：${missing}`);
    return;
  }
  // 协议：action 仅 accept/decline/cancel，content 为结构化 JSON 值（表单字段对象）
  emit("submit", buildFormContent());
}
</script>

<template>
  <div class="approval-hero">
    <div class="approval-label">来自 {{ params.serverName }} 的请求</div>
    <div v-if="params.message" class="approval-reason">{{ params.message }}</div>
  </div>
  <template v-if="obj(params).mode !== 'url'">
    <div v-for="[key, prop] in schemaProperties()" :key="key" class="question-row">
      <div class="question-text">
        {{ prop.title ?? key }}
        <span v-if="isRequired(key)" class="required-mark"> *</span>
      </div>
      <AppSelect
        v-if="isEnumSchema(prop)"
        v-model="formValues[key]"
        placeholder="请选择"
        :options="enumSelectOptions(prop)"
      />
      <div v-else-if="isMultiEnum(prop)" class="multi-options">
        <label
          v-for="opt in enumOptions(obj(prop.items))"
          :key="opt.value"
          class="option-btn"
        >
          <input type="checkbox" :value="opt.value" v-model="formMulti[key]" />
          {{ opt.title }}
        </label>
      </div>
      <AppSelect
        v-else-if="prop.type === 'boolean'"
        v-model="formValues[key]"
        :options="booleanOptions"
      />
      <input
        v-else-if="prop.type === 'number' || prop.type === 'integer'"
        v-model="formValues[key]"
        type="number"
        :placeholder="str(prop.description)"
      />
      <input
        v-else
        v-model="formValues[key]"
        :type="prop.format === 'password' ? 'password' : 'text'"
        style="width: 100%"
        :placeholder="str(prop.description)"
      />
    </div>
  </template>
  <div v-else class="approval-meta" style="margin-top: 6px">
    将在浏览器中打开：{{ params.url }}
  </div>

  <div class="interaction-foot">
    <button class="btn" @click="emit('cancel')">拒绝</button>
    <button class="btn primary" @click="handleElicitation()">
      {{ obj(params).mode === "url" ? "已在浏览器打开" : "提交" }}
    </button>
  </div>
</template>

<style scoped>
.required-mark {
  color: var(--red);
  font-weight: 700;
}

.multi-options {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.multi-options .option-btn {
  width: auto;
  cursor: pointer;
}

.multi-options .option-btn::before {
  border-radius: var(--radius-sm);
}

.multi-options .option-btn:has(input:checked) {
  border-color: var(--accent-dim);
  color: var(--text-bright);
  background: rgba(var(--accent-rgb), 0.06);
}
</style>
