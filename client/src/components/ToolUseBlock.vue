<template>
  <div class="tool-use-block" :class="{ expanded }">
    <div class="tool-header" @click="expanded = !expanded">
      <span class="tool-icon">{{ toolIcon }}</span>
      <span class="tool-name">{{ name }}</span>
      <span v-if="filePath" class="tool-path">{{ filePath }}</span>
      <span class="expand-toggle">{{ expanded ? '▾' : '▸' }}</span>
    </div>
    <div v-if="expanded" class="tool-body">
      <pre class="tool-input"><code>{{ formattedInput }}</code></pre>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'

const props = defineProps<{
  name: string
  input: Record<string, unknown>
  toolId: string
}>()

const expanded = ref(false)

const toolIcon = computed(() => {
  const icons: Record<string, string> = {
    Read: '📄', Write: '✏️', Edit: '🔧', MultiEdit: '🔧',
    Bash: '💻', Glob: '🔍', Grep: '🔎', LS: '📁',
    TodoRead: '📋', TodoWrite: '📋', WebSearch: '🌐',
    WebFetch: '🌐', Task: '🤖',
  }
  return icons[props.name] ?? '🔧'
})

const filePath = computed(() => {
  return (props.input.file_path ?? props.input.path ?? props.input.command ?? '') as string
})

const formattedInput = computed(() => JSON.stringify(props.input, null, 2))
</script>

<style scoped>
.tool-use-block {
  background: #181825;
  border: 1px solid #313244;
  border-radius: 6px;
  overflow: hidden;
  font-size: 12px;
}
.tool-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  cursor: pointer;
  user-select: none;
}
.tool-header:hover { background: #232634; }
.tool-name { font-weight: 600; color: #89b4fa; }
.tool-path { color: #a6adc8; font-family: monospace; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.expand-toggle { color: #585b70; margin-left: auto; }
.tool-body { border-top: 1px solid #313244; }
.tool-input { margin: 0; padding: 10px; overflow-x: auto; background: transparent; color: #cdd6f4; font-size: 11px; line-height: 1.5; }
</style>
