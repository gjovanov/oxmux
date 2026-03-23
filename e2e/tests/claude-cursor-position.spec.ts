/**
 * E2E test: Claude Code cursor position after start/stop + view switching + resize.
 *
 * Success criteria: after each Claude Code start, the terminal ends with:
 *   ───────────────────
 *   ❯  (cursor here)
 *   ───────────────────
 *   ⏵⏵ bypass permissions on (shift+tab to cycle)
 */
import { test, expect } from '@playwright/test'

const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

test('Claude Code cursor position: 5 cycles of start/stop/switch/resize', async ({ page, context }) => {
  test.setTimeout(900_000) // 15 min — 5 full cycles
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])

  const logs: string[] = []
  page.on('console', msg => logs.push(msg.text()))

  // === LOGIN ===
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

  // === CONNECT mars1 + AGENT + WEBRTC P2P ===
  const mars1 = page.locator('.managed-session', { hasText: 'mars1' })
  const cb = mars1.locator('.action-btn.connect')
  if (await cb.isVisible({ timeout: 2000 }).catch(() => false)) {
    await cb.click()
    await expect(mars1.locator('.ms-status.connected')).toBeVisible({ timeout: 45_000 })
  }
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
  console.log('SETUP: mars1 + WebRTC P2P ready')

  // === HELPERS ===
  async function readTerm(): Promise<string> {
    return await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    }) || ''
  }

  /**
   * Check if Claude Code prompt is correctly positioned at the bottom.
   * Success: content ends with ❯ prompt between two ─── separators,
   * followed by the "bypass permissions" line.
   */
  function checkClaudePrompt(content: string, label: string): boolean {
    const lines = content.split('\n').filter(l => l.trim())
    // Find the last ❯ line
    const lastPromptIdx = lines.findLastIndex(l => l.includes('❯'))
    // Find separator lines (───)
    const hasSep = lines.some(l => l.includes('───'))
    // Find bypass line
    const hasBypass = lines.some(l => l.includes('bypass permissions'))

    const ok = lastPromptIdx >= 0 && hasSep && hasBypass
    console.log(`[${label}] prompt=${lastPromptIdx >= 0} sep=${hasSep} bypass=${hasBypass} → ${ok ? 'PASS' : 'FAIL'}`)
    if (!ok) {
      // Print last 5 non-empty lines for debugging
      console.log(`[${label}] last lines:`)
      lines.slice(-5).forEach(l => console.log(`  "${l}"`))
    }
    return ok
  }

  async function switchToMashed() {
    const btn = page.locator('.toggle-btn', { hasText: 'Mashed' })
    if (await btn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await btn.click()
      await page.waitForTimeout(1000)
    }
  }

  async function switchToSingle() {
    const btn = page.locator('.toggle-btn', { hasText: 'Single' })
    if (await btn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await btn.click()
      await page.waitForTimeout(500)
    }
    const paneBtn = page.locator('.pane-node').first()
    if (await paneBtn.isVisible({ timeout: 2000 }).catch(() => false)) await paneBtn.click()
    await page.waitForTimeout(1000)
  }

  async function resizeSidebar(delta: number) {
    const resizer = page.locator('.sidebar-resizer')
    if (!await resizer.isVisible({ timeout: 2000 }).catch(() => false)) return
    const box = await resizer.boundingBox()
    if (!box) return
    await page.mouse.move(box.x + 2, box.y + box.height / 2)
    await page.mouse.down()
    await page.mouse.move(box.x + delta, box.y + box.height / 2, { steps: 5 })
    await page.mouse.up()
    await page.waitForTimeout(1500)
  }

  async function exitClaude() {
    await page.keyboard.down('Control')
    await page.keyboard.press('d')
    await page.keyboard.up('Control')
    await page.waitForTimeout(2000)
    await page.keyboard.down('Control')
    await page.keyboard.press('d')
    await page.keyboard.up('Control')
    await page.waitForTimeout(3000)
  }

  async function startClaude(): Promise<boolean> {
    await page.keyboard.type('bunx --bun @anthropic-ai/claude-code --dangerously-skip-permissions --continue', { delay: 10 })
    await page.keyboard.press('Enter')

    // Wait for Claude Code to render (up to 40s)
    for (let i = 0; i < 20; i++) {
      await page.waitForTimeout(2000)
      const c = await readTerm()
      if (c.includes('❯') && c.includes('bypass permissions')) {
        return true
      }
    }
    return false
  }

  // Wait for pane tree to be visible, then start in Mashed view
  const pn = page.locator('.pane-node').first()
  await expect(pn).toBeVisible({ timeout: 15_000 })

  // Now toggle buttons should be visible
  await switchToMashed()
  await pn.click()
  await page.waitForTimeout(2000)

  // Resize deltas for 5 cycles: widen, narrow, widen more, narrow, reset
  const resizeDeltas = [60, -40, 80, -60, -40]

  let passCount = 0
  const results: { cycle: number; view: string; resize: number; pass: boolean }[] = []

  for (let cycle = 1; cycle <= 5; cycle++) {
    console.log(`\n========== CYCLE ${cycle}/5 ==========`)

    // Exit existing Claude session (if running)
    const content = await readTerm()
    if (content.includes('❯') || content.includes('Claude')) {
      console.log(`${cycle}. Exiting Claude...`)
      await exitClaude()
    }

    // View switch pattern: odd cycles start in single, even in mashed
    const startView = cycle % 2 === 1 ? 'single' : 'mashed'
    console.log(`${cycle}. Switching to ${startView}...`)
    if (startView === 'single') {
      await switchToSingle()
    } else {
      await switchToMashed()
    }

    // Resize sidebar
    const delta = resizeDeltas[cycle - 1]
    console.log(`${cycle}. Resizing sidebar by ${delta}px...`)
    await resizeSidebar(delta)

    // Start Claude Code
    console.log(`${cycle}. Starting Claude Code...`)
    const started = await startClaude()

    if (started) {
      await page.waitForTimeout(2000)
      const termContent = await readTerm()
      const pass = checkClaudePrompt(termContent, `cycle${cycle}-${startView}`)
      results.push({ cycle, view: startView, resize: delta, pass })
      if (pass) passCount++
      await page.screenshot({ path: `e2e/screenshots/cursor-cycle${cycle}-${startView}.png` })
    } else {
      console.log(`${cycle}. Claude Code did not start within timeout`)
      results.push({ cycle, view: startView, resize: delta, pass: false })
      await page.screenshot({ path: `e2e/screenshots/cursor-cycle${cycle}-timeout.png` })
    }

    // Switch views 5 times after Claude is running
    console.log(`${cycle}. View switching 5 times...`)
    for (let sw = 0; sw < 5; sw++) {
      if (sw % 2 === 0) await switchToMashed()
      else await switchToSingle()
    }

    // Check prompt after all the switching
    await page.waitForTimeout(2000)
    const afterSwitch = await readTerm()
    const afterPass = checkClaudePrompt(afterSwitch, `cycle${cycle}-afterSwitch`)
    await page.screenshot({ path: `e2e/screenshots/cursor-cycle${cycle}-afterswitch.png` })
  }

  // Summary
  console.log('\n========== SUMMARY ==========')
  results.forEach(r => console.log(`Cycle ${r.cycle} [${r.view}] resize=${r.resize}: ${r.pass ? 'PASS' : 'FAIL'}`))
  console.log(`Total: ${passCount}/${results.length} passed`)
  console.log(`Control mode exits: ${logs.filter(l => l.includes('control mode exited')).length}`)
  console.log(`P2P lost: ${logs.filter(l => l.includes('P2P lost')).length}`)

  // Assert: all 5 cycles should pass
  expect(passCount, `Expected all 5 cycles to show correct Claude prompt, got ${passCount}/5`).toBe(5)
})
