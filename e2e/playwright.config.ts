import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,       // terminal tests are stateful
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: [['html'], ['list']],
  timeout: 30_000,

  use: {
    baseURL: 'http://localhost:5173',
    trace: 'on-first-retry',
    video: 'on-first-retry',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  webServer: {
    command: 'docker compose up',
    url: 'http://localhost:8080/health',
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
})
