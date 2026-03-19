<template>
  <LoginPage v-if="!auth.isAuthenticated" />
  <div v-else class="app">
    <SessionSidebar />
    <NewSessionDialog v-if="store.showNewSessionDialog" />
    <main class="pane-area">
      <!-- View mode toggle -->
      <div class="view-toggle" v-if="store.connectedSessionIds.size > 0">
        <button
          class="toggle-btn"
          :class="{ active: viewMode === 'single' }"
          @click="viewMode = 'single'"
        >Single</button>
        <button
          class="toggle-btn"
          :class="{ active: viewMode === 'mashed' }"
          @click="viewMode = 'mashed'"
        >Mashed</button>
      </div>

      <!-- Mashed view -->
      <MashedView
        v-if="viewMode === 'mashed'"
        @switch-view="viewMode = $event"
      />

      <!-- Single pane view -->
      <template v-else-if="store.activePane">
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
          <div class="user-bar">
            Logged in as <strong>{{ auth.user?.username }}</strong>
            <button class="logout-btn" @click="auth.logout()">Logout</button>
          </div>
          <h2>Oxmux</h2>
          <p>Select a tmux pane from the sidebar to begin.</p>
        </div>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useAuthStore } from '@/stores/auth'
import { useTmuxStore } from '@/stores/tmux'
import { parseQualifiedPaneId } from '@/utils/paneId'
import LoginPage from '@/components/LoginPage.vue'
import SessionSidebar from '@/components/SessionSidebar.vue'
import NewSessionDialog from '@/components/NewSessionDialog.vue'
import TerminalPane from '@/components/TerminalPane.vue'
import ClaudePane from '@/components/ClaudePane.vue'
import MashedView from '@/components/MashedView.vue'

const auth = useAuthStore()
const store = useTmuxStore()

const viewMode = ref<'single' | 'mashed'>(
  (localStorage.getItem('oxmux_view_mode') as 'single' | 'mashed') || 'single'
)
watch(viewMode, (v) => localStorage.setItem('oxmux_view_mode', v))

const wsProto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'

function connectWs() {
  if (auth.token) {
    const wsUrl = `${wsProto}//${window.location.host}/ws?token=${auth.token}`
    store.connect(wsUrl)
  }
}

onMounted(async () => {
  await auth.checkAuth()
  if (auth.isAuthenticated) {
    connectWs()
  }
})

watch(() => auth.isAuthenticated, (isAuth) => {
  if (isAuth) {
    connectWs()
  }
})

const activeIsClaudePane = computed(() => {
  if (!store.activePane) return false
  const { paneId } = parseQualifiedPaneId(store.activePane)
  return store.allPanes.find(p => p.qualifiedId === store.activePane)?.isClaude ?? false
})

function openDiff(path: string) {
  console.log('Open diff:', path)
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
.view-toggle {
  display: flex;
  gap: 2px;
  padding: 4px 8px;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}
.toggle-btn {
  background: #313244; border: none; color: #a6adc8;
  padding: 2px 12px; border-radius: 4px; font-size: 11px;
  cursor: pointer; font-weight: 600;
}
.toggle-btn:hover { background: #45475a; }
.toggle-btn.active { background: #89b4fa; color: #1e1e2e; }

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
h2 { font-size: 24px; margin-bottom: 8px; color: #89b4fa; }
p { font-size: 14px; }
.user-bar {
  margin-bottom: 20px;
  font-size: 12px;
  color: #a6adc8;
}
.logout-btn {
  margin-left: 8px;
  padding: 2px 10px;
  background: #313244;
  border: none;
  border-radius: 4px;
  color: #f38ba8;
  font-size: 11px;
  cursor: pointer;
}
.logout-btn:hover { background: #45475a; }
</style>
