import { test, expect } from '@playwright/test'

const BASE_URL = 'https://oxmux.app'
const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

test('Claude Code view switching (single ↔ mashed) x3', async ({ page, context }) => {
  test.setTimeout(600_000)
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])

  const logs: string[] = []
  page.on('console', msg => {
    const t = msg.text()
    if (t.includes('[oxmux') || t.includes('xterm') || t.includes('P2P'))
      logs.push(t)
  })

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

  // Connect mars1 FIRST (toggle only shows when sessions connected)
  const mars1 = page.locator('.managed-session', { hasText: 'mars1' })
  const connectBtn = mars1.locator('.action-btn.connect')
  if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await connectBtn.click()
    await expect(mars1.locator('.ms-status.connected')).toBeVisible({ timeout: 45_000 })
  }

  // Install agent if needed
  const installBtn = mars1.locator('.action-btn.install')
  if (await installBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await installBtn.click()
    await expect(mars1.locator('.agent-dot.online')).toBeVisible({ timeout: 90_000 })
  }

  // WebRTC P2P
  const webrtcBtn = mars1.locator('.action-btn.p2p', { hasText: 'WebRTC P2P' })
  if (await webrtcBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
    await webrtcBtn.click()
    for (let i = 0; i < 30; i++) {
      await page.waitForTimeout(1000)
      if (logs.some(l => l.includes('WebRTC P2P connected!'))) break
    }
  }
  console.log('Setup complete: mars1 + WebRTC P2P')

  // Switch to Single view
  const singleBtn = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await singleBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await singleBtn.click()
    await page.waitForTimeout(500)
  }

  // Select pane
  await page.locator('.pane-node').first().click()
  await page.waitForTimeout(1000)
  await expect(page.locator('.xterm-screen')).toBeVisible({ timeout: 15_000 })
  await page.locator('.terminal-pane').click()
  await page.waitForTimeout(2000)

  async function readTerm(): Promise<string> {
    return await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    }) || ''
  }

  async function checkContent(label: string) {
    await page.waitForTimeout(2000)
    const content = await readTerm()
    const lines = content.split('\n').filter(l => l.trim()).length
    const hasPrompt = content.includes('❯')
    const hasSep = content.includes('───')
    const hasHeader = content.includes('Claude Code') || content.includes('▐▛')
    console.log(`[${label}] lines=${lines} prompt=${hasPrompt} sep=${hasSep} header=${hasHeader}`)
    console.log(`[${label}] first 150: ${content.replace(/\n+/g, ' | ').slice(0, 150)}`)
    await page.screenshot({ path: `e2e/screenshots/viewswitch-${label}.png` })
    return { lines, hasPrompt, hasSep, hasHeader }
  }

  // === Claude Code lifecycle + view switching ===
  for (let cycle = 1; cycle <= 3; cycle++) {
    console.log(`\n=== CYCLE ${cycle} ===`)

    // Start Claude Code (if not running)
    let content = await readTerm()
    if (!content.includes('❯') && !content.includes('Claude')) {
      console.log(`${cycle}. Starting Claude Code...`)
      await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
      await page.keyboard.press('Enter')
      for (let i = 0; i < 20; i++) {
        await page.waitForTimeout(2000)
        content = await readTerm()
        if (content.includes('❯') || content.includes('Claude Code')) break
      }
    }

    await checkContent(`cycle${cycle}-single-claude`)

    // Switch to Mashed
    console.log(`${cycle}. Switching to Mashed...`)
    await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
    await page.waitForTimeout(2000)
    await checkContent(`cycle${cycle}-mashed`)

    // Switch back to Single
    console.log(`${cycle}. Switching to Single...`)
    await page.locator('.toggle-btn', { hasText: 'Single' }).click()
    await page.waitForTimeout(500)
    // Re-select pane (might lose selection on view switch)
    await page.locator('.pane-node').first().click()
    await page.waitForTimeout(1000)
    await checkContent(`cycle${cycle}-single-back`)

    // Exit Claude
    console.log(`${cycle}. Exiting Claude (Ctrl+D x2)...`)
    await page.keyboard.down('Control')
    await page.keyboard.press('d')
    await page.keyboard.up('Control')
    await page.waitForTimeout(2000)
    await page.keyboard.down('Control')
    await page.keyboard.press('d')
    await page.keyboard.up('Control')
    await page.waitForTimeout(3000)
    await checkContent(`cycle${cycle}-after-exit`)

    console.log(`${cycle}. P2P lost events: ${logs.filter(l => l.includes('P2P lost')).length}`)
  }

  // Dump P2P events
  console.log('\n=== P2P LIFECYCLE ===')
  logs.filter(l => l.includes('P2P') || l.includes('DataChannel')).forEach(l => console.log(l))
})
