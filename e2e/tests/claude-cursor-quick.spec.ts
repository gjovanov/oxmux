/**
 * Quick validation: Claude Code prompt renders correctly after P2P connect
 * and survives view switching (single → mashed → single).
 *
 * Success: terminal contains ❯ prompt + bypass permissions line.
 */
import { test, expect } from '@playwright/test'

const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

/** Poll readTerm() until predicate is true or timeout */
async function waitForTerm(
  page: import('@playwright/test').Page,
  predicate: (content: string) => boolean,
  label: string,
  timeoutMs = 15_000,
): Promise<{ content: string; ok: boolean }> {
  const start = Date.now()
  let content = ''
  while (Date.now() - start < timeoutMs) {
    content = await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    }) || ''
    if (predicate(content)) {
      console.log(`[${label}] matched after ${Date.now() - start}ms`)
      return { content, ok: true }
    }
    await page.waitForTimeout(500)
  }
  console.log(`[${label}] timed out after ${timeoutMs}ms`)
  console.log(`[${label}] last 200 chars: ${content.slice(-200)}`)
  return { content, ok: false }
}

const hasPrompt = (c: string) => c.includes('❯') && c.includes('bypass permissions')

test('Claude Code prompt visible after WebRTC P2P connect + view switch', async ({ page, context }) => {
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

  // Agent + WebRTC P2P
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
  console.log('P2P connected:', logs.some(l => l.includes('WebRTC P2P connected!')))

  // ── Single view + select pane ──────────────────────────────────────
  const pn = page.locator('.pane-node').first()
  await expect(pn).toBeVisible({ timeout: 15_000 })

  const singleBtn = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await singleBtn.isVisible({ timeout: 3000 }).catch(() => false)) await singleBtn.click()
  await page.waitForTimeout(500)
  await pn.click()
  await page.waitForTimeout(1000)

  // Focus terminal
  const termPane = page.locator('.terminal-pane, .claude-pane').first()
  if (await termPane.isVisible({ timeout: 5000 }).catch(() => false)) await termPane.click()
  await page.waitForTimeout(2000)

  await page.screenshot({ path: 'e2e/screenshots/cursor-quick-01-initial.png' })

  // ── Exit any running Claude session ────────────────────────────────
  // Escape first (in case Claude is in a sub-prompt), then Ctrl+D × 2
  console.log('Exiting any running Claude session...')
  await page.keyboard.press('Escape')
  await page.waitForTimeout(500)
  await page.keyboard.down('Control')
  await page.keyboard.press('d')
  await page.keyboard.up('Control')
  await page.waitForTimeout(2000)

  // Check if we're at a shell prompt ($ or ~ or gjovanov)
  let { content } = await waitForTerm(page, c => c.includes('$') || c.includes('~'), 'shell-check', 5000)
  if (!content.includes('$') && !content.includes('~')) {
    // Still in Claude or nested — send another Ctrl+D
    await page.keyboard.down('Control')
    await page.keyboard.press('d')
    await page.keyboard.up('Control')
    await page.waitForTimeout(3000)
  }

  // ── Start FRESH Claude Code ────────────────────────────────────────
  console.log('Starting fresh Claude Code...')
  await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions', { delay: 10 })
  await page.keyboard.press('Enter')

  // Poll for Claude prompt (up to 60s)
  const startResult = await waitForTerm(page, hasPrompt, 'claude-start', 60_000)
  await page.screenshot({ path: 'e2e/screenshots/cursor-quick-02-claude-started.png' })
  console.log(`After start: prompt=${startResult.ok}`)

  // Gate: initial render must work before testing view switching
  expect(startResult.ok, 'Claude ❯ prompt should be visible after start').toBe(true)

  // ── View switch: Single → Mashed → Single ─────────────────────────
  console.log('Switching to mashed...')
  const mashedBtn = page.locator('.toggle-btn', { hasText: 'Mashed' })
  if (await mashedBtn.isVisible({ timeout: 3000 }).catch(() => false)) await mashedBtn.click()

  // Poll for prompt in mashed view (terminal is smaller, content may differ)
  const mashedResult = await waitForTerm(page, hasPrompt, 'mashed-view', 10_000)
  await page.screenshot({ path: 'e2e/screenshots/cursor-quick-03-mashed.png' })
  console.log(`Mashed: prompt=${mashedResult.ok}`)

  console.log('Switching back to single...')
  if (await singleBtn.isVisible({ timeout: 3000 }).catch(() => false)) await singleBtn.click()
  await page.waitForTimeout(500)
  await pn.click()

  // Poll for prompt after switching back to single (up to 15s for resize settle)
  const singleResult = await waitForTerm(page, hasPrompt, 'single-back', 15_000)
  await page.screenshot({ path: 'e2e/screenshots/cursor-quick-04-single-back.png' })
  console.log(`Single back: prompt=${singleResult.ok}`)

  // ── Status ─────────────────────────────────────────────────────────
  console.log(`P2P lost: ${logs.filter(l => l.includes('P2P lost')).length}`)
  console.log(`Control mode exits: ${logs.filter(l => l.includes('control mode exited')).length}`)

  // Assertions
  expect.soft(singleResult.ok, 'Claude ❯ prompt should be visible after view switch back to single').toBe(true)
})
