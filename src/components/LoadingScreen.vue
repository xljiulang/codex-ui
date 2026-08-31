<template>
  <div class="loading-screen">
    <div class="loading-screen__logo">
      <svg viewBox="0 0 24 24">
        <path d="M12 2l8.66 5v10L12 22l-8.66-5V7z" />
        <path class="logo-c" d="M14.9 9.1a4.5 4.5 0 1 0 0 5.8" />
      </svg>
    </div>
    <span class="loading-screen__text" role="status">正在加载…</span>
  </div>
</template>

<style scoped>
.loading-screen {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  height: 100%;
  background: var(--bg);
}

.loading-screen__logo svg {
  width: 52px;
  height: 52px;
  fill: none;
  stroke: var(--accent);
  stroke-width: 2.2;
  animation: loading-spin 1.5s linear infinite;
}

.loading-screen__logo {
  animation: logo-breathe 2.4s ease-in-out infinite;
}

.loading-screen__logo svg .logo-c {
  stroke-width: 2;
  stroke-linecap: round;
}

.loading-screen__text {
  color: var(--text-dim);
}

@keyframes loading-spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes logo-breathe {
  0%,
  100% {
    filter: drop-shadow(0 0 10px rgba(var(--accent-rgb), 0.2));
  }
  50% {
    filter: drop-shadow(0 0 26px rgba(var(--accent-rgb), 0.45));
  }
}
</style>
