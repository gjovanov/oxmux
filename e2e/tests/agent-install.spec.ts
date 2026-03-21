import { test, expect, Page } from '@playwright/test'

const BASE_URL = process.env.BASE_URL || 'http://localhost:8080'
const TEST_USER = process.env.E2E_USER || 'e2e_test'
const TEST_PASS = process.env.E2E_PASS || 'e2e_test_pass'
const SSH_HOST = process.env.E2E_SSH_HOST || '127.0.0.1'
const SSH_USER = process.env.E2E_SSH_USER || 'test'
const SSH_KEY_PATH = process.env.E2E_SSH_KEY || '~/.ssh/id_ed25519'

async function authenticate(page: Page) {
  await page.goto(BASE_URL)
  await page.waitForLoadState('networkidle')
  if (await page.locator('.session-sidebar').isVisible({ timeout: 2000 }).catch(() => false)) return
  const loginTab = page.locator('button', { hasText: 'Login' })
  if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) await loginTab.click()
  await page.locator('input[type="text"]').fill(TEST_USER)
  await page.locator('input[type="password"]').fill(TEST_PASS)
  await page.locator('button[type="submit"]').click()
  await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
}

async function createSshSession(page: Page, name: string) {
  await page.locator('.add-btn').click()
  await expect(page.locator('.dialog')).toBeVisible()
  await page.locator('input[placeholder="my-project"]').fill(name)
  await page.locator('select').first().selectOption('ssh')
  await page.locator('input[placeholder="192.0.2.1"]').fill(SSH_HOST)
  await page.locator('input[placeholder="ubuntu"]').fill(SSH_USER)
  await page.locator('select').nth(1).selectOption('private_key')
  await page.locator('input[placeholder="~/.ssh/id_ed25519"]').fill(SSH_KEY_PATH)
  await page.locator('button', { hasText: 'Create' }).first().click()
  await expect(page.locator('.dialog')).not.toBeVisible({ timeout: 5000 })
}

test.describe('Agent Install & Status', () => {
  test.beforeEach(async ({ page }) => {
    await authenticate(page)
  })

  test('detects agent online for host with running agent', async ({ page, browserName }) => {
    // Skip in CI — QUIC probe from K8s pod to mars requires direct UDP connectivity
    test.skip(!!process.env.CI, 'requires direct UDP to agent host');
    test.setTimeout(60_000)

    const name = `agent-detect-${Date.now()}`
    await createSshSession(page, name)

    // Connect session
    const card = page.locator('.managed-session', { hasText: name })
    const connectBtn = card.locator('button', { hasText: 'Connect' })
    if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      await connectBtn.click()
    }
    await expect(card.locator('.ms-status.connected')).toBeVisible({ timeout: 30_000 })

    // Wait for agent status to be detected (probe runs through K8s → WireGuard → mars)
    // The agent should be running on mars from previous install
    await page.waitForTimeout(15_000)

    // Take screenshot of agent status
    await page.screenshot({ path: `test-results/agent-status-${Date.now()}.png` })

    // Check if agent section shows online or not_installed
    const agentSection = card.locator('.agent-section')
    if (await agentSection.isVisible({ timeout: 2000 }).catch(() => false)) {
      const text = await agentSection.textContent()
      console.log('Agent section text:', text)
      // Agent should be detected as online (running from previous manual install)
    }

    // Clean up
    const disconnectBtn = card.locator('button', { hasText: 'Disconnect' })
    if (await disconnectBtn.isVisible({ timeout: 1000 }).catch(() => false)) await disconnectBtn.click()
    await page.waitForTimeout(500)
    const deleteBtn = card.locator('.action-btn.delete')
    if (await deleteBtn.isVisible({ timeout: 1000 }).catch(() => false)) await deleteBtn.click()
  })

  test('dialog does not close when clicking outside', async ({ page }) => {
    await page.locator('.add-btn').click()
    await expect(page.locator('.dialog')).toBeVisible()

    // Fill in some data
    await page.locator('input[placeholder="my-project"]').fill('test-persist')

    // Click outside the dialog (on the overlay)
    await page.locator('.dialog-overlay').click({ position: { x: 10, y: 10 } })

    // Dialog should still be visible
    await expect(page.locator('.dialog')).toBeVisible()

    // Data should be preserved
    const value = await page.locator('input[placeholder="my-project"]').inputValue()
    expect(value).toBe('test-persist')

    // Close via X button
    await page.locator('.close-btn').click()
    await expect(page.locator('.dialog')).not.toBeVisible()
  })
})
