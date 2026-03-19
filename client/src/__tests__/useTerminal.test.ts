import { describe, it, expect, vi, beforeEach } from 'vitest'
import { qualifyPaneId, parseQualifiedPaneId } from '@/utils/paneId'

// Test the pane ID flow through the terminal lifecycle
// (We can't easily test xterm.js in happy-dom, but we can test the ID handling)

describe('useTerminal — qualified pane ID flow', () => {
  it('subscribe uses qualified pane ID', () => {
    const qid = qualifyPaneId('session-abc', '%0')
    expect(qid).toBe('session-abc::%0')

    // When sent to the server, the store strips the session prefix
    const { sessionId, paneId } = parseQualifiedPaneId(qid)
    expect(sessionId).toBe('session-abc')
    expect(paneId).toBe('%0')
  })

  it('sendInput preserves qualified pane ID for routing', () => {
    const qid = qualifyPaneId('session-xyz', '%1')
    // sendMsg receives qualified ID, parses it, routes to correct P2P
    const { sessionId, paneId } = parseQualifiedPaneId(qid)
    expect(sessionId).toBe('session-xyz')
    expect(paneId).toBe('%1')
  })

  it('output from server gets re-qualified correctly', () => {
    // Server sends: { t: 'o', pane: '%0', session_id: 'abc' }
    // Store qualifies: qualifyPaneId('abc', '%0') = 'abc::%0'
    // Looks up paneHandlers.get('abc::%0')
    const serverPane = '%0'
    const serverSessionId = 'abc'
    const qid = qualifyPaneId(serverSessionId, serverPane)
    expect(qid).toBe('abc::%0')
  })

  it('two sessions with same raw pane ID get different qualified IDs', () => {
    const mars = qualifyPaneId('mars-session', '%0')
    const zeus = qualifyPaneId('zeus-session', '%0')
    expect(mars).not.toBe(zeus)
    expect(mars).toBe('mars-session::%0')
    expect(zeus).toBe('zeus-session::%0')
  })
})
