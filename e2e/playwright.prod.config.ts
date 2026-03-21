import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  forbidOnly: true,
  retries: 2,
  workers: 1,
  reporter: [['html'], ['list']],
  timeout: 60_000,
  globalSetup: './global-setup.ts',

  use: {
    baseURL: process.env.BASE_URL || process.env.BASE_URL || 'http://localhost:8080',
    trace: 'on-first-retry',
    video: 'on-first-retry',
    ignoreHTTPSErrors: true,
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})
