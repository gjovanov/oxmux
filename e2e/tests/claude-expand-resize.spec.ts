import { test, expect } from '@playwright/test'

const BASE_URL = 'https://oxmux.app'
const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

test('Claude Code: expand from mashed + sidebar resize', async ({ page, context }) => {
  test.setTimeout(600_000)
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])

  const logs: string[] = []
  page.on('console', msg => {
    const t = msg.text()
    if (t.includes('[oxmux') || t.includes('P2P') || t.includes('control mode'))
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

  // Connect mars1
  const mars1 = page.locator('.managed-session', { hasText: 'mars1' })
  const connectBtn = mars1.locator('.action-btn.connect')
  if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await connectBtn.click()
    await expect(mars1.locator('.ms-status.connected')).toBeVisible({ timeout: 45_000 })
  }

  // Agent + WebRTC
  const installBtn = mars1.locator('.action-btn.install')
  if (await installBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await installBtn.click()
    await expect(mars1.locator('.agent-dot.online')).toBeVisible({ timeout: 90_000 })
  }
  const webrtcBtn = mars1.locator('.action-btn.p2p', { hasText: 'WebRTC P2P' })
  if (await webrtcBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
    await webrtcBtn.click()
    for (let i = 0; i < 30; i++) {
      await page.waitForTimeout(1000)
      if (logs.some(l => l.includes('WebRTC P2P connected!'))) break
    }
  }
  console.log('Setup: mars1 + WebRTC P2P')

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
    const hasHeader = content.includes('Claude Code') || content.includes('▐▛')
    console.log(`[${label}] ${lines} lines, header=${hasHeader}`)
    await page.screenshot({ path: `e2e/screenshots/expand-${label}.png` })
    return lines
  }

  // Start in Mashed view
  await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
  await page.waitForTimeout(1000)

  // Click pane to assign to grid cell
  await page.locator('.pane-node').first().click()
  await page.waitForTimeout(2000)

  const mashedLines = await checkContent('01-mashed')

  // === TEST 1: Expand from mashed view ===
  console.log('\n=== TEST 1: Expand ===')
  const expandBtn = page.locator('.cell-btn').first() // expand arrow
  if (await expandBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await expandBtn.click()
    await page.waitForTimeout(2000)
  } else {
    // Fallback: click Single button
    await page.locator('.toggle-btn', { hasText: 'Single' }).click()
    await page.waitForTimeout(2000)
  }
  const expandLines = await checkContent('02-expand')

  // === TEST 2: Back to Mashed ===
  console.log('\n=== TEST 2: Back to Mashed ===')
  await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
  await page.waitForTimeout(2000)
  const mashedLines2 = await checkContent('03-mashed-back')

  // === TEST 3: Sidebar resize (drag left panel wider) ===
  console.log('\n=== TEST 3: Sidebar resize ===')
  await page.locator('.toggle-btn', { hasText: 'Single' }).click()
  await page.waitForTimeout(1000)
  await page.locator('.pane-node').first().click()
  await page.waitForTimeout(1000)

  // Simulate sidebar resize by dragging the resizer
  const resizer = page.locator('.sidebar-resizer')
  if (await resizer.isVisible({ timeout: 2000 }).catch(() => false)) {
    const box = await resizer.boundingBox()
    if (box) {
      // Drag right (make sidebar wider)
      await page.mouse.move(box.x + 2, box.y + box.height / 2)
      await page.mouse.down()
      await page.mouse.move(box.x + 100, box.y + box.height / 2, { steps: 5 })
      await page.mouse.up()
      await page.waitForTimeout(2000)
      await checkContent('04-sidebar-wider')

      // Drag left (make sidebar narrower)
      const box2 = await resizer.boundingBox()
      if (box2) {
        await page.mouse.move(box2.x + 2, box2.y + box2.height / 2)
        await page.mouse.down()
        await page.mouse.move(box2.x - 80, box2.y + box2.height / 2, { steps: 5 })
        await page.mouse.up()
        await page.waitForTimeout(2000)
        await checkContent('05-sidebar-narrower')
      }
    }
  }

  // === TEST 4: Multiple rapid switches ===
  console.log('\n=== TEST 4: Rapid switches ===')
  for (let i = 0; i < 3; i++) {
    await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
    await page.waitForTimeout(500)
    await page.locator('.toggle-btn', { hasText: 'Single' }).click()
    await page.waitForTimeout(500)
  }
  await page.waitForTimeout(2000)
  await checkContent('06-after-rapid')

  // === TEST 5: Ctrl+D exit + restart ===
  console.log('\n=== TEST 5: Exit + restart ===')
  await page.locator('.terminal-pane').click()
  await page.waitForTimeout(500)
  await page.keyboard.down('Control')
  await page.keyboard.press('d')
  await page.keyboard.up('Control')
  await page.waitForTimeout(2000)
  await page.keyboard.down('Control')
  await page.keyboard.press('d')
  await page.keyboard.up('Control')
  await page.waitForTimeout(3000)
  await checkContent('07-after-exit')

  // Restart
  await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
  await page.keyboard.press('Enter')
  for (let i = 0; i < 20; i++) {
    await page.waitForTimeout(2000)
    const c = await readTerm()
    if (c.includes('Claude Code') || c.includes('❯')) break
  }
  await checkContent('08-restarted')

  // P2P status
  const p2pLost = logs.filter(l => l.includes('P2P lost')).length
  const ctrlCrash = logs.filter(l => l.includes('control mode exited')).length
  console.log(`\nP2P lost: ${p2pLost}, control mode exits: ${ctrlCrash}`)
})
