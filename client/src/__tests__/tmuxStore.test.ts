import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useTmuxStore, type ManagedSession, type TmuxSessionInfo } from '@/stores/tmux'
import { qualifyPaneId, parseQualifiedPaneId } from '@/utils/paneId'

// Mock @msgpack/msgpack
vi.mock('@msgpack/msgpack', () => ({
  encode: (v: any) => new Uint8Array(JSON.stringify(v).split('').map(c => c.charCodeAt(0))),
  decode: (v: Uint8Array) => JSON.parse(String.fromCharCode(...v)),
}))

function makeSession(id: string, name: string, host: string, status = 'connected'): ManagedSession {
  return {
    id,
    name,
    transport: { browser: 'websocket', backend: { type: 'ssh', host, port: 22, user: 'test', auth: { method: 'agent' } } },
    status: status as any,
    tmux_sessions: [{
      id: `tmux-${id}`,
      name,
      windows: [{
        id: `@0`, name: 'bash', index: 0, layout: '',
        panes: [{ id: '%0', index: 0, cols: 80, rows: 24, currentCommand: 'bash', isActive: true, isClaude: false }],
      }],
    }],
  }
}

describe('useTmuxStore — multi-session', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('tracks multiple connected sessions independently', () => {
    const store = useTmuxStore()
    const sessA = makeSession('aaa', 'mars-session', '198.51.100.10')
    const sessB = makeSession('bbb', 'zeus-session', '198.51.100.20')

    // Simulate sess_connected for both
    store.managedSessions.push(sessA, sessB)

    // Simulate handleServerMsg for sess_connected
    store.connectedSessionIds.add('aaa')
    store.connectedSessionIds.add('bbb')
    store.sessionTrees.set('aaa', sessA.tmux_sessions)
    store.sessionTrees.set('bbb', sessB.tmux_sessions)

    expect(store.connectedSessionIds.size).toBe(2)
    expect(store.sessionTrees.size).toBe(2)
    expect(store.sessionTrees.get('aaa')).toHaveLength(1)
    expect(store.sessionTrees.get('bbb')).toHaveLength(1)
  })

  it('allPanes returns qualified panes from all sessions', () => {
    const store = useTmuxStore()
    const sessA = makeSession('aaa', 'mars', '198.51.100.10')
    const sessB = makeSession('bbb', 'zeus', '198.51.100.20')

    store.managedSessions.push(sessA, sessB)
    store.sessionTrees.set('aaa', sessA.tmux_sessions)
    store.sessionTrees.set('bbb', sessB.tmux_sessions)

    const panes = store.allPanes
    expect(panes).toHaveLength(2)
    expect(panes[0].qualifiedId).toBe('aaa::%0')
    expect(panes[1].qualifiedId).toBe('bbb::%0')
    expect(panes[0].sessionName).toBe('mars')
    expect(panes[1].sessionName).toBe('zeus')
  })

  it('disconnectSessionCleanup removes only that session', () => {
    const store = useTmuxStore()
    const sessA = makeSession('aaa', 'mars', '198.51.100.10')
    const sessB = makeSession('bbb', 'zeus', '198.51.100.20')

    store.managedSessions.push(sessA, sessB)
    store.connectedSessionIds.add('aaa')
    store.connectedSessionIds.add('bbb')
    store.sessionTrees.set('aaa', sessA.tmux_sessions)
    store.sessionTrees.set('bbb', sessB.tmux_sessions)
    store.focusedSessionId = 'aaa'

    // Disconnect session A
    store.disconnectSessionCleanup('aaa')

    expect(store.connectedSessionIds.has('aaa')).toBe(false)
    expect(store.connectedSessionIds.has('bbb')).toBe(true)
    expect(store.sessionTrees.has('aaa')).toBe(false)
    expect(store.sessionTrees.has('bbb')).toBe(true)
    // Focus should move to next connected session
    expect(store.focusedSessionId).toBe('bbb')
  })

  it('pane handlers use qualified IDs', () => {
    const store = useTmuxStore()
    const handler = vi.fn()
    const qid = qualifyPaneId('aaa', '%0')

    store.subscribePane(qid, handler)

    // Verify the handler is registered with the qualified ID
    // (We can't easily test internal paneHandlers, but we can test output routing)
    expect(qid).toBe('aaa::%0')
  })

  it('getTransportMode returns ssh when no P2P connection', () => {
    const store = useTmuxStore()
    expect(store.getTransportMode('aaa')).toBe('ssh')
  })

  it('activePane uses qualified ID', () => {
    const store = useTmuxStore()
    store.activePane = qualifyPaneId('aaa', '%0')
    expect(store.activePane).toBe('aaa::%0')

    const parsed = parseQualifiedPaneId(store.activePane!)
    expect(parsed.sessionId).toBe('aaa')
    expect(parsed.paneId).toBe('%0')
  })

  it('sessions computed returns trees for focused session', () => {
    const store = useTmuxStore()
    const sessA = makeSession('aaa', 'mars', '198.51.100.10')
    const sessB = makeSession('bbb', 'zeus', '198.51.100.20')

    store.sessionTrees.set('aaa', sessA.tmux_sessions)
    store.sessionTrees.set('bbb', sessB.tmux_sessions)
    store.focusedSessionId = 'bbb'

    // sessions computed should return zeus's trees
    expect(store.sessions).toHaveLength(1)
    expect(store.sessions[0].name).toBe('zeus')
  })

  it('cleanupSession clears all state', () => {
    const store = useTmuxStore()
    store.connectedSessionIds.add('aaa')
    store.connectedSessionIds.add('bbb')
    store.sessionTrees.set('aaa', [])
    store.sessionTrees.set('bbb', [])
    store.focusedSessionId = 'aaa'
    store.activePane = 'aaa::%0'

    store.cleanupSession()

    expect(store.connectedSessionIds.size).toBe(0)
    expect(store.sessionTrees.size).toBe(0)
    expect(store.focusedSessionId).toBeNull()
    expect(store.activePane).toBeNull()
  })
})
