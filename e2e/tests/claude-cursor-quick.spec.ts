/**
 * Quick validation: Claude Code prompt renders correctly after P2P connect.
 * Success: terminal ends with ❯ prompt + bypass permissions line.
 */
import { test, expect } from '@playwright/test'

const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

test('Claude Code prompt visible after WebRTC P2P connect', async ({ page, context }) => {
  test.setTimeout(300_000)
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])

  const logs: string[] = []
  page.on('console', msg => logs.push(msg.text()))

  // Login
  await page.goto('https://oxmux.app')
  await page.waitForLoadState('networkidle')
  if (!await page.locator('.session-sidebar').isVisible({ timeout: 3000 }).catch(() => false)) {
    const lt = page.locator('button', { hasText: 'Login' })
    if (await lt.isVisible({ timeout: 2000 }).catch(() => false)) await lt.click()
    await page.locator('input[type="text"]').fill(USER)
    await page.locator('input[type="password"]').fill(PASS)
    await page.locator('button[type="submit"]').click()
    await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
  }

  // Connect mars1
  const mars1 = page.locator('.managed-session', { hasText: 'mars1' })
  const cb = mars1.locator('.action-btn.connect')
  if (await cb.isVisible({ timeout: 2000 }).catch(() => false)) {
    await cb.click()
    await expect(mars1.locator('.ms-status.connected')).toBeVisible({ timeout: 45_000 })
  }

  // Agent + WebRTC
  const ib = mars1.locator('.action-btn.install')
  if (await ib.isVisible({ timeout: 3000 }).catch(() => false)) {
    await ib.click()
    await expect(mars1.locator('.agent-dot.online')).toBeVisible({ timeout: 90_000 })
  }
  const wb = mars1.locator('.action-btn.p2p', { hasText: 'WebRTC P2P' })
  if (await wb.isVisible({ timeout: 5000 }).catch(() => false)) {
    await wb.click()
    for (let i = 0; i < 30; i++) {
      await page.waitForTimeout(1000)
      if (logs.some(l => l.includes('WebRTC P2P connected!'))) break
    }
  }

  // Single view + select pane
  const pn = page.locator('.pane-node').first()
  await expect(pn).toBeVisible({ timeout: 15_000 })

  const singleBtn = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await singleBtn.isVisible({ timeout: 3000 }).catch(() => false)) await singleBtn.click()
  await page.waitForTimeout(500)
  await pn.click()
  await page.waitForTimeout(1000)
  await page.locator('.terminal-pane').click()
  await page.waitForTimeout(3000)

  // Read terminal content
  async function readTerm(): Promise<string> {
    return await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    }) || ''
  }

  let content = await readTerm()
  console.log('Initial content (last 200):', content.slice(-200))
  await page.screenshot({ path: 'e2e/screenshots/cursor-quick-01-initial.png' })

  // Exit any running Claude session first
  console.log('Exiting any running Claude session...')
  await page.keyboard.down('Control')
  await page.keyboard.press('d')
  await page.keyboard.up('Control')
  await page.waitForTimeout(2000)
  await page.keyboard.down('Control')
  await page.keyboard.press('d')
  await page.keyboard.up('Control')
  await page.waitForTimeout(3000)

  // Start FRESH Claude Code (no --continue to avoid long history)
  {
    console.log('Starting fresh Claude Code...')
    await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions', { delay: 10 })
    await page.keyboard.press('Enter')

    // Wait for Claude to render (up to 60s)
    for (let i = 0; i < 30; i++) {
      await page.waitForTimeout(2000)
      content = await readTerm()
      if (content.includes('❯') && content.includes('bypass permissions')) {
        console.log(`Claude started after ${(i + 1) * 2}s`)
        break
      }
    }
    await page.screenshot({ path: 'e2e/screenshots/cursor-quick-02-claude-started.png' })
  }

  content = await readTerm()
  const hasPrompt = content.includes('❯')
  const hasBypass = content.includes('bypass permissions')
  console.log(`After start: prompt=${hasPrompt} bypass=${hasBypass}`)

  // Now switch to mashed, then back to single
  console.log('Switching to mashed...')
  const mashedBtn = page.locator('.toggle-btn', { hasText: 'Mashed' })
  if (await mashedBtn.isVisible({ timeout: 3000 }).catch(() => false)) await mashedBtn.click()
  await page.waitForTimeout(3000)

  content = await readTerm()
  console.log('Mashed (last 200):', content.slice(-200))
  await page.screenshot({ path: 'e2e/screenshots/cursor-quick-03-mashed.png' })

  console.log('Switching back to single...')
  if (await singleBtn.isVisible({ timeout: 3000 }).catch(() => false)) await singleBtn.click()
  await page.waitForTimeout(500)
  await pn.click()
  await page.waitForTimeout(5000)

  content = await readTerm()
  console.log('Single back (last 200):', content.slice(-200))
  await page.screenshot({ path: 'e2e/screenshots/cursor-quick-04-single-back.png' })

  const finalPrompt = content.includes('❯')
  const finalBypass = content.includes('bypass permissions')
  console.log(`Final: prompt=${finalPrompt} bypass=${finalBypass}`)

  // Log P2P status
  console.log(`P2P lost: ${logs.filter(l => l.includes('P2P lost')).length}`)
  console.log(`Control mode exits: ${logs.filter(l => l.includes('control mode exited')).length}`)

  expect.soft(finalPrompt, 'Claude ❯ prompt should be visible').toBe(true)
  expect.soft(finalBypass, 'bypass permissions line should be visible').toBe(true)
})
