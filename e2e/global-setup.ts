import { request } from '@playwright/test'

const BASE_URL = process.env.BASE_URL || 'https://oxmux.app'
const TEST_USER = 'e2e_test_user'
const TEST_PASS = 'e2e_test_pass_1234'

/**
 * Global setup: ensure the E2E test user exists.
 * Runs once before all tests. Tries register, falls back to login.
 */
async function globalSetup() {
  const api = await request.newContext({ baseURL: BASE_URL })

  // Try register
  let res = await api.post('/api/auth/register', {
    data: { username: TEST_USER, password: TEST_PASS },
  })

  if (res.status() === 409) {
    // User exists — login to verify credentials work
    res = await api.post('/api/auth/login', {
      data: { username: TEST_USER, password: TEST_PASS },
    })
  }

  if (!res.ok()) {
    const body = await res.text()
    throw new Error(`Failed to setup test user: ${res.status()} ${body}`)
  }

  const data = await res.json()
  console.log(`[global-setup] Test user ready: ${data.user.username} (${data.user.id})`)

  // Store auth state for tests
  process.env.E2E_TOKEN = data.token
  process.env.E2E_USER_ID = data.user.id

  await api.dispose()
}

export default globalSetup
