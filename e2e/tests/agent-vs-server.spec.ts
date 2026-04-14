/**
 * Direct comparison: connect to EXISTING Claude session via both paths.
 * Tests the actual user scenario: connect, see terminal, check cursor at ❯.
 */
import { test, expect } from '@playwright/test'

const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

async function readTerm(page: import('@playwright/test').Page): Promise<string> {
  return await page.evaluate(() => {
    const rows = document.querySelectorAll('.xterm-rows > div')
    return Array.from(rows).map(r => r.textContent || '').join('\n')
  }) || ''
}

async function waitForPrompt(
  page: import('@playwright/test').Page,
  label: string,
  timeoutMs = 15_000,
): Promise<{ content: string; ok: boolean }> {
  const start = Date.now()
  let content = ''
  while (Date.now() - start < timeoutMs) {
    content = await readTerm(page)
    if (content.includes('❯') && content.includes('bypass permissions')) {
      console.log(`[${label}] prompt found after ${Date.now() - start}ms`)
      return { content, ok: true }
    }
    await page.waitForTimeout(500)
  }
  console.log(`[${label}] NO prompt after ${timeoutMs}ms`)
  // Show what we DO see
  const rows = content.split('\n').filter(l => l.trim())
  console.log(`[${label}] ${rows.length} non-empty rows`)
  if (rows.length > 0) {
    console.log(`[${label}] first row: "${rows[0].slice(0, 100)}"`)
    console.log(`[${label}] last row: "${rows[rows.length - 1].slice(0, 100)}"`)
  }
  return { content, ok: false }
}

/** Ensure the pane is visible in a terminal by switching to Mashed and clicking pane node */
async function ensureTerminalVisible(page: import('@playwright/test').Page) {
  // Switch to Mashed view — it always renders TerminalPane (not ClaudePane)
  const mashedBtn = page.locator('.toggle-btn', { hasText: 'Mashed' })
  if (await mashedBtn.isVisible({ timeout: 3000 }).catch(() => false)) await mashedBtn.click()
  await page.waitForTimeout(500)

  // Click first pane node to assign it to a mashed cell
  const pn = page.locator('.pane-node').first()
  if (await pn.isVisible({ timeout: 3000 }).catch(() => false)) await pn.click()
  await page.waitForTimeout(1000)

  // Now switch to Single view for a larger terminal
  const singleBtn = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await singleBtn.isVisible({ timeout: 3000 }).catch(() => false)) await singleBtn.click()
  await page.waitForTimeout(500)

  // Click pane node again to select in single view
  if (await pn.isVisible({ timeout: 3000 }).catch(() => false)) await pn.click()
  await page.waitForTimeout(2000)
}

test('Agent vs Server: existing Claude session cursor position', async ({ page, context }) => {
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

  // Connect mars1 (SSH only — no agent yet)
  const mars1 = page.locator('.managed-session', { hasText: 'mars1' })
  const cb = mars1.locator('.action-btn.connect')
  if (await cb.isVisible({ timeout: 2000 }).catch(() => false)) {
    await cb.click()
    await expect(mars1.locator('.ms-status.connected')).toBeVisible({ timeout: 45_000 })
  }

  // Wait for pane tree
  await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 15_000 })

  // ── TEST 1: SSH server path ──────────────────────────────────────────
  console.log('\n=== TEST 1: SSH Server Path ===')
  await ensureTerminalVisible(page)

  const ssh1 = await waitForPrompt(page, 'ssh-initial', 15_000)
  await page.screenshot({ path: 'e2e/screenshots/avs-01-ssh-initial.png' })

  // ── TEST 1b: SSH + View Switch ────────────────────────────────────────
  console.log('\n=== TEST 1b: SSH + View Switch ===')
  const mashedBtn0 = page.locator('.toggle-btn', { hasText: 'Mashed' })
  if (await mashedBtn0.isVisible({ timeout: 3000 }).catch(() => false)) await mashedBtn0.click()
  await page.waitForTimeout(2000)
  const singleBtn0 = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await singleBtn0.isVisible({ timeout: 3000 }).catch(() => false)) await singleBtn0.click()
  await page.waitForTimeout(500)
  const pn0 = page.locator('.pane-node').first()
  if (await pn0.isVisible({ timeout: 3000 }).catch(() => false)) await pn0.click()
  await page.waitForTimeout(3000)

  const sshAfterSwitch = await waitForPrompt(page, 'ssh-after-switch', 10_000)
  await page.screenshot({ path: 'e2e/screenshots/avs-01b-ssh-after-switch.png' })
  console.log(`SSH after view switch: prompt=${sshAfterSwitch.ok}`)

  // ── Upgrade to WebRTC P2P ────────────────────────────────────────────
  console.log('\n=== Upgrading to WebRTC P2P ===')
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
  await page.waitForTimeout(3000)

  // ── TEST 2: Agent P2P path (same terminal, now via agent) ────────────
  console.log('\n=== TEST 2: Agent P2P Path ===')
  const agent1 = await waitForPrompt(page, 'agent-initial', 15_000)
  await page.screenshot({ path: 'e2e/screenshots/avs-02-agent-initial.png' })

  // ── TEST 3: View switch on agent path ────────────────────────────────
  console.log('\n=== TEST 3: Agent + View Switch ===')

  // Read terminal content before switch
  let preSwitchContent = await readTerm(page)
  console.log(`Before switch: ${preSwitchContent.split('\n').filter(l => l.trim()).length} rows, has prompt: ${preSwitchContent.includes('❯')}`)

  const mashedBtn = page.locator('.toggle-btn', { hasText: 'Mashed' })
  if (await mashedBtn.isVisible({ timeout: 3000 }).catch(() => false)) await mashedBtn.click()
  await page.waitForTimeout(1000)

  // Read in mashed view
  let mashedContent = await readTerm(page)
  console.log(`Mashed: ${mashedContent.split('\n').filter(l => l.trim()).length} rows, has prompt: ${mashedContent.includes('❯')}`)
  await page.screenshot({ path: 'e2e/screenshots/avs-03a-mashed.png' })

  const singleBtn = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await singleBtn.isVisible({ timeout: 3000 }).catch(() => false)) await singleBtn.click()
  await page.waitForTimeout(500)
  const pn = page.locator('.pane-node').first()
  if (await pn.isVisible({ timeout: 3000 }).catch(() => false)) await pn.click()

  // Check immediately after switch
  await page.waitForTimeout(500)
  let immContent = await readTerm(page)
  console.log(`Immediate after switch: ${immContent.split('\n').filter(l => l.trim()).length} rows, has prompt: ${immContent.includes('❯')}`)

  // Wait for settling + space+backspace to take effect
  await page.waitForTimeout(3000)
  let settledContent = await readTerm(page)
  console.log(`After 3s settle: ${settledContent.split('\n').filter(l => l.trim()).length} rows, has prompt: ${settledContent.includes('❯')}`)

  // Wait even longer
  const agent2 = await waitForPrompt(page, 'agent-after-switch', 15_000)
  await page.screenshot({ path: 'e2e/screenshots/avs-03-agent-after-switch.png' })

  // ── TEST 4: Back to SSH ──────────────────────────────────────────────
  console.log('\n=== TEST 4: Back to SSH ===')
  const backBtn = mars1.locator('button', { hasText: 'Back to SSH' })
  if (await backBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await backBtn.click()
    await page.waitForTimeout(5000)
  }

  const ssh2 = await waitForPrompt(page, 'ssh-after-agent', 15_000)
  await page.screenshot({ path: 'e2e/screenshots/avs-04-ssh-after-agent.png' })

  // ── Summary ──────────────────────────────────────────────────────────
  console.log('\n=== SUMMARY ===')
  console.log(`SSH initial:         prompt=${ssh1.ok}`)
  console.log(`Agent initial:       prompt=${agent1.ok}`)
  console.log(`Agent after switch:  prompt=${agent2.ok}`)
  console.log(`SSH after agent:     prompt=${ssh2.ok}`)
  console.log(`P2P lost: ${logs.filter(l => l.includes('P2P lost')).length}`)
  console.log(`Control mode exits: ${logs.filter(l => l.includes('control mode exited')).length}`)

  expect.soft(ssh1.ok, 'SSH initial should show prompt').toBe(true)
  expect.soft(agent1.ok, 'Agent initial should show prompt').toBe(true)
  expect.soft(agent2.ok, 'Agent after view switch should show prompt').toBe(true)
  expect.soft(ssh2.ok, 'SSH after agent should show prompt').toBe(true)
})
