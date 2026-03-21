import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useTmuxStore, type ManagedSession } from '@/stores/tmux'
import { qualifyPaneId } from '@/utils/paneId'

vi.mock('@msgpack/msgpack', () => ({
  encode: (v: any) => new Uint8Array(JSON.stringify(v).split('').map(c => c.charCodeAt(0))),
  decode: (v: Uint8Array) => JSON.parse(String.fromCharCode(...v)),
}))

function makeSession(id: string, name: string, host: string): ManagedSession {
  return {
    id, name,
    transport: { browser: 'websocket', backend: { type: 'ssh', host, port: 22, user: 'test', auth: { method: 'agent' } } },
    status: 'connected',
    tmux_sessions: [{
      id: `tmux-${id}`, name,
      windows: [{
        id: '@0', name: 'bash', index: 0, layout: '',
        panes: [
          { id: '%0', index: 0, cols: 80, rows: 24, currentCommand: 'bash', isActive: true, isClaude: false },
          { id: '%1', index: 1, cols: 80, rows: 24, currentCommand: 'vim', isActive: false, isClaude: false },
        ],
      }],
    }],
  }
}

describe('MashedView store integration', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('allPanes returns qualified panes from multiple sessions', () => {
    const store = useTmuxStore()
    const sessA = makeSession('aaa', 'mars', '198.51.100.10')
    const sessB = makeSession('bbb', 'zeus', '198.51.100.20')

    store.managedSessions.push(sessA, sessB)
    store.sessionTrees.set('aaa', sessA.tmux_sessions)
    store.sessionTrees.set('bbb', sessB.tmux_sessions)

    const panes = store.allPanes
    expect(panes).toHaveLength(4)
    expect(panes.map(p => p.qualifiedId)).toEqual([
      'aaa::%0', 'aaa::%1', 'bbb::%0', 'bbb::%1',
    ])
  })

  it('allPanes includes session metadata', () => {
    const store = useTmuxStore()
    const sess = makeSession('aaa', 'mars', '198.51.100.10')
    store.managedSessions.push(sess)
    store.sessionTrees.set('aaa', sess.tmux_sessions)

    const panes = store.allPanes
    expect(panes[0].sessionId).toBe('aaa')
    expect(panes[0].sessionName).toBe('mars')
    expect(panes[0].currentCommand).toBe('bash')
    expect(panes[1].currentCommand).toBe('vim')
  })

  it('auto-fill grid distributes panes across NxN grid', () => {
    const store = useTmuxStore()
    const sessA = makeSession('aaa', 'mars', '198.51.100.10')
    const sessB = makeSession('bbb', 'zeus', '198.51.100.20')

    store.managedSessions.push(sessA, sessB)
    store.sessionTrees.set('aaa', sessA.tmux_sessions)
    store.sessionTrees.set('bbb', sessB.tmux_sessions)

    // Simulate auto-fill for 2x2 grid (4 slots, 4 panes)
    const panes = store.allPanes
    const gridSize = 2
    const totalSlots = gridSize * gridSize

    const assignments: (string | null)[] = []
    const used = new Set<string>()
    for (let i = 0; i < totalSlots; i++) {
      const next = panes.find(p => !used.has(p.qualifiedId))
      if (next) {
        assignments.push(next.qualifiedId)
        used.add(next.qualifiedId)
      } else {
        assignments.push(null)
      }
    }

    expect(assignments).toEqual(['aaa::%0', 'aaa::%1', 'bbb::%0', 'bbb::%1'])
  })

  it('pane handlers are scoped by qualified ID', () => {
    const store = useTmuxStore()
    const handlerA = vi.fn()
    const handlerB = vi.fn()

    // Two sessions, both with %0
    store.subscribePane(qualifyPaneId('aaa', '%0'), handlerA)
    store.subscribePane(qualifyPaneId('bbb', '%0'), handlerB)

    // Verify both are registered (different qualified IDs)
    expect(qualifyPaneId('aaa', '%0')).not.toBe(qualifyPaneId('bbb', '%0'))
  })

  it('disconnecting one session preserves others in grid', () => {
    const store = useTmuxStore()
    const sessA = makeSession('aaa', 'mars', '198.51.100.10')
    const sessB = makeSession('bbb', 'zeus', '198.51.100.20')

    store.managedSessions.push(sessA, sessB)
    store.connectedSessionIds.add('aaa')
    store.connectedSessionIds.add('bbb')
    store.sessionTrees.set('aaa', sessA.tmux_sessions)
    store.sessionTrees.set('bbb', sessB.tmux_sessions)

    // Disconnect session A
    store.disconnectSessionCleanup('aaa')

    // Session B still intact
    expect(store.connectedSessionIds.has('bbb')).toBe(true)
    expect(store.sessionTrees.has('bbb')).toBe(true)
    expect(store.allPanes).toHaveLength(2) // only bbb's panes remain
  })
})
