import { test, expect } from '@playwright/test'

const BASE_URL = 'https://oxmux.app'
const USER = 'gjovanov2'
const PASS = 'Gj12345!!'

test('full Claude Code reconnect cycle x3', async ({ page, context }) => {
  test.setTimeout(600_000) // 10 min — agent install + 3 cycles
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])

  const logs: string[] = []
  page.on('console', msg => {
    const t = msg.text()
    if (t.includes('[oxmux') || t.includes('xterm') || t.includes('DataChannel') || t.includes('P2P'))
      logs.push(`[${Date.now()}] ${t}`)
  })

  // Step 1: Login
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

  // Step 2: Switch to Single view
  const singleBtn = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await singleBtn.isVisible({ timeout: 2000 }).catch(() => false)) await singleBtn.click()
  await page.waitForTimeout(500)

  // Step 3: Find mars1 session and connect it
  const mars1Card = page.locator('.managed-session', { hasText: 'mars1' })
  await expect(mars1Card).toBeVisible({ timeout: 5000 })

  // Connect if not already connected
  const mars1Connect = mars1Card.locator('.action-btn.connect')
  if (await mars1Connect.isVisible({ timeout: 2000 }).catch(() => false)) {
    await mars1Connect.click()
    console.log('2. Connecting mars1...')
    await expect(mars1Card.locator('.ms-status.connected')).toBeVisible({ timeout: 45_000 })
  }
  console.log('2. mars1 connected')
  await page.screenshot({ path: 'e2e/screenshots/full-01-connected.png' })

  // Step 4: Install agent if needed
  const installBtn = mars1Card.locator('.action-btn.install')
  if (await installBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    console.log('3. Installing agent...')
    await installBtn.click()
    await expect(mars1Card.locator('.agent-dot.online')).toBeVisible({ timeout: 90_000 })
    console.log('3. Agent online')
  } else {
    console.log('3. Agent already available')
  }
  await page.screenshot({ path: 'e2e/screenshots/full-02-agent.png' })

  // Step 5: Upgrade to WebRTC P2P
  const webrtcBtn = mars1Card.locator('.action-btn.p2p', { hasText: 'WebRTC P2P' })
  if (await webrtcBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
    console.log('4. Upgrading to WebRTC P2P...')
    await webrtcBtn.click()
    // Wait for P2P connected
    for (let i = 0; i < 30; i++) {
      await page.waitForTimeout(1000)
      if (logs.some(l => l.includes('WebRTC P2P connected!'))) {
        console.log('4. WebRTC P2P connected')
        break
      }
    }
  } else {
    console.log('4. Already on P2P or no button')
  }
  await page.screenshot({ path: 'e2e/screenshots/full-03-p2p.png' })

  // Step 6: Click the pane in sidebar
  // The pane node is inside the mars1 session card's tmux tree
  const paneNode = page.locator('.pane-node').first()
  if (!await paneNode.isVisible({ timeout: 5000 }).catch(() => false)) {
    // If no pane visible, the tmux tree might need a refresh
    console.log('5. No pane node visible, refreshing session...')
    const refreshBtn = mars1Card.locator('.action-btn.refresh')
    if (await refreshBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      await refreshBtn.click()
      await page.waitForTimeout(3000)
    }
  }
  await expect(paneNode).toBeVisible({ timeout: 10_000 })
  await paneNode.click()
  await page.waitForTimeout(1000)

  // Wait for terminal (might take a moment in single view)
  for (let i = 0; i < 10; i++) {
    if (await page.locator('.xterm-screen').isVisible({ timeout: 1000 }).catch(() => false)) break
    await page.waitForTimeout(500)
  }
  await expect(page.locator('.xterm-screen')).toBeVisible({ timeout: 15_000 })
  await page.locator('.terminal-pane').click()
  await page.waitForTimeout(2000)

  console.log('5. Terminal visible')
  await page.screenshot({ path: 'e2e/screenshots/full-04-terminal.png' })

  // Read terminal
  async function readTerm(): Promise<string> {
    return await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    }) || ''
  }

  let content = await readTerm()
  console.log('5. Initial content:', content.replace(/\n+/g, ' | ').slice(0, 200))

  // === CYCLE x3: exit Claude, restart Claude ===
  for (let cycle = 1; cycle <= 3; cycle++) {
    console.log(`\n=== CYCLE ${cycle} ===`)

    // Check if Claude is already running (look for ❯ prompt)
    content = await readTerm()
    const claudeRunning = content.includes('❯') || content.includes('Claude')

    if (claudeRunning) {
      // Exit Claude with Ctrl+D twice
      console.log(`${cycle}a. Exiting Claude (Ctrl+D x2)...`)
      await page.keyboard.down('Control')
      await page.keyboard.press('d')
      await page.keyboard.up('Control')
      await page.waitForTimeout(2000)
      await page.keyboard.down('Control')
      await page.keyboard.press('d')
      await page.keyboard.up('Control')
      await page.waitForTimeout(3000)

      await page.screenshot({ path: `e2e/screenshots/full-cycle${cycle}-01-afterexit.png` })
      content = await readTerm()
      console.log(`${cycle}a. After exit:`, content.replace(/\n+/g, ' | ').slice(-200))

      // Check P2P status
      const p2pLost = logs.filter(l => l.includes('P2P lost')).length
      console.log(`${cycle}a. P2P lost events so far:`, p2pLost)
    }

    // Start Claude Code
    console.log(`${cycle}b. Starting Claude Code...`)
    await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
    await page.keyboard.press('Enter')

    // Wait for Claude to render
    let claudeDetected = false
    for (let i = 0; i < 20; i++) {
      await page.waitForTimeout(2000)
      content = await readTerm()
      if (content.includes('❯') || content.includes('plan')) {
        console.log(`${cycle}b. Claude detected after ${(i+1)*2}s`)
        claudeDetected = true
        break
      }
    }

    await page.screenshot({ path: `e2e/screenshots/full-cycle${cycle}-02-claude.png` })
    content = await readTerm()
    const nonEmpty = content.split('\n').filter(l => l.trim()).length
    console.log(`${cycle}b. Content lines: ${nonEmpty}, detected: ${claudeDetected}`)
    console.log(`${cycle}b. Content (200):`, content.replace(/\n+/g, ' | ').slice(0, 200))

    // Check if it looks correct: should have ❯ prompt and proper formatting
    const hasPrompt = content.includes('❯')
    const hasSeparator = content.includes('───')
    console.log(`${cycle}b. Has ❯: ${hasPrompt}, Has ───: ${hasSeparator}`)

    // Wait a moment then check again (for delayed renders)
    await page.waitForTimeout(3000)
    await page.screenshot({ path: `e2e/screenshots/full-cycle${cycle}-03-settled.png` })
    content = await readTerm()
    const nonEmpty2 = content.split('\n').filter(l => l.trim()).length
    console.log(`${cycle}b. After settle: ${nonEmpty2} lines`)
  }

  // Dump P2P events
  console.log('\n=== P2P EVENTS ===')
  logs.filter(l => l.includes('P2P') || l.includes('DataChannel') || l.includes('control mode') || l.includes('SIGWINCH')).forEach(l => console.log(l))

  await page.screenshot({ path: 'e2e/screenshots/full-final.png' })
})
