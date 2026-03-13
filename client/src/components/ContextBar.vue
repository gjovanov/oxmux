<template>
  <div class="context-bar" :title="`Context: ${pct}% used`">
    <span class="ctx-label">ctx</span>
    <div class="ctx-track">
      <div class="ctx-fill" :style="{ width: pct + '%' }" :class="fillClass" />
    </div>
    <span class="ctx-pct">{{ pct }}%</span>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { TokenUsage } from '@/stores/tmux'

const MAX_CONTEXT = 200_000
const props = defineProps<{ usage: TokenUsage }>()
const used = computed(() =>
  (props.usage.inputTokens ?? 0) + (props.usage.cacheReadInputTokens ?? 0)
)
const pct = computed(() => Math.min(100, Math.round((used.value / MAX_CONTEXT) * 100)))
const fillClass = computed(() => {
  if (pct.value >= 90) return 'critical'
  if (pct.value >= 75) return 'warn'
  return 'ok'
})
</script>

<style scoped>
.context-bar { display: flex; align-items: center; gap: 6px; font-size: 11px; }
.ctx-label { color: #585b70; }
.ctx-track { width: 60px; height: 4px; background: #313244; border-radius: 2px; overflow: hidden; }
.ctx-fill { height: 100%; border-radius: 2px; transition: width 0.3s; }
.ctx-fill.ok { background: #a6e3a1; }
.ctx-fill.warn { background: #f9e2af; }
.ctx-fill.critical { background: #f38ba8; }
.ctx-pct { color: #585b70; width: 30px; text-align: right; }
</style>
