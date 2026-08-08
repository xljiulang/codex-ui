<script setup lang="ts">
import { ref } from "vue";
import { clearGoal, setGoal, store } from "../composables/useCodex";

const text = ref(store.goalText ?? "");

function save() {
  if (store.turnActive) return;
  void setGoal(text.value.trim());
}

function clear() {
  if (store.turnActive) return;
  void clearGoal();
  store.goalOpen = false;
  text.value = "";
}

function cancel() {
  // 未设置目标时取消，则回到执行模式
  if (store.taskMode === "goal" && !store.goalText) {
    store.taskMode = "execute";
  }
  store.goalOpen = false;
  text.value = store.goalText ?? "";
}
</script>

<template>
  <div class="modal-mask">
    <div class="modal">
      <div class="modal-head">
        <span class="modal-title">设置目标</span>
        <button class="modal-close" @click="cancel()">×</button>
      </div>
      <div class="modal-body">
        <textarea
          v-model="text"
          rows="4"
          style="width: 100%"
          placeholder="描述要持续追求的目标…"
        ></textarea>
      </div>
      <div class="modal-foot">
        <button class="btn" @click="clear()">清除目标</button>
        <button class="btn" @click="cancel()">取消</button>
        <button class="btn primary" :disabled="!text.trim()" @click="save()">保存</button>
      </div>
    </div>
  </div>
</template>
