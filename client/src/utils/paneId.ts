/**
 * Qualified Pane ID utilities.
 *
 * tmux pane IDs like %0, %1 are only unique within a single tmux server.
 * When multiple sessions connect to different hosts (mars, zeus), both may have %0.
 * Qualified pane IDs scope them: "sessionId::%0" is globally unique.
 */

/** Separator between session ID and pane ID */
const SEPARATOR = '::'

/** A globally unique pane identifier: `{managedSessionId}::{tmuxPaneId}` */
export type QualifiedPaneId = string

/**
 * Create a qualified pane ID from session and pane IDs.
 * @example qualifyPaneId('abc-123', '%0') // 'abc-123::%0'
 */
export function qualifyPaneId(sessionId: string, paneId: string): QualifiedPaneId {
  return `${sessionId}${SEPARATOR}${paneId}`
}

/**
 * Parse a qualified pane ID into session and pane components.
 * @example parseQualifiedPaneId('abc-123::%0') // { sessionId: 'abc-123', paneId: '%0' }
 */
export function parseQualifiedPaneId(qid: QualifiedPaneId): { sessionId: string; paneId: string } {
  const idx = qid.indexOf(SEPARATOR)
  if (idx === -1) {
    // Unqualified — treat as legacy (no session scope)
    return { sessionId: '', paneId: qid }
  }
  return {
    sessionId: qid.substring(0, idx),
    paneId: qid.substring(idx + SEPARATOR.length),
  }
}

/**
 * Check if a pane ID is qualified (contains session scope).
 */
export function isQualifiedPaneId(id: string): boolean {
  return id.includes(SEPARATOR)
}
