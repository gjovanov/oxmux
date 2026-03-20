import { test, expect, Page } from '@playwright/test'

const BASE_URL = process.env.BASE_URL || 'https://oxmux.app'
const TEST_USER = 'gjovanov'
const TEST_PASS = 'test1234'
const SSH_HOST = '94.130.141.98'
const SSH_USER = 'gjovanov'
const SSH_KEY = '~/.ssh/id_secunet'

async function authenticate(page: Page) {
  await page.goto(BASE_URL)
  await page.waitForLoadState('networkidle')

  if (await page.locator('.session-sidebar').isVisible({ timeout: 3000 }).catch(() => false)) {
    return
  }

  const loginTab = page.locator('button', { hasText: 'Login' })
  if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) {
    await loginTab.click()
  }

  await page.locator('input[type="text"]').fill(TEST_USER)
  await page.locator('input[type="password"]').fill(TEST_PASS)
  await page.locator('button[type="submit"]').click()
  await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
}

async function ensureConnectedSession(page: Page) {
  // Check if we already have a connected session with panes
  if (await page.locator('.pane-node').first().isVisible({ timeout: 2000 }).catch(() => false)) {
    return
  }

  // Check if there's an existing session to connect
  const connectBtn = page.locator('.action-btn.connect').first()
  if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await connectBtn.click()
    await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 30_000 })
    return
  }

  // No sessions — create one
  await page.locator('.add-btn').click()
  await expect(page.locator('.dialog')).toBeVisible({ timeout: 5000 })

  await page.locator('input[placeholder="my-project"]').fill('paste-test-' + Date.now())
  await page.locator('select').first().selectOption('ssh')
  await page.locator('input[placeholder="94.130.141.98"]').fill(SSH_HOST)
  await page.locator('input[placeholder="ubuntu"]').fill(SSH_USER)
  await page.locator('select').nth(1).selectOption('private_key')
  await page.locator('input[placeholder="~/.ssh/id_ed25519"]').fill(SSH_KEY)
  await page.locator('button', { hasText: 'Create' }).click()

  // Wait for session to appear and connect it
  await page.waitForTimeout(1000)
  const newConnectBtn = page.locator('.action-btn.connect').first()
  await expect(newConnectBtn).toBeVisible({ timeout: 5000 })
  await newConnectBtn.click()

  // Wait for panes to appear (SSH connection + tmux setup)
  await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 45_000 })
}

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
