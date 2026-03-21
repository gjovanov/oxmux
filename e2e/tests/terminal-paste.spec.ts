import { test, expect, Page } from '@playwright/test'
import { BASE_URL, TEST_USER, TEST_PASS, SSH_HOST, SSH_USER, SSH_KEY, authenticate, ensureConnectedSession } from './helpers'

// Uses shared helpers from helpers.ts — credentials come from env vars

test.describe('Terminal Paste', () => {
  test('paste via dispatched ClipboardEvent on xterm textarea', async ({ page, context }) => {
    test.setTimeout(90_000)
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
    await authenticate(page)
    await ensureConnectedSession(page)

    // Click the first pane
    await page.locator('.pane-node').first().click()
    await expect(page.locator('.xterm-screen')).toBeVisible({ timeout: 10_000 })
    await page.locator('.terminal-pane').click()
    await page.waitForTimeout(1000)

    await page.screenshot({ path: 'e2e/screenshots/paste-before.png' })

    const pasteText = 'echo oxmux-paste-e2e-ok'

    // Write to clipboard via Clipboard API, then press Ctrl+V
    await page.evaluate(async (text) => {
      await navigator.clipboard.writeText(text)
    }, pasteText)

    // Ensure xterm textarea is focused
    await page.evaluate(() => {
      const ta = document.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement
      if (ta) ta.focus()
    })

    // First verify typing works
    await page.keyboard.type('echo typing-works', { delay: 50 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(2000)

    const typingOutput = await page.locator('[data-testid="terminal-accessible-output"]').textContent()
    console.log('After typing:', JSON.stringify(typingOutput))

    // Now test paste: press Ctrl+V
    await page.keyboard.down('Control')
    await page.keyboard.press('v')
    await page.keyboard.up('Control')
    await page.keyboard.press('Enter')

    console.log('Ctrl+V pressed')

    // Wait for the text to appear
    await page.waitForTimeout(3000)

    await page.screenshot({ path: 'e2e/screenshots/paste-after.png' })

    // Read terminal content directly from xterm's rendered rows
    const termContent = await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    })
    console.log('Terminal content:', JSON.stringify(termContent?.slice(0, 500)))

    expect(termContent).toContain('oxmux-paste-e2e-ok')
  })
})
