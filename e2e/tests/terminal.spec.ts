import { test, expect, type Page } from '@playwright/test'

// ─── Helpers ──────────────────────────────────────────────────────────────────

async function waitForConnected(page: Page) {
  await expect(
    page.locator('[data-testid="connection-status-connected"]')
  ).toBeVisible({ timeout: 15_000 })
}

async function focusFirstPane(page: Page) {
  const pane = page.locator('[data-testid^="terminal-pane-"]').first()
  await pane.click()
  return pane
}

async function typeAndEnter(page: Page, text: string) {
  await page.keyboard.type(text)
  await page.keyboard.press('Enter')
}

async function expectOutput(page: Page, text: string, timeout = 5_000) {
  await expect(
    page.locator('[data-testid="terminal-accessible-output"]')
  ).toContainText(text, { timeout })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

test.describe('Terminal — basic I/O', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await waitForConnected(page)
  })

  test('connects and shows sessions in sidebar', async ({ page }) => {
    await expect(page.locator('.session-name')).toBeVisible()
  })

  test('types command and sees output', async ({ page }) => {
    await focusFirstPane(page)
    const marker = `oxmux_e2e_${Date.now()}`
    await typeAndEnter(page, `echo ${marker}`)
    await expectOutput(page, marker)
  })

  test('resize propagates to tmux', async ({ page }) => {
    await focusFirstPane(page)
    await page.setViewportSize({ width: 1400, height: 900 })
    await page.waitForTimeout(300) // debounce

    await typeAndEnter(page, 'echo ${COLUMNS}x${LINES}')
    // Should show large terminal dimensions
    const output = await page.locator('[data-testid="terminal-accessible-output"]').textContent()
    const cols = parseInt(output?.match(/(\d+)x/)?.[1] ?? '0')
    expect(cols).toBeGreaterThan(100)

    await page.setViewportSize({ width: 800, height: 600 })
    await page.waitForTimeout(300)

    await typeAndEnter(page, 'echo ${COLUMNS}x${LINES}')
    const output2 = await page.locator('[data-testid="terminal-accessible-output"]').textContent()
    const cols2 = parseInt(output2?.match(/(\d+)x/)?.[1] ?? '0')
    expect(cols2).toBeLessThan(cols)
  })

  test('reconnects after server restart', async ({ page }) => {
    await waitForConnected(page)

    // Server restart simulated by navigating away and back
    // In real CI this would restart the docker service
    await page.reload()
    await waitForConnected(page)

    await focusFirstPane(page)
    await typeAndEnter(page, 'echo still_alive_after_reconnect')
    await expectOutput(page, 'still_alive_after_reconnect')
  })
})

test.describe('Terminal — WebSocket connection state', () => {
  test('shows reconnecting state when WS is lost', async ({ page }) => {
    await page.goto('/')
    await waitForConnected(page)

    // Intercept and abort WS to simulate disconnect
    await page.route('**/ws', route => route.abort())
    await page.reload()

    // Should show reconnecting state
    await expect(
      page.locator('[data-testid="connection-status-reconnecting"], [data-testid="connection-status-connecting"]')
    ).toBeVisible({ timeout: 5_000 })
  })
})

test.describe('Claude Code session UI', () => {
  test.beforeEach(async ({ page }) => {
    // Use ?mock=true to get canned stream-json without a real Claude session
    await page.goto('/?mock_claude=true')
    await waitForConnected(page)
  })

  test('renders tool use blocks for Write operations', async ({ page }) => {
    const claudePane = page.locator('.claude-pane').first()
    await expect(claudePane).toBeVisible({ timeout: 10_000 })
    await expect(page.locator('.tool-use-block').first()).toBeVisible({ timeout: 10_000 })
  })

  test('cost meter updates from session accumulator', async ({ page }) => {
    await expect(page.locator('[data-testid="cost-meter"]'))
      .not.toContainText('$0.0000', { timeout: 10_000 })
  })

  test('changed files list populated after Write tool', async ({ page }) => {
    await expect(page.locator('[data-testid="changed-files"]'))
      .toBeVisible({ timeout: 10_000 })
  })
})

test.describe('TURN credentials', () => {
  test('ICE config endpoint returns valid structure', async ({ page }) => {
    const resp = await page.request.get('/api/ice-config?user=test')
    expect(resp.ok()).toBeTruthy()
    const body = await resp.json()
    expect(body).toHaveProperty('ice_servers')
    expect(Array.isArray(body.ice_servers)).toBe(true)
    expect(body.ice_servers.length).toBeGreaterThan(0)

    // Verify TURN entry has credentials
    const turnEntry = body.ice_servers.find((s: { username?: string }) => s.username)
    expect(turnEntry).toBeDefined()
    expect(turnEntry.username).toMatch(/^\d+:test$/) // "<timestamp>:test"
  })
})
