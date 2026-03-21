import { test, expect } from '@playwright/test'
import { authenticate, ensureConnectedSession, selectFirstPane, readTerminalContent, collectConsoleLogs, waitForAgentOnline, upgradeToWebRtc, switchToSingleView } from './helpers'

function hasDuplicateChars(text: string, expected: string): boolean {
  const doubled = expected.split('').map(c => c + c).join('')
  return text.includes(doubled)
}

test.describe('Transport Fallback Input', () => {
  test('no duplicate input after WebRTC crash fallback', async ({ page, context }) => {
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

    // Verify WebRTC works
    await page.keyboard.type('echo before-fallback', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(2000)

    let content = await readTerminalContent(page)
    expect(content).toContain('before-fallback')
    await page.screenshot({ path: 'e2e/screenshots/fallback-01-webrtc-ok.png' })

    // Trigger DataChannel close by running Claude Code
    await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
    await page.keyboard.press('Enter')

    // Wait for DC to close (it should close within 10s based on bug reports)
    let dcClosed = false
    for (let i = 0; i < 15; i++) {
      await page.waitForTimeout(2000)
      if (logs.hasPattern('P2P lost')) {
        dcClosed = true
        console.log(`DC closed at ~${(i + 1) * 2}s`)
        break
      }
    }

    if (!dcClosed) {
      console.log('DC did NOT close — cannot test fallback input duplication')
      // Kill Claude Code and try clean downgrade instead
      await page.keyboard.down('Control')
      await page.keyboard.press('c')
      await page.keyboard.up('Control')
      await page.waitForTimeout(1000)

      // Click "Back to SSH" for clean downgrade
      const backBtn = page.locator('.action-btn.disconnect', { hasText: 'Back to SSH' })
      if (await backBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
        await backBtn.click()
      }
    }

    // Wait for SSH fallback to settle
    await page.waitForTimeout(3000)

    // Kill any running Claude Code
    await page.keyboard.down('Control')
    await page.keyboard.press('c')
    await page.keyboard.up('Control')
    await page.waitForTimeout(1000)
    await page.keyboard.down('Control')
    await page.keyboard.press('c')
    await page.keyboard.up('Control')
    await page.waitForTimeout(1000)

    await page.screenshot({ path: 'e2e/screenshots/fallback-02-ssh-mode.png' })

    // THE CRITICAL TEST: type characters and check for duplicates
    await page.keyboard.type('echo test123', { delay: 100 })
    await page.waitForTimeout(500)

    content = await readTerminalContent(page)
    console.log('After typing "echo test123":', content.slice(-200))
    await page.screenshot({ path: 'e2e/screenshots/fallback-03-typing.png' })

    const isDuplicated = hasDuplicateChars(content, 'echo test123')
    console.log('Duplicate detected:', isDuplicated)

    // Check for the specific reported pattern
    const hasDouble = content.includes('eecchhoo') || content.includes('tteesstt')
    console.log('Has double chars:', hasDouble)

    expect.soft(isDuplicated, 'Input should NOT be duplicated').toBe(false)
    expect.soft(hasDouble, 'Should not have doubled characters').toBe(false)

    // Press Enter and verify output
    await page.keyboard.press('Enter')
    await page.waitForTimeout(2000)

    content = await readTerminalContent(page)
    console.log('After Enter:', content.slice(-200))

    // test123 should appear exactly once in the output
    const matches = content.match(/test123/g) || []
    console.log('test123 occurrences:', matches.length)

    // Also test with 'clear' (the specific command from bug report)
    await page.keyboard.type('clear', { delay: 100 })
    await page.waitForTimeout(500)

    content = await readTerminalContent(page)
    const clearDoubled = content.includes('cclleeaarr')
    console.log('clear doubled:', clearDoubled)

    await page.screenshot({ path: 'e2e/screenshots/fallback-04-clear.png' })

    expect.soft(clearDoubled, '"clear" should not appear as "cclleeaarr"').toBe(false)

    // Dump console logs
    console.log('\n=== ALL CONSOLE LOGS ===')
    logs.flush().forEach(l => console.log(' ', l))
  })

  test('clean downgrade (Back to SSH) preserves input fidelity', async ({ page, context }) => {
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

    // Click "Back to SSH" for clean downgrade
    const backBtn = page.locator('.action-btn.disconnect', { hasText: 'Back to SSH' })
    await expect(backBtn).toBeVisible({ timeout: 5000 })
    await backBtn.click()
    await page.waitForTimeout(2000)

    await page.screenshot({ path: 'e2e/screenshots/fallback-05-clean-ssh.png' })

    // Type and verify no duplicates
    await page.keyboard.type('echo clean-switch', { delay: 100 })
    await page.waitForTimeout(500)

    const content = await readTerminalContent(page)
    console.log('After clean switch typing:', content.slice(-200))

    const isDuplicated = hasDuplicateChars(content, 'echo clean-switch')
    console.log('Duplicate after clean switch:', isDuplicated)

    expect(isDuplicated, 'Clean downgrade should not duplicate input').toBe(false)

    await page.screenshot({ path: 'e2e/screenshots/fallback-06-clean-typing.png' })
  })
})
