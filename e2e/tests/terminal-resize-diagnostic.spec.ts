import { test, expect } from '@playwright/test'
import { authenticate, ensureConnectedSession, selectFirstPane, readTerminalContent, switchToSingleView } from './helpers'

test.describe('Terminal Resize Diagnostic', () => {
  test('tput cols matches actual terminal width', async ({ page }) => {
    test.setTimeout(90_000)
    await authenticate(page)
    await ensureConnectedSession(page)
    await switchToSingleView(page)
    await selectFirstPane(page)

    // Set wide viewport
    await page.setViewportSize({ width: 1400, height: 800 })
    await page.waitForTimeout(2000) // wait for FitAddon + resize propagation

    await page.screenshot({ path: 'e2e/screenshots/resize-diag-01-viewport.png' })

    // Get client-side xterm cols
    const clientInfo = await page.evaluate(() => {
      const container = document.querySelector('.xterm-container')
      return {
        containerWidth: container?.clientWidth || 0,
        containerHeight: container?.clientHeight || 0,
      }
    })
    console.log('Client info:', clientInfo)

    // Get sidebar pane size display
    const paneSizeText = await page.locator('.pane-size').first().textContent()
    console.log('Sidebar pane size:', paneSizeText)

    // Run tput cols
    await page.keyboard.type('tput cols', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(1500)

    // Run stty size
    await page.keyboard.type('stty size', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(1500)

    // Run echo $COLUMNS
    await page.keyboard.type('echo $COLUMNS', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(1500)

    const content = await readTerminalContent(page)
    console.log('Terminal content (first 500):', content.slice(0, 500))

    await page.screenshot({ path: 'e2e/screenshots/resize-diag-02-output.png' })

    // Extract tput cols value
    const tputMatch = content.match(/tput cols\n(\d+)/)
    const tputCols = tputMatch ? parseInt(tputMatch[1]) : -1
    console.log('tput cols:', tputCols)

    // Extract stty size
    const sttyMatch = content.match(/stty size\n(\d+)\s+(\d+)/)
    const sttyRows = sttyMatch ? parseInt(sttyMatch[1]) : -1
    const sttyCols = sttyMatch ? parseInt(sttyMatch[2]) : -1
    console.log('stty size:', sttyRows, 'rows', sttyCols, 'cols')

    // Extract $COLUMNS
    const colsMatch = content.match(/echo \$COLUMNS\n(\d+)/)
    const envCols = colsMatch ? parseInt(colsMatch[1]) : -1
    console.log('$COLUMNS:', envCols)

    // Sidebar pane size
    const sidebarCols = parseInt(paneSizeText?.split('×')[0] || '0')
    console.log('Sidebar cols:', sidebarCols)

    // Assertions
    console.log('\n=== RESIZE DIAGNOSTIC ===')
    console.log(`Container width: ${clientInfo.containerWidth}px`)
    console.log(`Sidebar pane size: ${paneSizeText}`)
    console.log(`tput cols: ${tputCols}`)
    console.log(`stty size: ${sttyRows}x${sttyCols}`)
    console.log(`$COLUMNS: ${envCols}`)
    console.log(`Match: sidebar=${sidebarCols} tput=${tputCols} stty=${sttyCols} env=${envCols}`)

    if (tputCols === 80) {
      console.log('BUG CONFIRMED: tput cols = 80 despite wide viewport')
      console.log('Resize messages reach agent but tmux ignores them')
    }

    // Soft assert: tput should be > 80 for a 1400px viewport
    expect.soft(tputCols, 'tput cols should be > 80 for wide viewport').toBeGreaterThan(80)
  })

  test('resize propagates after viewport change', async ({ page }) => {
    test.setTimeout(90_000)
    await authenticate(page)
    await ensureConnectedSession(page)
    await switchToSingleView(page)
    await selectFirstPane(page)

    // Narrow viewport
    await page.setViewportSize({ width: 800, height: 600 })
    await page.waitForTimeout(2000)
    await page.keyboard.type('tput cols', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(1500)

    let content = await readTerminalContent(page)
    const narrowMatch = content.match(/tput cols\n(\d+)/)
    const narrowCols = narrowMatch ? parseInt(narrowMatch[1]) : -1
    console.log('Narrow cols:', narrowCols)

    // Wide viewport
    await page.setViewportSize({ width: 1400, height: 800 })
    await page.waitForTimeout(2000)
    await page.keyboard.type('tput cols', { delay: 30 })
    await page.keyboard.press('Enter')
    await page.waitForTimeout(1500)

    content = await readTerminalContent(page)
    // Find the LAST tput cols output
    const wideMatches = [...content.matchAll(/tput cols\n(\d+)/g)]
    const wideCols = wideMatches.length > 0 ? parseInt(wideMatches[wideMatches.length - 1][1]) : -1
    console.log('Wide cols:', wideCols)

    await page.screenshot({ path: 'e2e/screenshots/resize-diag-03-propagation.png' })

    console.log(`\n=== RESIZE PROPAGATION: narrow=${narrowCols} wide=${wideCols} ===`)

    if (narrowCols === 80 && wideCols === 80) {
      console.log('BUG: Both return 80 — resize pipe completely broken')
    } else if (wideCols > narrowCols) {
      console.log('OK: Resize propagates correctly')
    }

    expect.soft(wideCols, 'wide viewport should have more cols than narrow').toBeGreaterThan(narrowCols)
  })
})
