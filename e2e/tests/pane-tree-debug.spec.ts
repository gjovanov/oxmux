import { test, expect } from '@playwright/test'

test('debug: why mars1 pane tree is empty', async ({ page }) => {
  test.setTimeout(120_000)

  const allLogs: string[] = []
  page.on('console', msg => {
    const t = msg.text()
    allLogs.push(t)
  })

  await page.goto('https://oxmux.app')
  await page.waitForLoadState('networkidle')

  // Login
  if (!await page.locator('.session-sidebar').isVisible({ timeout: 3000 }).catch(() => false)) {
    const loginTab = page.locator('button', { hasText: 'Login' })
    if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) await loginTab.click()
    await page.locator('input[type="text"]').fill(process.env.E2E_USER || 'gjovanov2')
    await page.locator('input[type="password"]').fill(process.env.E2E_PASS || 'Gj12345!!')
    await page.locator('button[type="submit"]').click()
    await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
  }

  // Wait for sess_list
  await page.waitForTimeout(2000)

  // Log all sess_list/sess_connected messages
  const sessLogs = allLogs.filter(l => l.includes('sess_list') || l.includes('sess_connected'))
  console.log('Session logs after login:')
  sessLogs.forEach(l => console.log(' ', l.slice(0, 200)))

  // Check mars1 status in UI
  const mars1 = page.locator('.managed-session', { hasText: 'mars1' })
  const mars1Visible = await mars1.isVisible({ timeout: 3000 }).catch(() => false)
  console.log('mars1 visible:', mars1Visible)

  if (mars1Visible) {
    const statusEl = mars1.locator('.ms-status')
    const status = await statusEl.textContent()
    console.log('mars1 status:', status)

    // If disconnected, connect it
    const connectBtn = mars1.locator('.action-btn.connect')
    if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      console.log('Clicking Connect...')
      await connectBtn.click()

      // Wait and capture sess_connected response
      await page.waitForTimeout(10000)

      const newLogs = allLogs.filter(l => l.includes('sess_connected'))
      console.log('sess_connected logs after connect:')
      newLogs.forEach(l => console.log(' ', l.slice(0, 300)))

      const status2 = await statusEl.textContent()
      console.log('mars1 status after connect:', status2)
    }

    // Check pane tree
    await page.waitForTimeout(2000)
    const panes = await page.locator('.pane-node').count()
    console.log('Pane nodes visible:', panes)

    // Check sidebar HTML for mars1 tree
    const mars1Html = await mars1.innerHTML()
    console.log('mars1 sidebar HTML (200):', mars1Html.slice(0, 200))
  }

  await page.screenshot({ path: 'e2e/screenshots/pane-debug-01.png' })

  // Also try clicking Refresh
  const refreshBtn = page.locator('.managed-session', { hasText: 'mars1' }).locator('.action-btn.refresh')
  if (await refreshBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    console.log('Clicking Refresh...')
    await refreshBtn.click()
    await page.waitForTimeout(5000)

    const newLogs2 = allLogs.filter(l => l.includes('sess_connected'))
    console.log('After refresh:')
    newLogs2.slice(-3).forEach(l => console.log(' ', l.slice(0, 300)))

    const panes2 = await page.locator('.pane-node').count()
    console.log('Panes after refresh:', panes2)
  }

  await page.screenshot({ path: 'e2e/screenshots/pane-debug-02.png' })
})
