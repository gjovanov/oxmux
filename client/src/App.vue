<template>
  <div class="app">
    <SessionSidebar />
    <main class="pane-area">
      <template v-if="store.activePane">
        <ClaudePane
          v-if="activeIsClaudePane"
          :session-id="store.activePane"
          :cost-alert-threshold="1.0"
          @open-diff="openDiff"
        />
        <TerminalPane
          v-else
          :pane-id="store.activePane"
          :is-active="true"
        />
      </template>
      <div v-else class="empty-pane">
        <div class="empty-message">
          <span class="empty-icon">🐙</span>
          <h2>Oxmux</h2>
          <p>Select a tmux pane from the sidebar to begin.</p>
        </div>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useTmuxStore } from '@/stores/tmux'
import SessionSidebar from '@/components/SessionSidebar.vue'
import TerminalPane from '@/components/TerminalPane.vue'
import ClaudePane from '@/components/ClaudePane.vue'

const store = useTmuxStore()

const WS_URL = import.meta.env.VITE_WS_URL ?? `ws://${window.location.host}/ws`

onMounted(() => {
  store.connect(WS_URL)
})

const activeIsClaudePane = computed(() => {
  if (!store.activePane) return false
  return store.allPanes.find(p => p.id === store.activePane)?.isClaude ?? false
})

function openDiff(path: string) {
  console.log('Open diff:', path) // TODO: Monaco diff viewer
}
</script>

<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body, #app { height: 100%; }
body { background: #1e1e2e; color: #cdd6f4; font-family: system-ui, sans-serif; }
</style>

<style scoped>
.app {
  display: flex;
  height: 100vh;
  overflow: hidden;
}
.pane-area {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.empty-pane {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.empty-message {
  text-align: center;
  color: #585b70;
}
.empty-icon { font-size: 48px; display: block; margin-bottom: 12px; }
h2 { font-size: 24px; margin-bottom: 8px; color: #89b4fa; }
p { font-size: 14px; }
</style>
