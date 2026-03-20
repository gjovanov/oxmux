import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { decode, encode } from '@msgpack/msgpack'
import { qualifyPaneId, parseQualifiedPaneId, type QualifiedPaneId } from '@/utils/paneId'

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
  | { method: 'uploaded_key'; key_id: string }

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

/** Per-session P2P connection */
interface P2PConnection {
  sessionId: string
  send: (msg: Record<string, unknown>) => void
  close: () => void
  mode: 'quic_p2p' | 'webrtc_p2p'
}

// Message types that go to the server (control plane)
const CONTROL_MSG_TYPES = new Set([
  'sess_create', 'sess_list', 'sess_connect', 'sess_disconnect',
  'sess_delete', 'sess_refresh', 'sess_update',
  'agent_status', 'agent_install', 'transport_upgrade',
  'ice_req',
])

// Message types that go to the agent (data plane)
const DATA_MSG_TYPES = new Set([
  'sub', 'unsub', 'i', 'r', 'ping',
])

export const useTmuxStore = defineStore('tmux', () => {
  // ── State ─────────────────────────────────────────────────────────────
  const managedSessions = ref<ManagedSession[]>([])
  const activePane = ref<QualifiedPaneId | null>(null)
  const focusedSessionId = ref<string | null>(null)
  const claudeSessions = ref<Map<string, ClaudeSessionState>>(new Map())
  const connectionStatus = ref<'disconnected' | 'connecting' | 'connected' | 'reconnecting'>('disconnected')
  const ws = ref<WebSocket | null>(null)
  const showNewSessionDialog = ref(false)
  const agentStatuses = ref<Map<string, { status: string; agentId?: string; version?: string; quicPort?: number }>>(new Map())

  // Multi-session: per-session tmux trees
  const sessionTrees = ref<Map<string, TmuxSessionInfo[]>>(new Map())

  // Multi-session: set of currently connected session IDs
  const connectedSessionIds = ref<Set<string>>(new Set())

  // qualifiedPaneId → output handler (registered by TerminalPane components)
  const paneHandlers = new Map<string, PaneOutputHandler>()
  const claudeHandlers = new Map<string, (event: unknown) => void>()

  let reconnectTimeout: ReturnType<typeof setTimeout> | null = null
  let backoffMs = 1000

  // ── Dual-transport: WS (control) + per-session P2P (data) ──────────
  let wsSend: ((msg: Record<string, unknown>) => void) | null = null
  let wsUrl: string | null = null

  // Per-session P2P connections
  const p2pConnections = new Map<string, P2PConnection>()

  // ── Computed ──────────────────────────────────────────────────────────

  /** All panes across all connected sessions, qualified by session ID */
  const allPanes = computed(() => {
    const result: (TmuxPaneInfo & { qualifiedId: QualifiedPaneId; sessionId: string; sessionName: string })[] = []
    for (const [sessionId, trees] of sessionTrees.value) {
      const ms = managedSessions.value.find(s => s.id === sessionId)
      const sessionName = ms?.name || sessionId
      for (const tree of trees) {
        for (const win of tree.windows) {
          for (const pane of win.panes) {
            result.push({
              ...pane,
              qualifiedId: qualifyPaneId(sessionId, pane.id),
              sessionId,
              sessionName,
            })
          }
        }
      }
    }
    return result
  })

  const claudePanes = computed(() =>
    allPanes.value.filter(p => p.isClaude)
  )

  const totalCostUsd = computed(() =>
    Array.from(claudeSessions.value.values())
      .reduce((sum, s) => sum + s.totalCostUsd, 0)
  )

  /** Get transport mode for a specific session */
  function getTransportMode(sessionId: string): 'ssh' | 'quic_p2p' | 'webrtc_p2p' {
    const p2p = p2pConnections.get(sessionId)
    return p2p?.mode || 'ssh'
  }

  /** Legacy: single active transport mode (for focused session) */
  const activeTransportMode = computed(() =>
    focusedSessionId.value ? getTransportMode(focusedSessionId.value) : 'ssh'
  )

  // Backward compat: sessions array for the focused session
  const sessions = computed(() =>
    focusedSessionId.value ? (sessionTrees.value.get(focusedSessionId.value) || []) : []
  )

  // Backward compat
  const activeSessionId = computed({
    get: () => focusedSessionId.value,
    set: (v) => { focusedSessionId.value = v },
  })

  // ── WS Connection ────────────────────────────────────────────────────

  function connect(url: string) {
    if (ws.value?.readyState === WebSocket.OPEN) return

    wsUrl = url
    connectionStatus.value = 'connecting'
    const socket = new WebSocket(url)
    socket.binaryType = 'arraybuffer'

    socket.onopen = () => {
      connectionStatus.value = 'connected'
      backoffMs = 1000
      wsSend = (msg) => {
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(encode(msg))
        }
      }
      sendMsg({ t: 'sess_list' })

      // Restore previously connected sessions after reconnect
      for (const sid of connectedSessionIds.value) {
        console.log('[oxmux] restoring session after reconnect:', sid)
        sendMsg({ t: 'sess_connect', session_id: sid })
      }
    }

    socket.onmessage = (ev: MessageEvent<ArrayBuffer>) => {
      try {
        const msg = decode(new Uint8Array(ev.data)) as Record<string, unknown>
        // When any P2P is active, skip pane output from WS for P2P-connected sessions
        if (msg.t === 'o') {
          // TODO: once server sends session_id with output, filter per-session
          // For now, skip all WS output if ANY P2P connection exists
          if (p2pConnections.size > 0) return
        }
        handleServerMsg(msg)
      } catch (e) {
        console.error('Failed to decode server message', e)
      }
    }

    socket.onclose = () => {
      connectionStatus.value = 'reconnecting'
      wsSend = null
      scheduleReconnect(url)
    }

    socket.onerror = () => {
      connectionStatus.value = 'reconnecting'
    }

    ws.value = socket
  }

  async function connectQuic(url: string, token: string) {
    const { connectQuic: doConnect } = await import('@/composables/useQuic')

    connectionStatus.value = 'connecting'
    try {
      const conn = await doConnect(url, token)
      connectionStatus.value = 'connected'
      backoffMs = 1000
      wsSend = (msg) => conn.send(msg)
      conn.onMessage((msg) => handleServerMsg(msg))
      conn.onClose(() => { connectionStatus.value = 'disconnected'; wsSend = null })
      sendMsg({ t: 'sess_list' })
    } catch (e) {
      console.error('[oxmux] QUIC connection failed:', e)
      connectionStatus.value = 'disconnected'
    }
  }

  // ── P2P Connections (per-session) ────────────────────────────────────

  async function connectWebRtcTransport(sessionId: string, agentHost: string, agentPort: number, token: string, certHash?: ArrayBuffer) {
    const { connectWebRtc } = await import('@/composables/useWebRtc')
    const { connectQuic: doQuic } = await import('@/composables/useQuic')

    try {
      const quicUrl = `https://${agentHost}:${agentPort}`
      console.log(`[oxmux] [${sessionId.slice(0, 8)}] opening QUIC signaling for WebRTC`)
      const quicConn = await doQuic(quicUrl, token, certHash)

      const sess = managedSessions.value.find(s => s.id === sessionId)
      quicConn.send({ t: 'sess_connect', name: sess?.name || 'default' })

      const keepAlive = setInterval(() => quicConn.send({ t: 'ping', ts: Date.now() }), 5000)

      const signalResolvers = new Map<string, (payload: Record<string, unknown>) => void>()

      quicConn.onMessage((msg) => {
        if (msg.t === 'webrtc_answer' || msg.t === 'webrtc_error') {
          const handler = signalResolvers.get('signal')
          if (handler && msg.t === 'webrtc_answer') {
            handler({ type: 'answer', sdp: msg.sdp })
          }
        } else if (msg.t !== 'o') {
          handleServerMsg(msg, sessionId)
        }
      })

      const iceRes = await fetch(`/api/ice-config?user=webrtc`)
      const raw = await iceRes.json()
      const iceConfig = {
        iceServers: (raw.ice_servers || []).map((s: any) => ({
          urls: s.urls, username: s.username, credential: s.credential,
        })),
      }

      await new Promise(r => setTimeout(r, 1000))

      const conn = await connectWebRtc(
        iceConfig,
        (payload) => {
          if (payload.type === 'offer') quicConn.send({ t: 'webrtc_offer', sdp: payload.sdp })
          else if (payload.type === 'ice') quicConn.send({ t: 'webrtc_ice', candidate: payload.candidate })
        },
        (handler) => { signalResolvers.set('signal', handler) },
      )

      // Register per-session P2P
      p2pConnections.set(sessionId, {
        sessionId,
        send: (msg) => conn.send(msg),
        close: () => { clearInterval(keepAlive); conn.close(); quicConn.close() },
        mode: 'webrtc_p2p',
      })

      conn.onMessage((msg) => handleServerMsg(msg, sessionId))
      conn.onClose(() => {
        console.warn(`[oxmux] [${sessionId.slice(0, 8)}] WebRTC P2P lost`)
        teardownP2P(sessionId)
      })

      console.log(`[oxmux] [${sessionId.slice(0, 8)}] WebRTC P2P connected!`)

      // Re-subscribe panes for this session via P2P
      resubscribeSessionPanes(sessionId)
    } catch (e) {
      console.warn(`[oxmux] [${sessionId.slice(0, 8)}] WebRTC failed, falling back to QUIC:`, e)
      teardownP2P(sessionId)

      try {
        await connectQuicP2P(sessionId, agentHost, agentPort, token, certHash)
      } catch (quicErr) {
        console.error(`[oxmux] [${sessionId.slice(0, 8)}] QUIC fallback also failed:`, quicErr)
        teardownP2P(sessionId)
      }
    }
  }

  async function connectQuicP2P(sessionId: string, host: string, port: number, token: string, certHash?: ArrayBuffer) {
    const { connectQuic: doConnect } = await import('@/composables/useQuic')

    const url = `https://${host}:${port}`
    console.log(`[oxmux] [${sessionId.slice(0, 8)}] connecting QUIC P2P to ${url}`)
    const conn = await doConnect(url, token, certHash)

    p2pConnections.set(sessionId, {
      sessionId,
      send: (msg) => conn.send(msg),
      close: () => conn.close(),
      mode: 'quic_p2p',
    })

    conn.onMessage((msg) => handleServerMsg(msg, sessionId))
    conn.onClose(() => {
      console.warn(`[oxmux] [${sessionId.slice(0, 8)}] QUIC P2P lost`)
      teardownP2P(sessionId)
    })

    console.log(`[oxmux] [${sessionId.slice(0, 8)}] QUIC P2P connected!`)

    const sess = managedSessions.value.find(s => s.id === sessionId)
    if (sess) {
      p2pConnections.get(sessionId)!.send({ t: 'sess_connect', name: sess.name })
    }

    resubscribeSessionPanes(sessionId)
  }

  /** Re-subscribe panes belonging to a session via its P2P connection */
  function resubscribeSessionPanes(sessionId: string) {
    const p2p = p2pConnections.get(sessionId)
    if (!p2p) return

    for (const qid of paneHandlers.keys()) {
      const { sessionId: sid, paneId } = parseQualifiedPaneId(qid)
      if (sid === sessionId) {
        p2p.send({ t: 'sub', pane: paneId })
      }
    }
  }

  function teardownP2P(sessionId?: string) {
    if (sessionId) {
      const conn = p2pConnections.get(sessionId)
      if (conn) {
        try { conn.close() } catch { /* ignore */ }
        p2pConnections.delete(sessionId)
      }
    } else {
      // Tear down all P2P connections
      for (const [, conn] of p2pConnections) {
        try { conn.close() } catch { /* ignore */ }
      }
      p2pConnections.clear()
    }
  }

  // ── Message Routing ──────────────────────────────────────────────────

  function scheduleReconnect(url: string) {
    if (reconnectTimeout) clearTimeout(reconnectTimeout)
    reconnectTimeout = setTimeout(() => {
      backoffMs = Math.min(backoffMs * 2, 30_000)
      connect(url)
    }, backoffMs)
  }

  /**
   * Route messages to the correct transport.
   * Data messages use the session's P2P connection if available.
   * Control messages always go via WS.
   */
  function sendMsg(msg: Record<string, unknown>) {
    const t = msg.t as string

    if (DATA_MSG_TYPES.has(t)) {
      // For data messages, try to find the session's P2P connection
      const qid = (msg.pane as string) || ''
      const { sessionId } = parseQualifiedPaneId(qid)
      const p2p = sessionId ? p2pConnections.get(sessionId) : null

      // Strip qualified prefix for the wire protocol (agent expects raw pane ID)
      const rawMsg = { ...msg }
      if (qid && sessionId) {
        rawMsg.pane = parseQualifiedPaneId(qid).paneId
      }

      const sender = p2p?.send || wsSend
      if (!sender) {
        console.warn('[oxmux] sendMsg: not connected, dropping:', t)
        return
      }
      sender(rawMsg)
      return
    }

    if (CONTROL_MSG_TYPES.has(t)) {
      if (!wsSend) {
        console.warn('[oxmux] sendMsg: WS not connected, dropping:', t)
        return
      }
      wsSend(msg)
      return
    }

    const sender = wsSend
    if (!sender) {
      console.warn('[oxmux] sendMsg: not connected, dropping:', t)
      return
    }
    sender(msg)
  }

  // ── Server Message Handling ──────────────────────────────────────────

  /**
   * Handle a message from server or agent.
   * @param sourceSessionId - if from a P2P agent, the session ID it belongs to
   */
  function handleServerMsg(msg: Record<string, unknown>, sourceSessionId?: string) {
    if (msg.t !== 'o' && msg.t !== 'pong') {
      console.log('[oxmux] received:', msg.t, msg)
    }

    switch (msg.t) {
      case 'o': {
        // Pane output — qualify the pane ID with the source session
        const rawPaneId = msg.pane as string
        const sid = sourceSessionId || (msg.session_id as string) || focusedSessionId.value || ''
        const qid = qualifyPaneId(sid, rawPaneId)
        const handler = paneHandlers.get(qid)
        handler?.(msg.data as Uint8Array)
        break
      }
      case 's': {
        const sid = sourceSessionId || focusedSessionId.value
        if (sid) {
          sessionTrees.value.set(sid, msg.sessions as TmuxSessionInfo[])
        }
        break
      }
      case 'e': {
        applyTmuxEvent(msg.event as Record<string, unknown>, sourceSessionId)
        break
      }
      case 'c': {
        const handler = claudeHandlers.get(msg.session_id as string)
        handler?.(msg.event)
        break
      }
      case 'ca': {
        claudeSessions.value.set(msg.session_id as string, msg.state as ClaudeSessionState)
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

      // ── Session management ──────────────────────────────────────────
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
        disconnectSessionCleanup(id)
        break
      }
      case 'sess_connected': {
        const session = msg.session as ManagedSession
        const idx = managedSessions.value.findIndex(s => s.id === session.id)
        if (idx >= 0) {
          managedSessions.value[idx] = session
          connectedSessionIds.value.add(session.id)

          if (session.tmux_sessions?.length) {
            sessionTrees.value.set(session.id, session.tmux_sessions)
          }

          // Set as focused if nothing focused
          if (!focusedSessionId.value) {
            focusedSessionId.value = session.id
          }

          const host = (session.transport?.backend as any)?.host
          if (host && session.transport?.backend?.type === 'ssh') {
            checkAgentStatus(host)
          }
        }
        break
      }
      case 'sess_disconnected': {
        const session = msg.session as ManagedSession
        const idx = managedSessions.value.findIndex(s => s.id === session.id)
        if (idx >= 0) managedSessions.value[idx] = session
        disconnectSessionCleanup(session.id)
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
        const certHashHex = msg.cert_hash as string | undefined
        console.log(`[oxmux] transport upgrade ready: ${host}:${port} (${target || 'quic_p2p'})${certHashHex ? ' cert_hash=' + certHashHex.slice(0, 16) + '...' : ''}`)

        // Convert hex cert hash to ArrayBuffer for WebTransport cert pinning
        let certHash: ArrayBuffer | undefined
        if (certHashHex && /^[0-9a-f]{64}$/.test(certHashHex)) {
          const bytes = new Uint8Array(certHashHex.match(/.{2}/g)!.map(b => parseInt(b, 16)))
          certHash = bytes.buffer
        }

        if (target === 'webrtc_p2p') {
          connectWebRtcTransport(sid, host, port, token, certHash)
        } else {
          connectQuicP2P(sid, host, port, token, certHash)
        }
        break
      }
      case 'transport_upgrade_failed': {
        console.error(`[oxmux] transport upgrade failed: ${msg.error}`)
        break
      }
    }
  }

  function applyTmuxEvent(event: Record<string, unknown>, sessionId?: string) {
    const sid = sessionId || focusedSessionId.value
    if (!sid) return

    const trees = sessionTrees.value.get(sid) || []
    switch (event.k) {
      case 'session_created':
        trees.push({ id: event.id as string, name: event.name as string, windows: [] })
        sessionTrees.value.set(sid, [...trees])
        break
      case 'session_closed':
        sessionTrees.value.set(sid, trees.filter(s => s.id !== event.id))
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
    if (wsSend) {
      wsSend({ t: 'sess_disconnect', session_id: sessionId })
    }
    disconnectSessionCleanup(sessionId)
  }

  /** Clean up state for a single disconnected session */
  function disconnectSessionCleanup(sessionId: string) {
    teardownP2P(sessionId)

    // Unsubscribe panes belonging to this session
    for (const qid of [...paneHandlers.keys()]) {
      const { sessionId: sid } = parseQualifiedPaneId(qid)
      if (sid === sessionId) {
        paneHandlers.delete(qid)
      }
    }

    connectedSessionIds.value.delete(sessionId)
    sessionTrees.value.delete(sessionId)

    if (focusedSessionId.value === sessionId) {
      // Focus the next connected session, or null
      const next = [...connectedSessionIds.value][0] || null
      focusedSessionId.value = next
    }

    if (activePane.value) {
      const { sessionId: sid } = parseQualifiedPaneId(activePane.value)
      if (sid === sessionId) {
        activePane.value = null
      }
    }
  }

  function deleteSession(sessionId: string) {
    disconnectSessionCleanup(sessionId)
    sendMsg({ t: 'sess_delete', session_id: sessionId })
  }

  function refreshSession(sessionId: string) {
    if (wsSend) {
      wsSend({ t: 'sess_refresh', session_id: sessionId })
    }
  }

  function listSessions() {
    sendMsg({ t: 'sess_list' })
  }

  /** Legacy: clean up all sessions */
  function cleanupSession() {
    teardownP2P()
    paneHandlers.clear()
    connectedSessionIds.value.clear()
    sessionTrees.value.clear()
    focusedSessionId.value = null
    activePane.value = null
  }

  // ── Pane Operations (use qualified IDs) ─────────────────────────────

  function subscribePane(qualifiedId: QualifiedPaneId, handler: PaneOutputHandler) {
    paneHandlers.set(qualifiedId, handler)

    const { sessionId, paneId } = parseQualifiedPaneId(qualifiedId)
    const p2p = sessionId ? p2pConnections.get(sessionId) : null

    const rawMsg = { t: 'sub', pane: paneId }
    const sender = p2p?.send || wsSend
    if (sender) sender(rawMsg)
  }

  function unsubscribePane(qualifiedId: QualifiedPaneId) {
    paneHandlers.delete(qualifiedId)

    const { sessionId, paneId } = parseQualifiedPaneId(qualifiedId)
    const p2p = sessionId ? p2pConnections.get(sessionId) : null

    const rawMsg = { t: 'unsub', pane: paneId }
    const sender = p2p?.send || wsSend
    if (sender) sender(rawMsg)
  }

  function sendInput(qualifiedId: QualifiedPaneId, data: Uint8Array) {
    sendMsg({ t: 'i', pane: qualifiedId, data })
  }

  function sendResize(qualifiedId: QualifiedPaneId, cols: number, rows: number) {
    sendMsg({ t: 'r', pane: qualifiedId, cols, rows })
  }

  // ── ICE / Claude / Ping ─────────────────────────────────────────────

  const iceConfigResolvers = new Map<string, (config: IceConfig) => void>()
  const lastPong = ref<number>(0)

  async function requestIceConfig(peerId: string): Promise<IceConfig> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { iceConfigResolvers.delete(peerId); reject(new Error('ICE config timeout')) }, 10000)
      iceConfigResolvers.set(peerId, (config) => { clearTimeout(timer); resolve(config) })
      sendMsg({ t: 'ice_req', peer_id: peerId })
    })
  }

  function subscribeClaudeSession(sessionId: string, handler: (event: unknown) => void) {
    claudeHandlers.set(sessionId, handler)
  }

  function ping() { sendMsg({ t: 'ping', ts: Date.now() }) }

  // ── Agent Management ────────────────────────────────────────────────

  function checkAgentStatus(host: string) {
    sendMsg({ t: 'agent_status', host })
  }

  function installAgent(sessionId: string) {
    sendMsg({ t: 'agent_install', session_id: sessionId })
  }

  function upgradeTransport(sessionId: string, target: 'quic_p2p' | 'webrtc_p2p') {
    sendMsg({ t: 'transport_upgrade', session_id: sessionId, target })
  }

  // ── Return ──────────────────────────────────────────────────────────

  return {
    // State
    sessions,
    managedSessions,
    activePane,
    activeSessionId,
    focusedSessionId,
    claudeSessions,
    connectionStatus,
    allPanes,
    claudePanes,
    totalCostUsd,
    lastPong,
    showNewSessionDialog,
    agentStatuses,
    activeTransportMode,
    sessionTrees,
    connectedSessionIds,
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
    getTransportMode,
    // Transport management
    teardownP2P,
    cleanupSession,
    disconnectSessionCleanup,
  }
})
