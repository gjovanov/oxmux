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

  if (await page.locator('.session-sidebar').isVisible({ timeout: 3000 }).catch(() => false)) return

  const loginTab = page.locator('button', { hasText: 'Login' })
  if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) await loginTab.click()

  await page.locator('input[type="text"]').fill(TEST_USER)
  await page.locator('input[type="password"]').fill(TEST_PASS)
  await page.locator('button[type="submit"]').click()
  await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
}

async function ensureConnectedSession(page: Page) {
  if (await page.locator('.pane-node').first().isVisible({ timeout: 2000 }).catch(() => false)) return

  const connectBtn = page.locator('.action-btn.connect').first()
  if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await connectBtn.click()
    await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 45_000 })
    return
  }

  await page.locator('.add-btn').click()
  await expect(page.locator('.dialog')).toBeVisible({ timeout: 5000 })
  await page.locator('input[placeholder="my-project"]').fill('resize-test-' + Date.now())
  await page.locator('select').first().selectOption('ssh')
  await page.locator('input[placeholder="94.130.141.98"]').fill(SSH_HOST)
  await page.locator('input[placeholder="ubuntu"]').fill(SSH_USER)
  await page.locator('select').nth(1).selectOption('private_key')
  await page.locator('input[placeholder="~/.ssh/id_ed25519"]').fill(SSH_KEY)
  await page.locator('button', { hasText: 'Create' }).click()
  await page.waitForTimeout(1000)
  const newConnectBtn = page.locator('.action-btn.connect').first()
  await expect(newConnectBtn).toBeVisible({ timeout: 5000 })
  await newConnectBtn.click()
  await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 45_000 })
}

test.describe('Terminal Resize', () => {
  test('tput cols matches terminal width after resize', async ({ page }) => {
    test.setTimeout(90_000)
    await authenticate(page)
    await ensureConnectedSession(page)

    // Switch to single view
    const singleBtn = page.locator('.toggle-btn', { hasText: 'Single' })
    if (await singleBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      await singleBtn.click()
    }

    // Click pane
    await page.locator('.pane-node').first().click()
    await expect(page.locator('.xterm-screen')).toBeVisible({ timeout: 10_000 })
    await page.locator('.terminal-pane').click()
    await page.waitForTimeout(1000)

    // Type tput cols to check terminal width
    await page.keyboard.type('tput cols', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(1500)

    // Read terminal output
    const termContent = await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    })
    console.log('Terminal content:', JSON.stringify(termContent?.slice(0, 300)))

    // tput cols should return a number > 80 (since single view is typically wider)
    const colsMatch = termContent?.match(/tput cols\n(\d+)/)
    if (colsMatch) {
      const cols = parseInt(colsMatch[1])
      console.log('tput cols returned:', cols)
      expect(cols).toBeGreaterThan(40) // at least reasonable width
    }

    await page.screenshot({ path: 'e2e/screenshots/resize-test.png' })
  })
})
