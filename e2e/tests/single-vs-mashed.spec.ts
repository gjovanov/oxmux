import { test, expect } from '@playwright/test'

const USER = process.env.E2E_USER || 'gjovanov2'
const PASS = process.env.E2E_PASS || 'Gj12345!!'

test('single vs mashed view + sidebar resize', async ({ page, context }) => {
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

  // Connect mars1 + agent + WebRTC P2P
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
  console.log('Setup done')

  async function readTerm(): Promise<string> {
    return await page.evaluate(() => {
      const rows = document.querySelectorAll('.xterm-rows > div')
      return Array.from(rows).map(r => r.textContent || '').join('\n')
    }) || ''
  }

  async function check(label: string): Promise<number> {
    await page.waitForTimeout(3000)
    const c = await readTerm()
    const lines = c.split('\n').filter(l => l.trim()).length
    console.log(`[${label}] ${lines} lines — ${c.replace(/\n+/g, ' | ').slice(0, 120)}`)
    await page.screenshot({ path: `e2e/screenshots/svm-${label}.png` })
    return lines
  }

  // === 1. Mashed view (should work) ===
  await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
  await page.waitForTimeout(1000)
  // Ensure pane is assigned to a cell
  const pn = page.locator('.pane-node').first()
  if (await pn.isVisible({ timeout: 3000 }).catch(() => false)) await pn.click()
  await page.waitForTimeout(2000)
  const mashedLines = await check('01-mashed')

  // === 2. Switch to Single (user reports partial) ===
  await page.locator('.toggle-btn', { hasText: 'Single' }).click()
  await page.waitForTimeout(500)
  // Need to click pane again in single view
  if (await pn.isVisible({ timeout: 3000 }).catch(() => false)) await pn.click()
  await page.waitForTimeout(2000)
  const singleLines = await check('02-single')

  // === 3. Back to Mashed (should still work) ===
  await page.locator('.toggle-btn', { hasText: 'Mashed' }).click()
  await page.waitForTimeout(2000)
  const mashedLines2 = await check('03-mashed-back')

  // === 4. Resize left panel in mashed view (user reports partial after) ===
  const resizer = page.locator('.sidebar-resizer')
  if (await resizer.isVisible({ timeout: 2000 }).catch(() => false)) {
    const box = await resizer.boundingBox()
    if (box) {
      await page.mouse.move(box.x + 2, box.y + box.height / 2)
      await page.mouse.down()
      await page.mouse.move(box.x + 80, box.y + box.height / 2, { steps: 5 })
      await page.mouse.up()
    }
  }
  const afterResize = await check('04-mashed-after-resize')

  // === 5. Single after resize ===
  await page.locator('.toggle-btn', { hasText: 'Single' }).click()
  await page.waitForTimeout(500)
  if (await pn.isVisible({ timeout: 3000 }).catch(() => false)) await pn.click()
  await page.waitForTimeout(2000)
  const singleAfterResize = await check('05-single-after-resize')

  // Check control mode crashes
  const ctrlExits = logs.filter(l => l.includes('control mode exited')).length
  console.log(`\nControl mode exits: ${ctrlExits}`)
  console.log(`P2P lost: ${logs.filter(l => l.includes('P2P lost')).length}`)

  // Summary
  console.log('\n=== SUMMARY ===')
  console.log(`Mashed initial: ${mashedLines} lines`)
  console.log(`Single initial: ${singleLines} lines`)
  console.log(`Mashed back: ${mashedLines2} lines`)
  console.log(`Mashed after resize: ${afterResize} lines`)
  console.log(`Single after resize: ${singleAfterResize} lines`)

  // Assertions
  expect(mashedLines, 'Mashed should have content').toBeGreaterThan(5)
  expect.soft(singleLines, 'Single should have similar content as mashed').toBeGreaterThan(mashedLines - 5)
  expect.soft(afterResize, 'Mashed after resize should have content').toBeGreaterThan(5)
})
