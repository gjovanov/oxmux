import { describe, it, expect } from 'vitest'
import { qualifyPaneId, parseQualifiedPaneId, isQualifiedPaneId } from '@/utils/paneId'

describe('qualifyPaneId', () => {
  it('creates qualified ID from session and pane', () => {
    expect(qualifyPaneId('abc-123', '%0')).toBe('abc-123::%0')
  })

  it('handles UUID session IDs', () => {
    const sid = 'a1b2c3d4-e5f6-7890-abcd-ef1234567890'
    expect(qualifyPaneId(sid, '%3')).toBe(`${sid}::%3`)
  })

  it('handles empty session ID', () => {
    expect(qualifyPaneId('', '%0')).toBe('::%0')
  })

  it('handles empty pane ID', () => {
    expect(qualifyPaneId('abc', '')).toBe('abc::')
  })
})

describe('parseQualifiedPaneId', () => {
  it('parses qualified ID into components', () => {
    const result = parseQualifiedPaneId('abc-123::%0')
    expect(result).toEqual({ sessionId: 'abc-123', paneId: '%0' })
  })

  it('handles UUID session IDs', () => {
    const sid = 'a1b2c3d4-e5f6-7890-abcd-ef1234567890'
    const result = parseQualifiedPaneId(`${sid}::%3`)
    expect(result).toEqual({ sessionId: sid, paneId: '%3' })
  })

  it('handles unqualified pane ID (legacy)', () => {
    const result = parseQualifiedPaneId('%0')
    expect(result).toEqual({ sessionId: '', paneId: '%0' })
  })

  it('handles empty components', () => {
    expect(parseQualifiedPaneId('::%0')).toEqual({ sessionId: '', paneId: '%0' })
    expect(parseQualifiedPaneId('abc::')).toEqual({ sessionId: 'abc', paneId: '' })
  })

  it('round-trips with qualifyPaneId', () => {
    const sid = 'session-xyz'
    const pid = '%42'
    const qualified = qualifyPaneId(sid, pid)
    const parsed = parseQualifiedPaneId(qualified)
    expect(parsed).toEqual({ sessionId: sid, paneId: pid })
  })

  it('handles pane ID containing special chars', () => {
    const result = parseQualifiedPaneId('abc::%0:extra')
    // Only first :: is the separator
    expect(result).toEqual({ sessionId: 'abc', paneId: '%0:extra' })
  })
})

describe('isQualifiedPaneId', () => {
  it('returns true for qualified IDs', () => {
    expect(isQualifiedPaneId('abc::%0')).toBe(true)
  })

  it('returns false for unqualified IDs', () => {
    expect(isQualifiedPaneId('%0')).toBe(false)
    expect(isQualifiedPaneId('')).toBe(false)
  })

  it('returns true for edge cases with separator', () => {
    expect(isQualifiedPaneId('::')).toBe(true)
    expect(isQualifiedPaneId('abc::')).toBe(true)
    expect(isQualifiedPaneId('::%0')).toBe(true)
  })
})
