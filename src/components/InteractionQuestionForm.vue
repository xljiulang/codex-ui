<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import type { PendingInteraction } from "../lib/types";
import { str } from "../lib/interaction";

const props = defineProps<{ interaction: PendingInteraction }>();

const emit = defineEmits<{
  /** 提交答案（协议：{ answers: { [questionId]: { answers: [选择] } } }） */
  submit: [answers: Record<string, { answers: string[] }>];
  cancel: [];
}>();

const params = computed(() => props.interaction.params);
const questions = computed<Record<string, unknown>[]>(() => {
  const q = params.value.questions;
  return Array.isArray(q) ? (q as Record<string, unknown>[]) : [];
});

const selectedOptions = reactive<Record<string, string>>({});
const otherInputs = reactive<Record<string, string>>({});

// 分步提问：一次只展示一道题
const qIndex = ref(0);
const currentQuestion = computed(() => questions.value[qIndex.value] ?? null);
const hasMultipleQuestions = computed(() => questions.value.length > 1);
const isLastQuestion = computed(
  () => qIndex.value >= questions.value.length - 1,
);

function nextQuestion() {
  if (qIndex.value < questions.value.length - 1) qIndex.value++;
}

function prevQuestion() {
  if (qIndex.value > 0) qIndex.value--;
}

function submitUserInput() {
  // 协议要求按问题 id 返回：{ answers: { [questionId]: { answers: [选择] } } }
  const answers: Record<string, { answers: string[] }> = {};
  for (const q of questions.value) {
    const id = str(q.id);
    const selected = selectedOptions[id];
    const value =
      q.isOther && selected === "__other__"
        ? (otherInputs[id] ?? "")
        : (selected ?? "");
    answers[id] = { answers: [value] };
  }
  emit("submit", answers);
}
</script>

<template>
  <div v-if="hasMultipleQuestions" class="question-progress">
    <span>第 {{ qIndex + 1 }} / {{ questions.length }} 题</span>
    <div class="question-progress-bar">
      <div
        class="question-progress-fill"
        :style="{ width: `${((qIndex + 1) / questions.length) * 100}%` }"
      ></div>
    </div>
  </div>
  <div v-if="currentQuestion" class="question-row">
    <div class="question-text">
      {{ currentQuestion.header ? `${str(currentQuestion.header)}：` : ""
      }}{{ str(currentQuestion.question) }}
    </div>
    <div
      v-if="
        Array.isArray(currentQuestion.options) &&
        (currentQuestion.options as unknown[]).length
      "
      class="question-options"
    >
      <button
        v-for="opt in currentQuestion.options as {
          label: string;
          description?: string;
        }[]"
        :key="opt.label"
        class="option-btn"
        :class="{
          selected: selectedOptions[str(currentQuestion.id)] === opt.label,
        }"
        @click="selectedOptions[str(currentQuestion.id)] = opt.label"
      >
        {{ opt.label }}
      </button>
      <button
        v-if="currentQuestion.isOther"
        class="option-btn"
        :class="{
          selected: selectedOptions[str(currentQuestion.id)] === '__other__',
        }"
        @click="selectedOptions[str(currentQuestion.id)] = '__other__'"
      >
        其他…
      </button>
    </div>
    <input
      v-if="
        selectedOptions[str(currentQuestion.id)] === '__other__' ||
        !Array.isArray(currentQuestion.options) ||
        !(currentQuestion.options as unknown[]).length
      "
      v-model="otherInputs[str(currentQuestion.id)]"
      :type="currentQuestion.isSecret ? 'password' : 'text'"
      placeholder="输入内容"
      style="width: 100%; margin-top: 6px"
    />
  </div>

  <div class="interaction-foot">
    <button class="btn" @click="emit('cancel')">取消</button>
    <template v-if="hasMultipleQuestions">
      <button class="btn" :disabled="qIndex === 0" @click="prevQuestion()">
        上一题
      </button>
      <button
        v-if="!isLastQuestion"
        class="btn primary"
        @click="nextQuestion()"
      >
        下一题
      </button>
      <button v-else class="btn primary" @click="submitUserInput()">
        提交
      </button>
    </template>
    <button v-else class="btn primary" @click="submitUserInput()">提交</button>
  </div>
</template>
