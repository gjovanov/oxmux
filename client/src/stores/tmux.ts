import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { decode, encode } from '@msgpack/msgpack'

export interface TmuxPaneInfo {
  id: string
  index: number
  cols: number
  rows: number
  currentCommand: string
  isActive: boolean
  isClaude: boolean
}

export interface TmuxWindowInfo {
  id: string
  name: string
  index: number
  layout: string
  panes: TmuxPaneInfo[]
}

export interface TmuxSessionInfo {
  id: string
  name: string
  windows: TmuxWindowInfo[]
}

export interface ClaudeSessionState {
  sessionId: string
  totalCostUsd: number
  turnCount: number
  fileChanges: FileChange[]
  lastUsage: TokenUsage | null
  isComplete: boolean
  isError: boolean
}

export interface FileChange {
  tool: string
  path: string
  kind: 'create' | 'edit' | 'delete'
}

export interface TokenUsage {
  inputTokens: number
  outputTokens: number
  cacheReadInputTokens: number
}

export interface IceConfig {
  iceServers: Array<{
    urls: string[]
    username?: string
    credential?: string
  }>
}

type PaneOutputHandler = (data: Uint8Array) => void

export const useTmuxStore = defineStore('tmux', () => {
  // State
  const sessions = ref<TmuxSessionInfo[]>([])
  const activePane = ref<string | null>(null)
  const claudeSessions = ref<Map<string, ClaudeSessionState>>(new Map())
  const connectionStatus = ref<'disconnected' | 'connecting' | 'connected' | 'reconnecting'>('disconnected')
  const ws = ref<WebSocket | null>(null)

  // pane_id → output handler (registered by TerminalPane components)
  const paneHandlers = new Map<string, PaneOutputHandler>()
  // claude session_id → event handler
  const claudeHandlers = new Map<string, (event: unknown) => void>()

  let reconnectTimeout: ReturnType<typeof setTimeout> | null = null
  let backoffMs = 1000

  // Computed
  const allPanes = computed(() =>
    sessions.value.flatMap(s => s.windows.flatMap(w => w.panes))
  )

  const claudePanes = computed(() =>
    allPanes.value.filter(p => p.isClaude)
  )

  const totalCostUsd = computed(() =>
    Array.from(claudeSessions.value.values())
      .reduce((sum, s) => sum + s.totalCostUsd, 0)
  )

  // Actions
  function connect(wsUrl: string) {
    if (ws.value?.readyState === WebSocket.OPEN) return

    connectionStatus.value = 'connecting'
    const socket = new WebSocket(wsUrl)
    socket.binaryType = 'arraybuffer'

    socket.onopen = () => {
      connectionStatus.value = 'connected'
      backoffMs = 1000
      // Request full state on connect
      sendMsg({ t: 'cmd', command: 'list-sessions' })
    }

    socket.onmessage = (ev: MessageEvent<ArrayBuffer>) => {
      try {
        const msg = decode(new Uint8Array(ev.data)) as Record<string, unknown>
        handleServerMsg(msg)
      } catch (e) {
        console.error('Failed to decode server message', e)
      }
    }

    socket.onclose = () => {
      connectionStatus.value = 'reconnecting'
      scheduleReconnect(wsUrl)
    }

    socket.onerror = () => {
      connectionStatus.value = 'reconnecting'
    }

    ws.value = socket
  }

  function scheduleReconnect(wsUrl: string) {
    if (reconnectTimeout) clearTimeout(reconnectTimeout)
    reconnectTimeout = setTimeout(() => {
      backoffMs = Math.min(backoffMs * 2, 30_000)
      connect(wsUrl)
    }, backoffMs)
  }

  function sendMsg(msg: Record<string, unknown>) {
    if (ws.value?.readyState !== WebSocket.OPEN) return
    ws.value.send(encode(msg))
  }

  function handleServerMsg(msg: Record<string, unknown>) {
    switch (msg.t) {
      case 'o': {
        // Raw PTY output
        const handler = paneHandlers.get(msg.pane as string)
        handler?.(msg.data as Uint8Array)
        break
      }
      case 's': {
        // Full state dump
        sessions.value = (msg.sessions as TmuxSessionInfo[])
        break
      }
      case 'e': {
        // Incremental tmux event
        applyTmuxEvent(msg.event as Record<string, unknown>)
        break
      }
      case 'c': {
        // Structured Claude event
        const handler = claudeHandlers.get(msg.session_id as string)
        handler?.(msg.event)
        break
      }
      case 'ca': {
        // Claude accumulator snapshot
        claudeSessions.value.set(
          msg.session_id as string,
          msg.state as ClaudeSessionState
        )
        break
      }
      case 'ice': {
        // ICE config — resolve pending promise
        iceConfigResolvers.get(msg.peer_id as string)?.(msg.config as IceConfig)
        break
      }
      case 'pong': {
        lastPong.value = msg.ts as number
        break
      }
    }
  }

  function applyTmuxEvent(event: Record<string, unknown>) {
    switch (event.k) {
      case 'session_created':
        sessions.value.push({ id: event.id as string, name: event.name as string, windows: [] })
        break
      case 'session_closed':
        sessions.value = sessions.value.filter(s => s.id !== event.id)
        break
      // Additional events handled incrementally
    }
  }

  // Pane subscription
  function subscribePane(paneId: string, handler: PaneOutputHandler) {
    paneHandlers.set(paneId, handler)
    sendMsg({ t: 'sub', pane: paneId })
  }

  function unsubscribePane(paneId: string) {
    paneHandlers.delete(paneId)
    sendMsg({ t: 'unsub', pane: paneId })
  }

  function sendInput(paneId: string, data: Uint8Array) {
    sendMsg({ t: 'i', pane: paneId, data })
  }

  function sendResize(paneId: string, cols: number, rows: number) {
    sendMsg({ t: 'r', pane: paneId, cols, rows })
  }

  // ICE config request (returns Promise)
  const iceConfigResolvers = new Map<string, (config: IceConfig) => void>()
  const lastPong = ref<number>(0)

  async function requestIceConfig(peerId: string): Promise<IceConfig> {
    return new Promise((resolve) => {
      iceConfigResolvers.set(peerId, resolve)
      sendMsg({ t: 'ice_req', peer_id: peerId })
    })
  }

  function subscribeClaudeSession(sessionId: string, handler: (event: unknown) => void) {
    claudeHandlers.set(sessionId, handler)
  }

  function ping() {
    sendMsg({ t: 'ping', ts: Date.now() })
  }

  return {
    // State
    sessions,
    activePane,
    claudeSessions,
    connectionStatus,
    allPanes,
    claudePanes,
    totalCostUsd,
    lastPong,
    // Actions
    connect,
    subscribePane,
    unsubscribePane,
    sendInput,
    sendResize,
    requestIceConfig,
    subscribeClaudeSession,
    ping,
  }
})
