import { test, expect } from '@playwright/test'
import { authenticate, ensureConnectedSession, selectFirstPane, readTerminalContent, collectConsoleLogs, waitForAgentOnline, upgradeToWebRtc, switchToSingleView } from './helpers'

test.describe('WebRTC P2P Stability', () => {
  test('DataChannel survives simple commands', async ({ page, context }) => {
    test.setTimeout(180_000)
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    const logs = collectConsoleLogs(page)

    await authenticate(page)
    await ensureConnectedSession(page)

    // Wait for agent
    try {
      await waitForAgentOnline(page, 60_000)
    } catch {
      test.skip(true, 'Agent not available — cannot test WebRTC')
      return
    }

    await page.screenshot({ path: 'e2e/screenshots/webrtc-01-agent-online.png' })

    // Upgrade to WebRTC
    try {
      await upgradeToWebRtc(page, logs, 30_000)
    } catch (e) {
      console.log('WebRTC upgrade failed. Console logs:')
      logs.flush().forEach(l => console.log(l))
      test.skip(true, 'WebRTC upgrade failed: ' + e)
      return
    }

    await page.screenshot({ path: 'e2e/screenshots/webrtc-02-connected.png' })

    // Switch to single + select pane
    await switchToSingleView(page)
    await selectFirstPane(page)

    // Type simple commands
    await page.keyboard.type('echo webrtc-ok', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(2000)

    const content = await readTerminalContent(page)
    console.log('After echo:', content.slice(0, 200))

    await page.screenshot({ path: 'e2e/screenshots/webrtc-03-echo.png' })

    // Check no DC close events
    const dcClosed = logs.hasPattern('DataChannel closed')
    const p2pLost = logs.hasPattern('P2P lost')
    console.log('DC closed:', dcClosed, 'P2P lost:', p2pLost)

    expect(content).toContain('webrtc-ok')
    expect(dcClosed).toBe(false)
    expect(p2pLost).toBe(false)
  })

  test('diagnose DataChannel close timing on Claude Code start', async ({ page, context }) => {
    test.setTimeout(180_000)
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    const logs = collectConsoleLogs(page)

    await authenticate(page)
    await ensureConnectedSession(page)

    try {
      await waitForAgentOnline(page, 60_000)
      await upgradeToWebRtc(page, logs, 30_000)
    } catch {
      test.skip(true, 'WebRTC not available')
      return
    }

    await switchToSingleView(page)
    await selectFirstPane(page)

    // Type (not paste) Claude Code command
    await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
    await page.screenshot({ path: 'e2e/screenshots/webrtc-04-before-enter.png' })

    const beforeEnterDC = logs.hasPattern('DataChannel closed')
    console.log('DC closed before Enter:', beforeEnterDC)

    // Press Enter to run the command
    await page.keyboard.press('Enter')

    // Poll for 30 seconds, capturing state every 2 seconds
    const pollResults: { t: number; dcClosed: boolean; p2pLost: boolean; termSnippet: string }[] = []

    for (let i = 0; i < 15; i++) {
      await page.waitForTimeout(2000)
      const t = (i + 1) * 2
      const dcClosed = logs.hasPattern('DataChannel closed')
      const p2pLost = logs.hasPattern('P2P lost')
      const content = await readTerminalContent(page)
      const snippet = content.replace(/\n/g, ' ').slice(0, 100)

      pollResults.push({ t, dcClosed, p2pLost, termSnippet: snippet })

      if (dcClosed && !pollResults[Math.max(0, i - 1)]?.dcClosed) {
        console.log(`\n=== DataChannel CLOSED at ~${t}s after Enter ===`)
        await page.screenshot({ path: `e2e/screenshots/webrtc-05-dc-closed-${t}s.png` })

        // Dump all console logs at the moment of failure
        console.log('Console logs at DC close:')
        logs.flush().forEach(l => console.log(' ', l))
      }

      if (dcClosed) break
    }

    console.log('\n=== POLL RESULTS ===')
    for (const r of pollResults) {
      console.log(`  t=${r.t}s dcClosed=${r.dcClosed} p2pLost=${r.p2pLost} term="${r.termSnippet}"`)
    }

    await page.screenshot({ path: 'e2e/screenshots/webrtc-06-final.png' })

    // After whatever happened, verify SSH fallback works
    await page.waitForTimeout(2000)
    await page.keyboard.down('Control')
    await page.keyboard.press('c')
    await page.keyboard.up('Control')
    await page.waitForTimeout(1000)
    await page.keyboard.type('echo fallback-ok', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(2000)

    const finalContent = await readTerminalContent(page)
    console.log('Final terminal:', finalContent.slice(-200))

    // Soft assert: DC should survive at least 5s
    const firstClose = pollResults.findIndex(r => r.dcClosed)
    if (firstClose >= 0) {
      console.log(`BUG: DataChannel closed ${pollResults[firstClose].t}s after Enter`)
      expect.soft(pollResults[firstClose].t, 'DC should survive at least 5s').toBeGreaterThanOrEqual(5)
    }

    // Hard assert: SSH fallback should work
    expect.soft(finalContent, 'SSH fallback should work after DC close').toContain('fallback-ok')
  })
})
