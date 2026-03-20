import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  workers: 1,
  timeout: 60_000,
  use: {
    baseURL: 'https://oxmux.app',
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
})
