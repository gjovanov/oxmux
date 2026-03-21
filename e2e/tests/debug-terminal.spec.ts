import { test, expect } from '@playwright/test'

const BASE_URL = process.env.BASE_URL || 'http://localhost:8080'
const TEST_USER = process.env.E2E_USER || 'e2e_test'
const TEST_PASS = process.env.E2E_PASS || 'e2e_test_pass'
const SSH_HOST = process.env.E2E_SSH_HOST || '127.0.0.1'
const SSH_KEY = process.env.E2E_SSH_KEY || '~/.ssh/id_ed25519'

test('debug terminal output flow', async ({ page }) => {
  const consoleLogs: string[] = []
  page.on('console', msg => {
    if (msg.text().includes('[oxmux]')) {
      consoleLogs.push(`${msg.type()}: ${msg.text()}`)
    }
  })

  // Login
  await page.goto(BASE_URL)
  await page.waitForLoadState('networkidle')
  const loginTab = page.locator('button', { hasText: 'Login' })
  if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) {
    await loginTab.click()
  }
  await page.locator('input[type="text"]').fill(TEST_USER)
  await page.locator('input[type="password"]').fill(TEST_PASS)
  await page.locator('button[type="submit"]').click()
  await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })

  console.log('=== After login ===')
  consoleLogs.forEach(l => console.log(' ', l))
  consoleLogs.length = 0

  // Create session
  const name = `debug-${Date.now()}`
  await page.locator('.add-btn').click()
  await page.locator('input[placeholder="my-project"]').fill(name)
  await page.locator('select').first().selectOption('ssh')
  await page.locator('input[placeholder="192.0.2.1"]').fill(SSH_HOST)
  await page.locator('input[placeholder="ubuntu"]').fill(SSH_USER)
  await page.locator('select').nth(1).selectOption('private_key')
  await page.locator('input[placeholder="~/.ssh/id_ed25519"]').fill(SSH_KEY)
  await page.locator('button', { hasText: 'Create' }).first().click()
  await page.waitForTimeout(1000)

  console.log('=== After create ===')
  consoleLogs.forEach(l => console.log(' ', l))
  consoleLogs.length = 0

  // Connect
  const card = page.locator('.managed-session', { hasText: name })
  const connectBtn = card.locator('button', { hasText: 'Connect' })
  if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await connectBtn.click()
  }
  await expect(card.locator('.ms-status.connected')).toBeVisible({ timeout: 30_000 })
  await page.waitForTimeout(1000)

  console.log('=== After connect ===')
  consoleLogs.forEach(l => console.log(' ', l))
  consoleLogs.length = 0

  // Click first pane
  const pane = page.locator('.pane-node').first()
  await expect(pane).toBeVisible({ timeout: 5000 })
  const paneText = await pane.textContent()
  console.log('=== Clicking pane:', paneText, '===')
  await pane.click()
  await page.waitForTimeout(2000)

  console.log('=== After pane click ===')
  consoleLogs.forEach(l => console.log(' ', l))
  consoleLogs.length = 0

  // Focus terminal and type
  await page.locator('.terminal-pane').click()
  await page.waitForTimeout(500)
  await page.keyboard.type('echo HELLO_OXMUX', { delay: 30 })
  await page.keyboard.press('Enter')
  await page.waitForTimeout(3000)

  console.log('=== After typing ===')
  consoleLogs.forEach(l => console.log(' ', l))
  consoleLogs.length = 0

  // Check accessible buffer
  const buffer = await page.locator('[data-testid="terminal-accessible-output"]').textContent().catch(() => '')
  console.log('=== Accessible buffer ===')
  console.log(buffer?.slice(-300) || '(empty)')

  // Screenshot
  await page.screenshot({ path: `test-results/debug-terminal-${Date.now()}.png` })

  // Cleanup
  const disconnectBtn = card.locator('button', { hasText: 'Disconnect' })
  if (await disconnectBtn.isVisible({ timeout: 1000 }).catch(() => false)) {
    await disconnectBtn.click()
    await page.waitForTimeout(1000)
  }
  const deleteBtn = card.locator('.action-btn.delete')
  if (await deleteBtn.isVisible({ timeout: 1000 }).catch(() => false)) {
    await deleteBtn.click()
  }
})
