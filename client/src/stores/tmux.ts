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

// ── Session management types ──────────────────────────────────────────

export type BrowserTransportType = 'websocket' | 'quic' | 'webrtc'

export type SshAuth =
  | { method: 'agent' }
  | { method: 'password'; password: string }
  | { method: 'private_key'; path: string; passphrase?: string }

export interface SshBackend {
  type: 'ssh'
  host: string
  port: number
  user: string
  auth: SshAuth
}

export interface AgentBackend {
  type: 'agent'
  host: string
  port: number
  agent_id?: string
}

export interface LocalBackend {
  type: 'local'
}

export type BackendTransport = SshBackend | AgentBackend | LocalBackend

export interface TransportConfig {
  browser: BrowserTransportType
  backend: BackendTransport
}

export type SessionStatus = 'created' | 'connecting' | 'connected' | 'reconnecting' | 'disconnected' | 'error'

export interface ManagedSession {
  id: string
  name: string
  transport: TransportConfig
  status: SessionStatus
  error?: string
  tmux_sessions: TmuxSessionInfo[]
}

type PaneOutputHandler = (data: Uint8Array) => void

export const useTmuxStore = defineStore('tmux', () => {
  // State
  const sessions = ref<TmuxSessionInfo[]>([])
  const managedSessions = ref<ManagedSession[]>([])
  const activePane = ref<string | null>(null)
  const activeSessionId = ref<string | null>(null)
  const claudeSessions = ref<Map<string, ClaudeSessionState>>(new Map())
  const connectionStatus = ref<'disconnected' | 'connecting' | 'connected' | 'reconnecting'>('disconnected')
  const ws = ref<WebSocket | null>(null)
  const showNewSessionDialog = ref(false)
  const agentStatuses = ref<Map<string, { status: string; agentId?: string; version?: string; quicPort?: number }>>(new Map())
  const activeTransportMode = ref<'ssh' | 'quic_p2p' | 'webrtc_p2p'>('ssh')

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

  // Abstract transport — send function set by connect()
  let transportSend: ((msg: Record<string, unknown>) => void) | null = null
  let transportClose: (() => void) | null = null

  // Actions
  function connect(wsUrl: string) {
    if (ws.value?.readyState === WebSocket.OPEN) return

    connectionStatus.value = 'connecting'
    const socket = new WebSocket(wsUrl)
    socket.binaryType = 'arraybuffer'

    socket.onopen = () => {
      connectionStatus.value = 'connected'
      backoffMs = 1000
      transportSend = (msg) => {
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(encode(msg))
        }
      }
      transportClose = () => socket.close()
      sendMsg({ t: 'sess_list' })
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
      transportSend = null
      scheduleReconnect(wsUrl)
    }

    socket.onerror = () => {
      connectionStatus.value = 'reconnecting'
    }

    ws.value = socket
  }

  /** Connect via WebTransport (QUIC). */
  async function connectQuic(url: string, token: string) {
    const { connectQuic: doConnect } = await import('@/composables/useQuic')

    connectionStatus.value = 'connecting'
    try {
      const conn = await doConnect(url, token)

      connectionStatus.value = 'connected'
      backoffMs = 1000
      transportSend = (msg) => conn.send(msg)
      transportClose = () => conn.close()

      conn.onMessage((msg) => handleServerMsg(msg))
      conn.onClose(() => {
        connectionStatus.value = 'disconnected'
        transportSend = null
      })

      sendMsg({ t: 'sess_list' })
    } catch (e) {
      console.error('[oxmux] QUIC connection failed:', e)
      connectionStatus.value = 'disconnected'
    }
  }

  /**
   * Connect via WebRTC DataChannel to agent.
   * Uses existing QUIC P2P connection for signaling (SDP/ICE exchange).
   */
  async function connectWebRtcTransport(agentHost: string, agentPort: number, token: string) {
    const { connectWebRtc } = await import('@/composables/useWebRtc')
    const { connectQuic: doQuic } = await import('@/composables/useQuic')

    connectionStatus.value = 'connecting'
    try {
      // First establish a QUIC connection for signaling
      const quicUrl = `https://${agentHost}:${agentPort}`
      console.log('[oxmux] opening QUIC signaling channel for WebRTC')
      const quicConn = await doQuic(quicUrl, token)

      // Send session name so agent can set up control mode
      const activeSess = managedSessions.value.find(s => s.id === activeSessionId.value)
      quicConn.send({ t: 'sess_connect', name: activeSess?.name || 'default' })

      // Keep QUIC alive during WebRTC negotiation
      const keepAlive = setInterval(() => quicConn.send({ t: 'ping', ts: Date.now() }), 5000)

      // Set up signaling handlers via QUIC
      const signalResolvers = new Map<string, (payload: Record<string, unknown>) => void>()

      quicConn.onMessage((msg) => {
        if (msg.t === 'webrtc_answer' || msg.t === 'webrtc_ice' || msg.t === 'webrtc_error') {
          console.log('[oxmux] WebRTC signaling msg:', msg.t, msg)
          const handler = signalResolvers.get('signal')
          if (handler) {
            if (msg.t === 'webrtc_answer') {
              handler({ type: 'answer', sdp: msg.sdp })
            } else if (msg.t === 'webrtc_ice') {
              handler({ type: 'ice_candidate', candidate: msg.candidate, sdp_mid: '0' })
            }
          }
        } else {
          handleServerMsg(msg)
        }
      })

      // Request ICE config from server (snake_case → camelCase)
      const iceRes = await fetch(`/api/ice-config?user=webrtc`)
      const raw = await iceRes.json()
      const iceConfig = {
        iceServers: (raw.ice_servers || []).map((s: any) => ({
          urls: s.urls,
          username: s.username,
          credential: s.credential,
        })),
      }

      // Wait for agent to set up control mode before starting WebRTC
      await new Promise(r => setTimeout(r, 1000))

      const conn = await connectWebRtc(
        iceConfig,
        (payload) => {
          // Send signaling via QUIC to agent
          if (payload.type === 'offer') {
            quicConn.send({ t: 'webrtc_offer', peer_id: 'browser', sdp: payload.sdp })
          } else if (payload.type === 'ice_candidate') {
            quicConn.send({ t: 'webrtc_ice', peer_id: 'browser', candidate: payload.candidate })
          }
        },
        (handler) => {
          signalResolvers.set('signal', handler)
        },
      )

      connectionStatus.value = 'connected'
      backoffMs = 1000
      transportSend = (msg) => conn.send(msg)
      transportClose = () => { clearInterval(keepAlive); conn.close(); quicConn.close() }
      activeTransportMode.value = 'webrtc_p2p'

      conn.onMessage((msg) => handleServerMsg(msg))
      conn.onClose(() => {
        console.warn('[oxmux] WebRTC P2P connection lost')
        activeTransportMode.value = 'ssh'
      })

      console.log('[oxmux] WebRTC P2P connected!')

      // Re-subscribe panes
      for (const paneId of paneHandlers.keys()) {
        sendMsg({ t: 'sub', pane: paneId })
      }
    } catch (e) {
      console.error('[oxmux] WebRTC P2P failed:', e)
      activeTransportMode.value = 'ssh'
    }
  }

  function scheduleReconnect(wsUrl: string) {
    if (reconnectTimeout) clearTimeout(reconnectTimeout)
    reconnectTimeout = setTimeout(() => {
      backoffMs = Math.min(backoffMs * 2, 30_000)
      connect(wsUrl)
    }, backoffMs)
  }

  function sendMsg(msg: Record<string, unknown>) {
    if (!transportSend) {
      console.warn('[oxmux] sendMsg: not connected, dropping:', msg.t)
      return
    }
    transportSend(msg)
  }

  function handleServerMsg(msg: Record<string, unknown>) {
    if (msg.t !== 'o' && msg.t !== 'pong') {
      console.log('[oxmux] received:', msg.t, msg)
    }
    switch (msg.t) {
      case 'o': {
        const handler = paneHandlers.get(msg.pane as string)
        handler?.(msg.data as Uint8Array)
        break
      }
      case 's': {
        sessions.value = (msg.sessions as TmuxSessionInfo[])
        break
      }
      case 'e': {
        applyTmuxEvent(msg.event as Record<string, unknown>)
        break
      }
      case 'c': {
        const handler = claudeHandlers.get(msg.session_id as string)
        handler?.(msg.event)
        break
      }
      case 'ca': {
        claudeSessions.value.set(
          msg.session_id as string,
          msg.state as ClaudeSessionState
        )
        break
      }
      case 'ice': {
        iceConfigResolvers.get(msg.peer_id as string)?.(msg.config as IceConfig)
        break
      }
      case 'pong': {
        lastPong.value = msg.ts as number
        break
      }

      // ── Session management responses ──────────────────────────────
      case 'sess_list': {
        managedSessions.value = (msg.sessions as ManagedSession[]) ?? []
        break
      }
      case 'sess_created': {
        const session = msg.session as ManagedSession
        managedSessions.value.push(session)
        break
      }
      case 'sess_updated': {
        const session = msg.session as ManagedSession
        const idx = managedSessions.value.findIndex(s => s.id === session.id)
        if (idx >= 0) managedSessions.value[idx] = session
        break
      }
      case 'sess_deleted': {
        const id = msg.session_id as string
        managedSessions.value = managedSessions.value.filter(s => s.id !== id)
        if (activeSessionId.value === id) {
          activeSessionId.value = null
          sessions.value = []
        }
        break
      }
      case 'sess_connected': {
        const session = msg.session as ManagedSession
        // Only update managed sessions if this is a known session (not from P2P agent)
        const idx = managedSessions.value.findIndex(s => s.id === session.id)
        if (idx >= 0) {
          managedSessions.value[idx] = session
          if (session.tmux_sessions?.length) {
            sessions.value = session.tmux_sessions
          }
          activeSessionId.value = session.id
          // Auto-check agent status for SSH sessions
          const host = (session.transport?.backend as any)?.host
          if (host && session.transport?.backend?.type === 'ssh') {
            checkAgentStatus(host)
          }
        }
        // If from P2P agent (unknown ID), don't overwrite state
        break
      }
      case 'sess_disconnected': {
        const session = msg.session as ManagedSession
        const idx = managedSessions.value.findIndex(s => s.id === session.id)
        if (idx >= 0) managedSessions.value[idx] = session
        if (activeSessionId.value === session.id) {
          sessions.value = []
          activePane.value = null
        }
        break
      }
      case 'err': {
        console.error(`Server error [${msg.code}]: ${msg.message}`)
        break
      }

      // ── Agent management ──────────────────────────────────────────
      case 'agent_status': {
        const host = msg.host as string
        agentStatuses.value.set(host, {
          status: msg.status as string,
          agentId: msg.agent_id as string | undefined,
          version: msg.version as string | undefined,
          quicPort: msg.quic_port as number | undefined,
        })
        break
      }
      case 'transport_upgrade_ready': {
        const sid = msg.session_id as string
        const token = msg.agent_token as string
        const host = msg.agent_host as string
        const port = msg.agent_port as number
        const target = msg.target as string | undefined
        console.log(`[oxmux] transport upgrade ready: ${host}:${port} (${target || 'quic_p2p'})`)

        if (target === 'webrtc_p2p') {
          connectWebRtcTransport(host, port, token)
        } else {
          connectQuicP2P(host, port, token)
        }
        break
      }
      case 'transport_upgrade_failed': {
        console.error(`[oxmux] transport upgrade failed: ${msg.error}`)
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
    }
  }

  // ── Session CRUD ────────────────────────────────────────────────────

  function createSession(name: string, transport: TransportConfig) {
    sendMsg({ t: 'sess_create', name, transport })
  }

  function connectSession(sessionId: string) {
    sendMsg({ t: 'sess_connect', session_id: sessionId })
  }

  function disconnectSession(sessionId: string) {
    sendMsg({ t: 'sess_disconnect', session_id: sessionId })
  }

  function deleteSession(sessionId: string) {
    sendMsg({ t: 'sess_delete', session_id: sessionId })
  }

  function refreshSession(sessionId: string) {
    sendMsg({ t: 'sess_refresh', session_id: sessionId })
  }

  function listSessions() {
    sendMsg({ t: 'sess_list' })
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

  // ── Agent management ────────────────────────────────────────────────

  function checkAgentStatus(host: string) {
    sendMsg({ t: 'agent_status', host })
  }

  function installAgent(sessionId: string) {
    sendMsg({ t: 'agent_install', session_id: sessionId })
  }

  function upgradeTransport(sessionId: string, target: 'quic_p2p' | 'webrtc_p2p') {
    sendMsg({ t: 'transport_upgrade', session_id: sessionId, target })
  }

  /** Connect directly to agent via QUIC (P2P). */
  async function connectQuicP2P(host: string, port: number, token: string) {
    const { connectQuic: doConnect } = await import('@/composables/useQuic')

    try {
      const url = `https://${host}:${port}`
      console.log(`[oxmux] connecting QUIC P2P to ${url}`)
      const conn = await doConnect(url, token)

      // Switch transport to P2P for pane I/O
      transportSend = (msg) => conn.send(msg)
      transportClose = () => conn.close()
      activeTransportMode.value = 'quic_p2p'

      conn.onMessage((msg) => handleServerMsg(msg))
      conn.onClose(() => {
        console.warn('[oxmux] P2P connection lost, falling back to SSH')
        activeTransportMode.value = 'ssh'
      })

      console.log('[oxmux] QUIC P2P connected!')

      // Tell agent which tmux session to attach to
      const activeSess = managedSessions.value.find(s => s.id === activeSessionId.value)
      if (activeSess) {
        sendMsg({ t: 'sess_connect', name: activeSess.name })
      }

      // Re-subscribe all active panes on the new transport
      for (const paneId of paneHandlers.keys()) {
        console.log('[oxmux] re-subscribing pane on P2P:', paneId)
        sendMsg({ t: 'sub', pane: paneId })
      }
    } catch (e) {
      console.error('[oxmux] QUIC P2P failed:', e)
      activeTransportMode.value = 'ssh'
    }
  }

  return {
    // State
    sessions,
    managedSessions,
    activePane,
    activeSessionId,
    claudeSessions,
    connectionStatus,
    allPanes,
    claudePanes,
    totalCostUsd,
    lastPong,
    showNewSessionDialog,
    agentStatuses,
    activeTransportMode,
    // Session CRUD
    createSession,
    connectSession,
    disconnectSession,
    deleteSession,
    refreshSession,
    listSessions,
    // Pane actions
    connect,
    connectQuic,
    connectWebRtc: connectWebRtcTransport,
    subscribePane,
    unsubscribePane,
    sendInput,
    sendResize,
    requestIceConfig,
    subscribeClaudeSession,
    ping,
    // Agent management
    checkAgentStatus,
    installAgent,
    upgradeTransport,
  }
})
