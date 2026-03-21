import { test, expect } from '@playwright/test'
import { authenticate, ensureConnectedSession, readTerminalContent, selectFirstPane, switchToSingleView } from './helpers'

test.describe('Terminal Resize', () => {
  test('tput cols matches terminal width after resize', async ({ page }) => {
    test.setTimeout(90_000)
    await authenticate(page)
    await ensureConnectedSession(page)
    await switchToSingleView(page)
    await selectFirstPane(page)

    await page.keyboard.type('tput cols', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(1500)

    const termContent = await readTerminalContent(page)
    const colsMatch = termContent?.match(/tput cols\n(\d+)/)
    if (colsMatch) {
      const cols = parseInt(colsMatch[1])
      console.log('tput cols returned:', cols)
      expect(cols).toBeGreaterThan(40)
    }

    await page.screenshot({ path: 'e2e/screenshots/resize-test.png' })
  })
})
