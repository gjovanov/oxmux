<template>
  <aside class="session-sidebar">
    <div class="sidebar-header">
      <span class="sidebar-title">Sessions</span>
      <button class="add-btn" @click="store.showNewSessionDialog = true" title="New Session">+</button>
    </div>

    <div class="connection-status" :class="store.connectionStatus" :data-testid="'connection-status-' + store.connectionStatus">
      <span class="status-dot" />
      {{ statusLabel }}
      <span class="transport-mode">{{ transportLabel }}</span>
    </div>

    <div class="total-cost" v-if="store.totalCostUsd > 0" title="Total cost across all Claude sessions">
      ${{ store.totalCostUsd.toFixed(3) }}
    </div>

    <div class="sessions-tree">
      <!-- Managed sessions -->
      <div
        v-for="ms in store.managedSessions"
        :key="ms.id"
        class="managed-session"
        :class="{ active: store.activeSessionId === ms.id }"
      >
        <div class="ms-header">
          <span class="transport-badge" :class="ms.transport.backend?.type || 'local'">{{ ms.transport.backend?.type || 'local' }}</span>
          <span class="browser-badge" :class="ms.transport.browser || 'websocket'">{{ ms.transport.browser || 'ws' }}</span>
          <span class="ms-name">{{ ms.name }}</span>
          <span class="ms-status" :class="ms.status">{{ ms.status }}</span>
        </div>

        <div class="ms-actions">
          <button
            v-if="ms.status === 'created' || ms.status === 'disconnected' || ms.status === 'error'"
            class="action-btn connect"
            @click="store.connectSession(ms.id)"
            title="Connect"
          >Connect</button>
          <button
            v-if="ms.status === 'connected'"
            class="action-btn refresh"
            @click="store.refreshSession(ms.id)"
            title="Refresh"
          >Refresh</button>
          <button
            v-if="ms.status === 'connected' || ms.status === 'connecting'"
            class="action-btn disconnect"
            @click="store.disconnectSession(ms.id)"
            title="Disconnect"
          >Disconnect</button>
          <button
            class="action-btn delete"
            @click="store.deleteSession(ms.id)"
            title="Delete"
          >&times;</button>
        </div>

        <div v-if="ms.error" class="ms-error">{{ ms.error }}</div>

        <!-- Agent section (only for SSH sessions that are connected) -->
        <div v-if="ms.status === 'connected' && ms.transport.backend?.type === 'ssh'" class="agent-section">
          <template v-if="getAgentStatus(ms) === 'online'">
            <div class="agent-online">
              <span class="agent-dot online"></span> Agent online
              <span class="agent-version">v{{ getAgentVersion(ms) }}</span>
            </div>
            <div class="transport-switch">
              <span class="current-transport" :class="store.activeTransportMode">{{ transportLabel }}</span>
              <button
                v-if="store.activeTransportMode === 'ssh'"
                class="action-btn p2p"
                @click="store.upgradeTransport(ms.id, 'quic_p2p')"
              >QUIC P2P</button>
              <button
                v-if="store.activeTransportMode === 'ssh'"
                class="action-btn p2p"
                @click="connectWebRtcP2P(ms)"
              >WebRTC P2P</button>
              <button
                v-if="store.activeTransportMode !== 'ssh'"
                class="action-btn disconnect"
                @click="store.teardownP2P()"
              >Back to SSH</button>
            </div>
          </template>
          <template v-else-if="getAgentStatus(ms) === 'installing'">
            <div class="agent-installing">
              <span class="agent-dot installing"></span> Installing agent...
            </div>
          </template>
          <template v-else>
            <div class="agent-not-installed">
              <span class="agent-dot offline"></span> No agent
              <button class="action-btn install" @click="store.installAgent(ms.id)">Install</button>
            </div>
          </template>
        </div>

        <!-- tmux tree for connected session -->
        <template v-if="ms.status === 'connected'">
          <div
            v-for="session in (store.activeSessionId === ms.id ? store.sessions : ms.tmux_sessions || [])"
            :key="session.id"
            class="session-node"
          >
            <div class="session-name">
              <span class="node-icon">&#x25AA;</span>
              {{ session.name }}
              <span class="session-id">{{ session.id }}</span>
            </div>

            <div
              v-for="window in session.windows"
              :key="window.id"
              class="window-node"
            >
              <div class="window-name">
                <span class="node-icon">&#x25B8;</span>
                {{ window.index }}: {{ window.name }}
              </div>

              <div
                v-for="pane in window.panes"
                :key="pane.id"
                class="pane-node"
                :class="{ active: pane.isActive, claude: pane.isClaude, selected: store.activeSessionId === ms.id && store.activePane === pane.id }"
                @click="switchToSession(ms.id, pane.id)"
              >
                <span class="pane-icon">{{ pane.isClaude ? '&#x1F916;' : '&#x25B8;' }}</span>
                <span class="pane-cmd">{{ pane.currentCommand }}</span>
                <span class="pane-size">{{ pane.cols }}&times;{{ pane.rows }}</span>
                <span v-if="pane.isClaude" class="pane-cost">
                  {{ claudeCost(pane.id) }}
                </span>
              </div>
            </div>
          </div>
        </template>
      </div>

      <div v-if="store.managedSessions.length === 0" class="empty-state">
        No sessions yet.<br />
        Click <strong>+</strong> to create one.
      </div>
    </div>
  </aside>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useTmuxStore } from '@/stores/tmux'

const store = useTmuxStore()

const transportLabel = computed(() => {
  const mode = store.activeTransportMode
  const labels: Record<string, string> = {
    'ssh': 'WS → Server → SSH',
    'quic_p2p': 'QUIC P2P → Agent',
    'webrtc_p2p': 'WebRTC P2P → Agent',
  }
  return labels[mode] || mode
})

const statusLabel = computed(() => ({
  connected: 'Connected',
  connecting: 'Connecting...',
  reconnecting: 'Reconnecting...',
  disconnected: 'Disconnected',
}[store.connectionStatus]))

function switchToSession(sessionId: string, paneId: string) {
  if (store.activeSessionId !== sessionId) {
    // Switching to a different session — reconnect it (cleans up old session)
    store.connectSession(sessionId)
  }
  store.activePane = paneId
}

function getAgentStatus(ms: any): string {
  const host = ms.transport?.backend?.host
  if (!host) return 'not_installed'
  return store.agentStatuses.get(host)?.status || 'not_installed'
}

function getAgentVersion(ms: any): string {
  const host = ms.transport?.backend?.host
  if (!host) return ''
  return store.agentStatuses.get(host)?.version || ''
}

async function connectWebRtcP2P(ms: any) {
  // Request agent token + info via transport upgrade
  store.upgradeTransport(ms.id, 'webrtc_p2p')
  // The transport_upgrade_ready response will trigger connectWebRtcTransport
}

function claudeCost(paneId: string): string {
  const s = store.claudeSessions.get(paneId)
  if (!s || s.totalCostUsd === 0) return ''
  return `$${s.totalCostUsd.toFixed(3)}`
}
</script>

<style scoped>
.session-sidebar {
  width: 260px;
  min-width: 220px;
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
.add-btn {
  background: #313244; border: none; color: #89b4fa;
  width: 24px; height: 24px; border-radius: 4px;
  font-size: 16px; cursor: pointer; display: flex;
  align-items: center; justify-content: center; font-weight: 700;
}
.add-btn:hover { background: #45475a; color: #74c7ec; }
.total-cost { padding: 4px 12px; color: #f9e2af; font-size: 11px; border-bottom: 1px solid #313244; }
.connection-status {
  display: flex; align-items: center; gap: 6px;
  padding: 6px 12px; font-size: 11px; color: #585b70;
  border-bottom: 1px solid #313244;
}
.status-dot { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
.transport-mode { margin-left: auto; font-size: 9px; opacity: 0.7; text-transform: uppercase; }
.connection-status.connected { color: #a6e3a1; }
.connection-status.connecting, .connection-status.reconnecting { color: #f9e2af; }
.connection-status.disconnected { color: #f38ba8; }
.sessions-tree { flex: 1; overflow-y: auto; padding: 4px 0; }

/* Managed session cards */
.managed-session {
  margin: 4px 6px; padding: 8px 10px;
  background: #11111b; border: 1px solid #313244;
  border-radius: 6px;
}
.managed-session.active { border-color: #89b4fa; }
.ms-header {
  display: flex; align-items: center; gap: 6px;
}
.transport-badge {
  font-size: 9px; font-weight: 700; text-transform: uppercase;
  padding: 1px 5px; border-radius: 3px;
  background: #313244; color: #a6adc8;
}
.transport-badge.local { background: #1e4620; color: #a6e3a1; }
.transport-badge.ssh { background: #1e3a5f; color: #89b4fa; }
.transport-badge.quic { background: #4c1d95; color: #cba6f7; }
.transport-badge.webrtc { background: #5f3a1e; color: #fab387; }
.ms-name { flex: 1; font-weight: 600; color: #cdd6f4; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ms-status { font-size: 10px; }
.ms-status.connected { color: #a6e3a1; }
.ms-status.connecting { color: #f9e2af; }
.ms-status.created { color: #585b70; }
.ms-status.disconnected { color: #a6adc8; }
.ms-status.error { color: #f38ba8; }
.ms-status.reconnecting { color: #f9e2af; }
.ms-actions {
  display: flex; gap: 4px; margin-top: 6px;
}
.action-btn {
  font-size: 10px; padding: 2px 8px; border: none;
  border-radius: 3px; cursor: pointer; font-weight: 600;
}
.action-btn.connect { background: #a6e3a1; color: #1e1e2e; }
.action-btn.connect:hover { background: #94e2d5; }
.action-btn.refresh { background: #89b4fa; color: #1e1e2e; }
.action-btn.refresh:hover { background: #74c7ec; }
.action-btn.disconnect { background: #f9e2af; color: #1e1e2e; }
.action-btn.disconnect:hover { background: #f5c2e7; }
.action-btn.delete { background: #45475a; color: #f38ba8; font-size: 14px; padding: 0 6px; }
.action-btn.delete:hover { background: #f38ba8; color: #1e1e2e; }
.ms-error {
  margin-top: 4px; padding: 4px 6px; font-size: 10px;
  background: #2d1520; color: #f38ba8; border-radius: 3px;
  word-break: break-all;
}

/* tmux tree */
.session-node { margin-top: 6px; }
.session-name {
  display: flex; align-items: center; gap: 6px;
  padding: 3px 4px; font-weight: 600; color: #cdd6f4;
}
.session-id { color: #45475a; font-size: 10px; margin-left: auto; }
.window-node { margin-left: 6px; }
.window-name {
  display: flex; align-items: center; gap: 6px;
  padding: 2px 4px; color: #a6adc8;
}
.pane-node {
  display: flex; align-items: center; gap: 6px;
  padding: 2px 12px; cursor: pointer; border-radius: 4px;
  margin: 1px 2px; color: #585b70;
}
.pane-node:hover { background: #232634; color: #cdd6f4; }
.pane-node.active { color: #a6adc8; }
.pane-node.selected { background: #313244; color: #cdd6f4; }
.pane-node.claude { color: #89b4fa; }
.pane-cmd { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pane-size { color: #45475a; font-size: 10px; }
.pane-cost { color: #f9e2af; font-size: 10px; }
/* Agent section */
.agent-section { margin-top: 6px; padding: 6px 8px; background: #11111b; border-radius: 4px; }
.agent-online, .agent-not-installed, .agent-installing {
  display: flex; align-items: center; gap: 6px; font-size: 10px; color: #a6adc8;
}
.agent-dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; }
.agent-dot.online { background: #a6e3a1; }
.agent-dot.offline { background: #585b70; }
.agent-dot.installing { background: #f9e2af; animation: pulse 1s infinite; }
@keyframes pulse { 50% { opacity: 0.4; } }
.agent-version { color: #585b70; margin-left: auto; }
.transport-switch { display: flex; gap: 4px; margin-top: 4px; align-items: center; }
.current-transport { font-size: 9px; color: #585b70; text-transform: uppercase; padding: 1px 4px; border-radius: 3px; }
.current-transport.ssh { color: #a6adc8; }
.current-transport.quic_p2p { color: #cba6f7; background: #2d1b4e; }
.current-transport.webrtc_p2p { color: #fab387; background: #3d2b1b; }
.action-btn.p2p { background: #cba6f7; color: #1e1e2e; }
.action-btn.p2p:hover { background: #b4befe; }
.action-btn.install { background: #89b4fa; color: #1e1e2e; margin-left: auto; }

.empty-state { padding: 20px 12px; color: #45475a; text-align: center; line-height: 1.6; }
</style>
