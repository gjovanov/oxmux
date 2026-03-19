import { test, expect, Page } from '@playwright/test'

const BASE_URL = process.env.BASE_URL || 'https://oxmux.app'
const TEST_USER = 'gjovanov'
const TEST_PASS = 'test1234'

async function authenticate(page: Page) {
  await page.goto(BASE_URL)
  await page.waitForLoadState('networkidle')

  if (await page.locator('.session-sidebar').isVisible({ timeout: 2000 }).catch(() => false)) {
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

test.describe('Mashed View', () => {
  test.beforeEach(async ({ page }) => {
    await authenticate(page)
  })

  test('view toggle appears when session connected', async ({ page }) => {
    // Connect first available session
    const connectBtn = page.locator('.action-btn.connect').first()
    if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await connectBtn.click()
    }

    // Wait for connection
    await expect(page.locator('.ms-status.connected').first()).toBeVisible({ timeout: 30_000 })

    // View toggle should appear
    await expect(page.locator('.view-toggle')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.toggle-btn', { hasText: 'Single' })).toBeVisible()
    await expect(page.locator('.toggle-btn', { hasText: 'Mashed' })).toBeVisible()

    await page.screenshot({ path: 'e2e/screenshots/view-toggle.png' })
  })

  test('switching to mashed view shows grid', async ({ page }) => {
    // Connect session
    const connectBtn = page.locator('.action-btn.connect').first()
    if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await connectBtn.click()
    }
    await expect(page.locator('.ms-status.connected').first()).toBeVisible({ timeout: 30_000 })

    // Switch to mashed view
    await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()

    // Grid should appear
    await expect(page.locator('.mashed-view')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.mashed-toolbar')).toBeVisible()
    await expect(page.locator('.mashed-grid')).toBeVisible()

    await page.screenshot({ path: 'e2e/screenshots/mashed-view-grid.png' })
  })

  test('grid size buttons change layout', async ({ page }) => {
    const connectBtn = page.locator('.action-btn.connect').first()
    if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await connectBtn.click()
    }
    await expect(page.locator('.ms-status.connected').first()).toBeVisible({ timeout: 30_000 })

    await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
    await expect(page.locator('.mashed-grid')).toBeVisible({ timeout: 5_000 })

    // Default 2x2
    const grid = page.locator('.mashed-grid')
    await expect(grid).toHaveCSS('grid-template-columns', /1fr 1fr/)

    // Switch to 3x3
    await page.locator('.grid-btn', { hasText: '3x3' }).click()
    await expect(grid).toHaveCSS('grid-template-columns', /1fr 1fr 1fr/)

    await page.screenshot({ path: 'e2e/screenshots/mashed-view-3x3.png' })
  })

  test('mashed cell shows terminal with header', async ({ page }) => {
    const connectBtn = page.locator('.action-btn.connect').first()
    if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await connectBtn.click()
    }
    await expect(page.locator('.ms-status.connected').first()).toBeVisible({ timeout: 30_000 })

    await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
    await expect(page.locator('.mashed-cell').first()).toBeVisible({ timeout: 10_000 })

    // Cell header should show session info
    const cell = page.locator('.mashed-cell').first()
    await expect(cell.locator('.cell-header')).toBeVisible()
    await expect(cell.locator('.cell-session')).toBeVisible()
    await expect(cell.locator('.cell-transport')).toBeVisible()

    // Terminal should be present
    await expect(cell.locator('.xterm-container')).toBeVisible()

    await page.screenshot({ path: 'e2e/screenshots/mashed-cell.png' })
  })

  test('switching back to single view preserves active pane', async ({ page }) => {
    const connectBtn = page.locator('.action-btn.connect').first()
    if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await connectBtn.click()
    }
    await expect(page.locator('.ms-status.connected').first()).toBeVisible({ timeout: 30_000 })

    // Go to mashed view
    await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
    await expect(page.locator('.mashed-cell').first()).toBeVisible({ timeout: 10_000 })

    // Click a cell to focus it
    await page.locator('.mashed-cell').first().click()

    // Switch back to single view
    await page.locator('.toggle-btn', { hasText: 'Single' }).click()

    // Should show a terminal pane (not empty state)
    await expect(page.locator('.terminal-pane')).toBeVisible({ timeout: 5_000 })

    await page.screenshot({ path: 'e2e/screenshots/single-after-mashed.png' })
  })
})
