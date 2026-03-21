import { expect, Page } from '@playwright/test'

export const BASE_URL = process.env.BASE_URL || 'http://localhost:8080'
export const TEST_USER = requireEnv('E2E_USER', 'e2e_test')
export const TEST_PASS = requireEnv('E2E_PASS', 'e2e_test_pass')
export const SSH_HOST = requireEnv('E2E_SSH_HOST', '127.0.0.1')
export const SSH_USER = requireEnv('E2E_SSH_USER', 'test')
export const SSH_KEY = requireEnv('E2E_SSH_KEY', '~/.ssh/id_ed25519')

function requireEnv(name: string, fallback: string): string {
  return process.env[name] || fallback
}

export async function authenticate(page: Page) {
  await page.goto(BASE_URL)
  await page.waitForLoadState('networkidle')

  if (await page.locator('.session-sidebar').isVisible({ timeout: 3000 }).catch(() => false)) return

  const loginTab = page.locator('button', { hasText: 'Login' })
  if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) await loginTab.click()

  await page.locator('input[type="text"]').fill(TEST_USER)
  await page.locator('input[type="password"]').fill(TEST_PASS)
  await page.locator('button[type="submit"]').click()
  await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
}

export async function ensureConnectedSession(page: Page) {
  if (await page.locator('.pane-node').first().isVisible({ timeout: 2000 }).catch(() => false)) return

  const connectBtn = page.locator('.action-btn.connect').first()
  if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await connectBtn.click()
    await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 45_000 })
    return
  }

  // Create new session
  await page.locator('.add-btn').click()
  await expect(page.locator('.dialog')).toBeVisible({ timeout: 5000 })
  await page.locator('input[placeholder="my-project"]').fill('e2e-' + Date.now())
  await page.locator('select').first().selectOption('ssh')
  await page.locator('input[placeholder="192.0.2.1"]').fill(SSH_HOST)
  await page.locator('input[placeholder="ubuntu"]').fill(SSH_USER)
  await page.locator('select').nth(1).selectOption('private_key')
  await page.locator('input[placeholder="~/.ssh/id_ed25519"]').fill(SSH_KEY)
  await page.locator('button', { hasText: 'Create' }).click()
  await page.waitForTimeout(1000)
  await page.locator('.action-btn.connect').first().click()
  await expect(page.locator('.pane-node').first()).toBeVisible({ timeout: 45_000 })
}

export async function selectFirstPane(page: Page) {
  await page.locator('.pane-node').first().click()
  await expect(page.locator('.xterm-screen')).toBeVisible({ timeout: 10_000 })
  await page.locator('.terminal-pane').click()
  await page.waitForTimeout(500)
}

export async function readTerminalContent(page: Page): Promise<string> {
  return await page.evaluate(() => {
    const rows = document.querySelectorAll('.xterm-rows > div')
    return Array.from(rows).map(r => r.textContent || '').join('\n')
  }) || ''
}

export function collectConsoleLogs(page: Page) {
  const logs: string[] = []
  page.on('console', msg => {
    const text = msg.text()
    if (text.includes('[oxmux') || text.includes('xterm.js')) {
      logs.push(`[${new Date().toISOString()}] ${text}`)
    }
  })
  return {
    logs,
    flush: () => { const copy = [...logs]; logs.length = 0; return copy },
    hasPattern: (pattern: string) => logs.some(l => l.includes(pattern)),
  }
}

export async function waitForAgentOnline(page: Page, timeout = 60_000) {
  const agentOnline = page.locator('.agent-dot.online')
  if (await agentOnline.isVisible({ timeout: 3000 }).catch(() => false)) return

  // Try installing agent
  const installBtn = page.locator('.action-btn.install')
  if (await installBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
    await installBtn.click()
  }

  await expect(agentOnline).toBeVisible({ timeout })
}

export async function upgradeToWebRtc(page: Page, consoleLogs: ReturnType<typeof collectConsoleLogs>, timeout = 30_000) {
  const webrtcBtn = page.locator('.action-btn.p2p', { hasText: 'WebRTC P2P' })
  await expect(webrtcBtn).toBeVisible({ timeout: 5000 })
  await webrtcBtn.click()

  // Wait for WebRTC P2P connected in console logs
  const start = Date.now()
  while (Date.now() - start < timeout) {
    if (consoleLogs.hasPattern('WebRTC P2P connected!')) return
    await page.waitForTimeout(500)
  }
  throw new Error('WebRTC P2P connection timeout')
}

export async function switchToSingleView(page: Page) {
  const btn = page.locator('.toggle-btn', { hasText: 'Single' })
  if (await btn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await btn.click()
  }
}
