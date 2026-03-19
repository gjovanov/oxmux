<template>
  <div
    class="mashed-cell"
    :class="{ focused: isFocused }"
    @click="$emit('focus')"
  >
    <!-- Compact header -->
    <div class="cell-header">
      <span class="cell-session">{{ sessionName }}</span>
      <span v-if="host" class="cell-host">{{ host }}</span>
      <span class="cell-cmd">{{ paneCommand }}</span>
      <span class="cell-transport" :class="transportMode">{{ transportLabel }}</span>
      <button class="cell-btn" @click.stop="$emit('expand')" title="Expand">&#x2922;</button>
      <button class="cell-btn remove" @click.stop="$emit('remove')" title="Remove">&times;</button>
    </div>

    <!-- Terminal -->
    <div class="cell-terminal">
      <TerminalPane
        :pane-id="qualifiedPaneId"
        :is-active="isFocused"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import TerminalPane from '@/components/TerminalPane.vue'

const props = defineProps<{
  qualifiedPaneId: string
  sessionName: string
  paneCommand: string
  transportMode: string
  host: string
  isFocused: boolean
}>()

defineEmits<{
  focus: []
  remove: []
  expand: []
}>()

const transportLabel = computed(() => {
  const labels: Record<string, string> = {
    'ssh': 'SSH',
    'quic_p2p': 'QUIC',
    'webrtc_p2p': 'WebRTC',
  }
  return labels[props.transportMode] || props.transportMode
})
</script>

<style scoped>
.mashed-cell {
  display: flex;
  flex-direction: column;
  border: 1px solid #313244;
  border-radius: 4px;
  overflow: hidden;
  min-height: 0;
  transition: border-color 0.15s;
}
.mashed-cell.focused { border-color: #89b4fa; }

.cell-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 6px;
  background: #11111b;
  border-bottom: 1px solid #313244;
  font-size: 10px;
  flex-shrink: 0;
  min-height: 22px;
}

.cell-session { color: #89b4fa; font-weight: 700; }
.cell-host { color: #585b70; }
.cell-cmd { color: #a6adc8; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.cell-transport {
  font-size: 9px; font-weight: 600; padding: 0 4px;
  border-radius: 3px; text-transform: uppercase;
}
.cell-transport.ssh { color: #a6adc8; background: #1e3a5f; }
.cell-transport.quic_p2p { color: #cba6f7; background: #2d1b4e; }
.cell-transport.webrtc_p2p { color: #fab387; background: #3d2b1b; }

.cell-btn {
  background: none; border: none; color: #585b70;
  cursor: pointer; font-size: 12px; padding: 0 2px;
  line-height: 1;
}
.cell-btn:hover { color: #cdd6f4; }
.cell-btn.remove:hover { color: #f38ba8; }

.cell-terminal {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
</style>
