import { test, expect } from '@playwright/test'

const BASE_URL = 'https://oxmux.app'
const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

test('pane tree visible after WebRTC P2P upgrade', async ({ page }) => {
  test.setTimeout(300_000)

  const logs: string[] = []
  page.on('console', msg => logs.push(msg.text()))

  // Login
  await page.goto(BASE_URL)
  await page.waitForLoadState('networkidle')
  if (!await page.locator('.session-sidebar').isVisible({ timeout: 3000 }).catch(() => false)) {
    const loginTab = page.locator('button', { hasText: 'Login' })
    if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) await loginTab.click()
    await page.locator('input[type="text"]').fill(USER)
    await page.locator('input[type="password"]').fill(PASS)
    await page.locator('button[type="submit"]').click()
    await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
  }
  console.log('1. Logged in')
  await page.screenshot({ path: 'e2e/screenshots/p2p-vis-01-loggedin.png' })

  // Connect mars1 if not connected
  const mars1 = page.locator('.managed-session', { hasText: 'mars1' })
  await expect(mars1).toBeVisible({ timeout: 5000 })

  const connectBtn = mars1.locator('.action-btn.connect')
  if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await connectBtn.click()
    await expect(mars1.locator('.ms-status.connected')).toBeVisible({ timeout: 45_000 })
  }
  console.log('2. mars1 connected')

  // Check pane tree BEFORE P2P
  await page.waitForTimeout(2000)
  const panesBefore = await page.locator('.pane-node').count()
  console.log('3. Panes before P2P:', panesBefore)
  await page.screenshot({ path: 'e2e/screenshots/p2p-vis-02-before-p2p.png' })

  // Check session trees in store
  const storeState = await page.evaluate(() => {
    // @ts-ignore
    const store = window.__pinia?.state?.value?.tmux
    if (!store) return { error: 'no store' }
    return {
      managedSessions: store.managedSessions?.length,
      connectedIds: [...(store.connectedSessionIds || [])],
      sessionTreesKeys: [...(store.sessionTrees?.keys?.() || [])],
      focusedSessionId: store.focusedSessionId,
      activePane: store.activePane,
    }
  })
  console.log('3b. Store state:', JSON.stringify(storeState))

  // Install agent if needed
  const installBtn = mars1.locator('.action-btn.install')
  if (await installBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    console.log('4. Installing agent...')
    await installBtn.click()
    await expect(mars1.locator('.agent-dot.online')).toBeVisible({ timeout: 90_000 })
  }
  console.log('4. Agent ready')

  // Click WebRTC P2P
  const webrtcBtn = mars1.locator('.action-btn.p2p', { hasText: 'WebRTC P2P' })
  if (await webrtcBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
    console.log('5. Clicking WebRTC P2P...')
    await webrtcBtn.click()

    // Wait for P2P connected
    for (let i = 0; i < 30; i++) {
      await page.waitForTimeout(1000)
      if (logs.some(l => l.includes('WebRTC P2P connected!'))) {
        console.log('5. WebRTC P2P connected!')
        break
      }
    }
  }

  await page.waitForTimeout(3000)
  await page.screenshot({ path: 'e2e/screenshots/p2p-vis-03-after-p2p.png' })

  // Check pane tree AFTER P2P
  const panesAfter = await page.locator('.pane-node').count()
  console.log('6. Panes after P2P:', panesAfter)

  // Check store state after P2P
  const storeAfter = await page.evaluate(() => {
    // @ts-ignore
    const store = window.__pinia?.state?.value?.tmux
    if (!store) return { error: 'no store' }
    return {
      managedSessions: store.managedSessions?.map((s: any) => ({
        id: s.id?.slice(0, 8),
        name: s.name,
        status: s.status,
        hasTmuxSessions: (s.tmux_sessions?.length || 0) > 0,
      })),
      connectedIds: [...(store.connectedSessionIds || [])].map((s: string) => s.slice(0, 8)),
      sessionTreesKeys: [...(store.sessionTrees?.keys?.() || [])].map((s: string) => s.slice(0, 8)),
      focusedSessionId: store.focusedSessionId?.slice(0, 8),
      activePane: store.activePane,
    }
  })
  console.log('6b. Store after P2P:', JSON.stringify(storeAfter))

  // Check relevant console logs
  const p2pLogs = logs.filter(l =>
    l.includes('[oxmux]') && (l.includes('sess_connected') || l.includes('P2P') || l.includes('transport'))
  )
  console.log('7. Relevant logs:')
  p2pLogs.slice(-10).forEach(l => console.log('  ', l))

  // Assert pane tree is still visible
  expect(panesAfter, 'Panes should be visible after P2P upgrade').toBeGreaterThan(0)

  await page.screenshot({ path: 'e2e/screenshots/p2p-vis-04-final.png' })
})
