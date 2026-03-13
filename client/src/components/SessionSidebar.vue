<template>
  <aside class="session-sidebar">
    <div class="sidebar-header">
      <span class="sidebar-title">Sessions</span>
      <div class="total-cost" v-if="store.totalCostUsd > 0" title="Total cost across all Claude sessions">
        💰 ${{ store.totalCostUsd.toFixed(3) }}
      </div>
    </div>

    <div class="connection-status" :class="store.connectionStatus" :data-testid="'connection-status-' + store.connectionStatus">
      <span class="status-dot" />
      {{ statusLabel }}
    </div>

    <div class="sessions-tree">
      <div
        v-for="session in store.sessions"
        :key="session.id"
        class="session-node"
      >
        <div class="session-name">
          <span class="node-icon">▪</span>
          {{ session.name }}
          <span class="session-id">{{ session.id }}</span>
        </div>

        <div
          v-for="window in session.windows"
          :key="window.id"
          class="window-node"
        >
          <div class="window-name">
            <span class="node-icon">▸</span>
            {{ window.index }}: {{ window.name }}
          </div>

          <div
            v-for="pane in window.panes"
            :key="pane.id"
            class="pane-node"
            :class="{ active: pane.isActive, claude: pane.isClaude, selected: store.activePane === pane.id }"
            @click="selectPane(pane.id)"
          >
            <span class="pane-icon">{{ pane.isClaude ? '🤖' : '▸' }}</span>
            <span class="pane-cmd">{{ pane.currentCommand }}</span>
            <span class="pane-size">{{ pane.cols }}×{{ pane.rows }}</span>
            <span v-if="pane.isClaude" class="pane-cost">
              {{ claudeCost(pane.id) }}
            </span>
          </div>
        </div>
      </div>

      <div v-if="store.sessions.length === 0" class="empty-state">
        No active tmux sessions
      </div>
    </div>
  </aside>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useTmuxStore } from '@/stores/tmux'

const store = useTmuxStore()

const statusLabel = computed(() => ({
  connected: 'Connected',
  connecting: 'Connecting…',
  reconnecting: 'Reconnecting…',
  disconnected: 'Disconnected',
}[store.connectionStatus]))

function selectPane(paneId: string) {
  store.activePane = paneId
}

function claudeCost(paneId: string): string {
  const s = store.claudeSessions.get(paneId)
  if (!s || s.totalCostUsd === 0) return ''
  return `$${s.totalCostUsd.toFixed(3)}`
}
</script>

<style scoped>
.session-sidebar {
  width: 240px;
  min-width: 200px;
  background: #181825;
  border-right: 1px solid #313244;
  display: flex;
  flex-direction: column;
  font-size: 12px;
  user-select: none;
}
.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px;
  border-bottom: 1px solid #313244;
}
.sidebar-title { font-weight: 700; color: #89b4fa; font-size: 13px; }
.total-cost { color: #f9e2af; font-size: 11px; }
.connection-status {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  font-size: 11px;
  color: #585b70;
  border-bottom: 1px solid #313244;
}
.status-dot {
  width: 6px; height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.connection-status.connected { color: #a6e3a1; }
.connection-status.connecting,
.connection-status.reconnecting { color: #f9e2af; }
.connection-status.disconnected { color: #f38ba8; }
.sessions-tree { flex: 1; overflow-y: auto; padding: 8px 0; }
.session-node { margin-bottom: 4px; }
.session-name {
  display: flex; align-items: center; gap: 6px;
  padding: 4px 12px; font-weight: 600; color: #cdd6f4;
}
.session-id { color: #45475a; font-size: 10px; margin-left: auto; }
.window-node { margin-left: 8px; }
.window-name {
  display: flex; align-items: center; gap: 6px;
  padding: 3px 12px; color: #a6adc8;
}
.pane-node {
  display: flex; align-items: center; gap: 6px;
  padding: 3px 20px; cursor: pointer; border-radius: 4px;
  margin: 1px 4px; color: #585b70;
}
.pane-node:hover { background: #232634; color: #cdd6f4; }
.pane-node.active { color: #a6adc8; }
.pane-node.selected { background: #313244; color: #cdd6f4; }
.pane-node.claude { color: #89b4fa; }
.pane-cmd { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pane-size { color: #45475a; font-size: 10px; }
.pane-cost { color: #f9e2af; font-size: 10px; }
.empty-state { padding: 20px 12px; color: #45475a; text-align: center; }
</style>
