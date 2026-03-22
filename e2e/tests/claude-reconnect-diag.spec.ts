import { test, expect } from '@playwright/test'

const BASE_URL = process.env.BASE_URL || 'https://oxmux.app'
const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

// Note: run with: BASE_URL=https://oxmux.app E2E_USER=gjovanov2 E2E_PASS='Gj12345!!' npx playwright test ...

test('diagnose Claude Code reconnect behavior', async ({ page, context }) => {
  test.setTimeout(180_000)
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])

  // Collect ALL console logs
  const logs: string[] = []
  page.on('console', msg => logs.push(`[${msg.type()}] ${msg.text()}`))

  // 1. Login
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

  await page.screenshot({ path: 'e2e/screenshots/claude-diag-01-loggedin.png' })
  console.log('Step 1: Logged in')

  // 2. Connect first session
  const connectBtn = page.locator('.action-btn.connect').first()
  if (await connectBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await connectBtn.click()
  }
  await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 30_000 })
  await page.screenshot({ path: 'e2e/screenshots/claude-diag-02-connected.png' })
  console.log('Step 2: Session connected')

  // 3. Switch to single view and select pane
  await page.locator('.toggle-btn', { hasText: 'Single' }).click()
  await page.waitForTimeout(500)
  await page.locator('.pane-node').first().click()
  await page.waitForTimeout(1000)
  // Wait for terminal to appear (might take a moment after pane selection)
  await expect(page.locator('.xterm-screen')).toBeVisible({ timeout: 15_000 })
  await page.locator('.terminal-pane').click()
  await page.waitForTimeout(2000)

  await page.screenshot({ path: 'e2e/screenshots/claude-diag-03-terminal.png' })

  // 4. Read initial terminal content
  let content = await page.evaluate(() => {
    const rows = document.querySelectorAll('.xterm-rows > div')
    return Array.from(rows).map(r => r.textContent || '').join('\n')
  })
  console.log('Step 3: Initial terminal content (first 300 chars):')
  console.log(content.slice(0, 300))

  // 5. Check if WebRTC P2P is available and upgrade
  const webrtcBtn = page.locator('.action-btn.p2p', { hasText: 'WebRTC P2P' })
  if (await webrtcBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    console.log('Step 4: Upgrading to WebRTC P2P...')
    await webrtcBtn.click()

    // Wait for P2P connected
    for (let i = 0; i < 30; i++) {
      await page.waitForTimeout(1000)
      if (logs.some(l => l.includes('WebRTC P2P connected!'))) {
        console.log('Step 4: WebRTC P2P connected!')
        break
      }
    }
  } else {
    console.log('Step 4: No WebRTC button, checking agent status...')
    // Try installing agent
    const installBtn = page.locator('.action-btn.install')
    if (await installBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      console.log('Installing agent...')
      await installBtn.click()
      await page.waitForTimeout(60_000) // wait for install
    }
  }

  await page.waitForTimeout(3000)
  await page.screenshot({ path: 'e2e/screenshots/claude-diag-04-afterp2p.png' })

  content = await page.evaluate(() => {
    const rows = document.querySelectorAll('.xterm-rows > div')
    return Array.from(rows).map(r => r.textContent || '').join('\n')
  })
  console.log('Step 5: Terminal after P2P (first 300):')
  console.log(content.slice(0, 300))

  // 6. Check terminal dimensions
  await page.keyboard.type('tput cols; tput lines', { delay: 30 })
  await page.keyboard.press('Enter')
  await page.waitForTimeout(2000)

  content = await page.evaluate(() => {
    const rows = document.querySelectorAll('.xterm-rows > div')
    return Array.from(rows).map(r => r.textContent || '').join('\n')
  })
  console.log('Step 6: After tput cols:')
  const colsMatch = content.match(/tput cols.*\n(\d+)/)
  const linesMatch = content.match(/tput lines\n(\d+)/)
  console.log('cols:', colsMatch?.[1], 'lines:', linesMatch?.[1])

  await page.screenshot({ path: 'e2e/screenshots/claude-diag-05-tputcols.png' })

  // 7. Start Claude Code
  console.log('Step 7: Starting Claude Code...')
  await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
  await page.keyboard.press('Enter')

  // Wait for Claude Code to start
  for (let i = 0; i < 15; i++) {
    await page.waitForTimeout(2000)
    content = await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    })
    if (content.includes('❯') || content.includes('Claude')) {
      console.log(`Step 7: Claude Code detected after ${(i+1)*2}s`)
      break
    }
  }

  await page.screenshot({ path: 'e2e/screenshots/claude-diag-06-claude-started.png' })
  console.log('Terminal with Claude:')
  console.log(content.slice(0, 500))

  // 8. Exit Claude Code with Ctrl+D twice
  console.log('Step 8: Exiting Claude Code (Ctrl+D x2)...')
  await page.keyboard.down('Control')
  await page.keyboard.press('d')
  await page.keyboard.up('Control')
  await page.waitForTimeout(2000)
  await page.keyboard.down('Control')
  await page.keyboard.press('d')
  await page.keyboard.up('Control')
  await page.waitForTimeout(3000)

  await page.screenshot({ path: 'e2e/screenshots/claude-diag-07-after-exit.png' })
  content = await page.evaluate(() => {
    const rows = document.querySelectorAll('.xterm-rows > div')
    return Array.from(rows).map(r => r.textContent || '').join('\n')
  })
  console.log('Step 8: After Ctrl+D exit:')
  console.log(content.slice(0, 300))

  // Check if P2P is still alive
  const p2pLost = logs.some(l => l.includes('P2P lost'))
  console.log('P2P lost after Ctrl+D:', p2pLost)

  // 9. Start Claude Code again
  console.log('Step 9: Starting Claude Code again...')
  await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
  await page.keyboard.press('Enter')

  for (let i = 0; i < 15; i++) {
    await page.waitForTimeout(2000)
    content = await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    })
    if (content.includes('❯') || content.includes('Claude')) {
      console.log(`Step 9: Claude Code restarted after ${(i+1)*2}s`)
      break
    }
  }

  await page.screenshot({ path: 'e2e/screenshots/claude-diag-08-claude-restarted.png' })
  console.log('Terminal after restart:')
  console.log(content.slice(0, 500))

  // 10. Try typing in Claude Code
  console.log('Step 10: Typing in Claude Code...')
  await page.keyboard.type('hello test', { delay: 50 })
  await page.waitForTimeout(2000)

  await page.screenshot({ path: 'e2e/screenshots/claude-diag-09-typing.png' })
  content = await page.evaluate(() => {
    const rows = document.querySelectorAll('.xterm-rows > div')
    return Array.from(rows).map(r => r.textContent || '').join('\n')
  })
  console.log('After typing:')
  console.log(content.slice(0, 500))

  // 11. Dump relevant console logs
  console.log('\n=== RELEVANT CONSOLE LOGS ===')
  logs.filter(l =>
    l.includes('[oxmux') ||
    l.includes('DataChannel') ||
    l.includes('P2P') ||
    l.includes('resize') ||
    l.includes('xterm')
  ).forEach(l => console.log(l))

  // Final screenshot
  await page.screenshot({ path: 'e2e/screenshots/claude-diag-10-final.png' })
})
