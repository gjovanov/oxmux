import { test, expect, Page } from '@playwright/test'
import * as fs from 'fs'
import * as path from 'path'
import * as os from 'os'

const BASE_URL = process.env.BASE_URL || 'http://localhost:8080'
const TEST_USER = 'e2e_test_user'
const TEST_PASS = 'e2e_test_pass_1234'
const SSH_HOST = process.env.E2E_SSH_HOST || '127.0.0.1'
const SSH_USER = process.env.E2E_SSH_USER || 'test'

// Path to a real key for E2E testing (same as used by other tests)
const SSH_KEY_PATH = process.env.E2E_SSH_KEY || path.join(os.homedir(), '.ssh', 'id_ed25519')

async function authenticate(page: Page) {
  await page.goto(BASE_URL)
  await page.waitForLoadState('networkidle')

  if (await page.locator('.session-sidebar').isVisible({ timeout: 2000 }).catch(() => false)) {
    return
  }

  const loginTab = page.locator('button', { hasText: 'Login' })
  if (await loginTab.isVisible({ timeout: 2000 }).catch(() => false)) {
    await loginTab.click()
  }

  await page.locator('input[type="text"]').fill(TEST_USER)
  await page.locator('input[type="password"]').fill(TEST_PASS)
  await page.locator('button[type="submit"]').click()

  await expect(page.locator('.session-sidebar')).toBeVisible({ timeout: 15_000 })
}

async function deleteSession(page: Page, sessionName: string) {
  const card = page.locator('.managed-session', { hasText: sessionName })
  if (await card.isVisible({ timeout: 2000 }).catch(() => false)) {
    const disconnectBtn = card.locator('button', { hasText: 'Disconnect' })
    if (await disconnectBtn.isVisible({ timeout: 500 }).catch(() => false)) {
      await disconnectBtn.click()
      await page.waitForTimeout(1000)
    }
    const deleteBtn = card.locator('.action-btn.delete')
    if (await deleteBtn.isVisible({ timeout: 500 }).catch(() => false)) {
      await deleteBtn.click()
      await page.waitForTimeout(500)
    }
  }
}

test.describe('SSH Key Upload', () => {
  let sessionName: string

  test.beforeEach(async ({ page }) => {
    sessionName = `key-upload-test-${Date.now()}`
    await authenticate(page)
  })

  test.afterEach(async ({ page }) => {
    await deleteSession(page, sessionName)
  })

  test('upload key option appears in auth method dropdown', async ({ page }) => {
    await page.locator('.add-btn').click()
    await expect(page.locator('.dialog')).toBeVisible()

    // Select SSH backend
    await page.locator('select').first().selectOption('ssh')

    // Check auth method dropdown has "Upload Key" option
    const authSelect = page.locator('select').nth(1)
    const options = await authSelect.locator('option').allTextContents()
    expect(options).toContain('Upload Key')

    await page.screenshot({ path: `test-results/key-upload-dropdown-${Date.now()}.png` })
  })

  test('file input appears when Upload Key is selected', async ({ page }) => {
    await page.locator('.add-btn').click()
    await expect(page.locator('.dialog')).toBeVisible()

    await page.locator('select').first().selectOption('ssh')
    await page.locator('select').nth(1).selectOption('uploaded_key')

    // File input should be visible
    const fileInput = page.locator('input[type="file"]')
    await expect(fileInput).toBeVisible()

    // Passphrase field should be visible
    const passphraseInput = page.locator('input[type="password"]')
    await expect(passphraseInput).toBeVisible()

    await page.screenshot({ path: `test-results/key-upload-file-input-${Date.now()}.png` })
  })

  test('create button disabled without key file', async ({ page }) => {
    await page.locator('.add-btn').click()
    await expect(page.locator('.dialog')).toBeVisible()

    await page.locator('input[placeholder="my-project"]').fill(sessionName)
    await page.locator('select').first().selectOption('ssh')
    await page.locator('input[placeholder="192.0.2.1"]').fill(SSH_HOST)
    await page.locator('input[placeholder="ubuntu"]').fill(SSH_USER)
    await page.locator('select').nth(1).selectOption('uploaded_key')

    // Create button should be disabled (no key file selected)
    const createBtn = page.locator('button', { hasText: 'Create' }).first()
    await expect(createBtn).toBeDisabled()
  })

  test('creates session with uploaded key and connects', async ({ page }) => {
    test.setTimeout(120_000)
    await page.setViewportSize({ width: 1280, height: 900 })

    // Skip if key file doesn't exist
    if (!fs.existsSync(SSH_KEY_PATH)) {
      test.skip(true, `SSH key not found at ${SSH_KEY_PATH}`)
      return
    }

    await page.locator('.add-btn').click()
    await expect(page.locator('.dialog')).toBeVisible()

    await page.locator('input[placeholder="my-project"]').fill(sessionName)
    await page.locator('select').first().selectOption('ssh')
    await page.locator('input[placeholder="192.0.2.1"]').fill(SSH_HOST)
    await page.locator('input[placeholder="ubuntu"]').fill(SSH_USER)
    await page.locator('select').nth(1).selectOption('uploaded_key')

    // Upload the key file
    const fileInput = page.locator('input[type="file"]')
    await fileInput.setInputFiles(SSH_KEY_PATH)

    // Wait for file to be read
    await page.waitForTimeout(500)

    // Key name should be shown
    const keyStatus = page.locator('.key-status')
    await expect(keyStatus).toBeVisible()

    await page.screenshot({ path: `test-results/key-upload-selected-${Date.now()}.png` })

    // Create button should be enabled now
    const createBtn = page.locator('button', { hasText: /Create|Uploading/ }).first()
    await expect(createBtn).toBeEnabled()
    await createBtn.scrollIntoViewIfNeeded()
    await createBtn.click({ force: true })

    // Dialog should close after upload + create
    await expect(page.locator('.dialog')).not.toBeVisible({ timeout: 10_000 })

    // Session card should appear
    const card = page.locator('.managed-session', { hasText: sessionName })
    await expect(card).toBeVisible({ timeout: 5000 })

    await page.screenshot({ path: `test-results/key-upload-created-${Date.now()}.png` })

    // Connect the session
    const connectBtn = card.locator('button', { hasText: 'Connect' })
    if (await connectBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      await connectBtn.click()
    }

    // Wait for connection
    await expect(card.locator('.ms-status.connected')).toBeVisible({ timeout: 30_000 })

    await page.screenshot({ path: `test-results/key-upload-connected-${Date.now()}.png` })

    // Verify panes visible
    const panes = page.locator('.pane-node')
    await expect(panes.first()).toBeVisible({ timeout: 10_000 })

    console.log('[test] key upload session connected successfully')
  })
})
