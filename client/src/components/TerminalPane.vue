<template>
  <div class="terminal-pane" :class="{ active: isActive }" @click="onFocus">
    <div
      ref="containerRef"
      class="xterm-container"
      :data-testid="`terminal-pane-${paneId}`"
    />
    <!-- Accessibility output for Playwright assertions -->
    <div
      class="sr-only"
      :data-testid="`terminal-accessible-output`"
      aria-live="polite"
      aria-atomic="false"
    >{{ accessibleBuffer }}</div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, onMounted, toRef } from 'vue'
import { useTerminal } from '@/composables/useTerminal'

const props = defineProps<{
  paneId: string
  isActive?: boolean
}>()

const emit = defineEmits<{
  focus: [paneId: string]
}>()

const containerRef = ref<HTMLElement | null>(null)
const paneIdRef = ref(props.paneId)
const isActiveRef = ref(props.isActive ?? false)
const accessibleBuffer = ref('')

// Sync isActive prop to ref
watch(() => props.isActive, (v) => { isActiveRef.value = !!v })

const { term, focus } = useTerminal(paneIdRef, containerRef, isActiveRef)

// Keep accessible buffer updated (last 500 chars, for E2E tests)
onMounted(() => {
  watch(term, (t) => {
    if (!t) return
    t.onWriteParsed(() => {
      try {
        const rows = []
        for (let i = Math.max(0, t.buffer.active.length - 5); i < t.buffer.active.length; i++) {
          const line = t.buffer.active.getLine(i)
          if (line) rows.push(line.translateToString(true))
        }
        accessibleBuffer.value = rows.join('\n').slice(-500)
      } catch { /* ignore */ }
    })
  }, { immediate: true })
})

function onFocus() {
  focus()
  emit('focus', props.paneId)
}
</script>

<style scoped>
.terminal-pane {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 4px;
  overflow: hidden;
  transition: border-color 0.15s;
}

.terminal-pane.active {
  border-color: #89b4fa;
}

.xterm-container {
  flex: 1;
  min-height: 0;
  padding: 4px;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
}
</style>
