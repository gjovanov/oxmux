<template>
  <div class="cost-meter" :class="{ alert: isAlert }" :data-testid="'cost-meter'">
    <span class="cost-icon">💰</span>
    <span class="cost-value">${{ cost.toFixed(4) }}</span>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
const props = defineProps<{ cost: number; alertThreshold?: number }>()
const isAlert = computed(() => props.alertThreshold && props.cost >= props.alertThreshold)
</script>

<style scoped>
.cost-meter { display: flex; align-items: center; gap: 4px; font-size: 12px; padding: 2px 8px; border-radius: 10px; background: #313244; }
.cost-meter.alert { background: rgba(249, 226, 175, 0.2); color: #f9e2af; }
.cost-value { font-variant-numeric: tabular-nums; }
</style>
