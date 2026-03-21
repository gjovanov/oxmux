<template>
  <LoginPage v-if="!auth.isAuthenticated" />
  <div v-else class="app" :class="{ 'mobile': isMobile, 'tablet': isTablet }">
    <!-- Mobile sidebar backdrop -->
    <div
      v-if="!isDesktop && sidebarOpen"
      class="sidebar-backdrop"
      @click="closeSidebar"
    />

    <!-- Sidebar -->
    <SessionSidebar
      :class="{ open: isDesktop || sidebarOpen }"
      :style="isDesktop ? { width: sidebarWidth + 'px' } : undefined"
    />

    <!-- Desktop resize handle -->
    <div v-if="isDesktop" class="sidebar-resizer" @mousedown="startResize" />

    <NewSessionDialog v-if="store.showNewSessionDialog" />

    <main class="pane-area">
      <!-- Top bar: hamburger + view toggle -->
      <div class="top-bar" v-if="store.connectedSessionIds.size > 0 || !isDesktop">
        <button v-if="!isDesktop" class="hamburger" @click="toggleSidebar">
          <span /><span /><span />
        </button>
        <template v-if="store.connectedSessionIds.size > 0">
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
        </template>
        <span v-if="!isDesktop" class="top-bar-spacer" />
        <span v-if="!isDesktop && store.connectedSessionIds.size > 0" class="session-badge">
          {{ store.connectedSessionIds.size }}
        </span>
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
import { useResponsive } from '@/composables/useResponsive'
import LoginPage from '@/components/LoginPage.vue'
import SessionSidebar from '@/components/SessionSidebar.vue'
import NewSessionDialog from '@/components/NewSessionDialog.vue'
import TerminalPane from '@/components/TerminalPane.vue'
import ClaudePane from '@/components/ClaudePane.vue'
import MashedView from '@/components/MashedView.vue'

const auth = useAuthStore()
const store = useTmuxStore()
const { isMobile, isTablet, isDesktop, sidebarOpen, toggleSidebar, closeSidebar } = useResponsive()

const viewMode = ref<'single' | 'mashed'>(
  (localStorage.getItem('oxmux_view_mode') as 'single' | 'mashed') || 'mashed'
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
  if (auth.isAuthenticated) connectWs()
})

watch(() => auth.isAuthenticated, (isAuth) => {
  if (isAuth) connectWs()
})

const activeIsClaudePane = computed(() => {
  if (!store.activePane) return false
  return store.allPanes.find(p => p.qualifiedId === store.activePane)?.isClaude ?? false
})

function openDiff(path: string) {
  console.log('Open diff:', path)
}

// Resizable sidebar (desktop only)
const sidebarWidth = ref(parseInt(localStorage.getItem('oxmux_sidebar_w') || '260'))

function startResize(e: MouseEvent) {
  e.preventDefault()
  const startX = e.clientX
  const startW = sidebarWidth.value
  const onMove = (ev: MouseEvent) => {
    sidebarWidth.value = Math.max(180, Math.min(500, startW + ev.clientX - startX))
  }
  const onUp = () => {
    document.removeEventListener('mousemove', onMove)
    document.removeEventListener('mouseup', onUp)
    document.body.style.cursor = ''
    document.body.style.userSelect = ''
    localStorage.setItem('oxmux_sidebar_w', String(sidebarWidth.value))
  }
  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'
  document.addEventListener('mousemove', onMove)
  document.addEventListener('mouseup', onUp)
}
</script>

<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body, #app { height: 100%; overscroll-behavior: none; }
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

/* ── Top bar (hamburger + view toggle) ────────────────────────── */
.top-bar {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}
.top-bar-spacer { flex: 1; }
.toggle-btn {
  background: #313244; border: none; color: #a6adc8;
  padding: 2px 12px; border-radius: 4px; font-size: 11px;
  cursor: pointer; font-weight: 600;
}
.toggle-btn:hover { background: #45475a; }
.toggle-btn.active { background: #89b4fa; color: #1e1e2e; }

/* ── Hamburger ────────────────────────────────────────────────── */
.hamburger {
  display: flex; flex-direction: column; gap: 3px;
  background: none; border: none; cursor: pointer;
  padding: 6px; border-radius: 4px;
}
.hamburger span {
  display: block; width: 18px; height: 2px;
  background: #a6adc8; border-radius: 1px;
}
.hamburger:hover span { background: #cdd6f4; }

.session-badge {
  background: #89b4fa; color: #1e1e2e;
  font-size: 10px; font-weight: 700;
  width: 18px; height: 18px; border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
}

/* ── Desktop sidebar resizer ──────────────────────────────────── */
.sidebar-resizer {
  width: 4px; cursor: col-resize;
  background: transparent; flex-shrink: 0;
  transition: background 0.15s;
}
.sidebar-resizer:hover { background: #89b4fa; }

/* ── Mobile sidebar (overlay) ─────────────────────────────────── */
.sidebar-backdrop {
  position: fixed; inset: 0;
  background: rgba(0, 0, 0, 0.5);
  z-index: 99;
}

.app.mobile :deep(.session-sidebar),
.app.tablet :deep(.session-sidebar) {
  position: fixed;
  left: 0; top: 0;
  width: 85vw !important;
  max-width: 320px;
  height: 100vh;
  z-index: 100;
  transform: translateX(-100%);
  transition: transform 0.25s ease;
}
.app.mobile :deep(.session-sidebar.open),
.app.tablet :deep(.session-sidebar.open) {
  transform: translateX(0);
}

/* ── Mobile touch targets ─────────────────────────────────────── */
.app.mobile .toggle-btn {
  padding: 8px 16px; font-size: 13px;
}
.app.mobile .top-bar {
  padding: 6px 8px;
}

/* ── Empty state ──────────────────────────────────────────────── */
.empty-pane {
  flex: 1; display: flex;
  align-items: center; justify-content: center;
}
.empty-message { text-align: center; color: #585b70; }
h2 { font-size: 24px; margin-bottom: 8px; color: #89b4fa; }
p { font-size: 14px; }
.user-bar { margin-bottom: 20px; font-size: 12px; color: #a6adc8; }
.logout-btn {
  margin-left: 8px; padding: 2px 10px;
  background: #313244; border: none; border-radius: 4px;
  color: #f38ba8; font-size: 11px; cursor: pointer;
}
.logout-btn:hover { background: #45475a; }
</style>
