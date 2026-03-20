import { ref, onMounted, onUnmounted, nextTick, watch, type Ref } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { SearchAddon } from '@xterm/addon-search'
import { useTmuxStore } from '@/stores/tmux'

const OXMUX_THEME = {
  background:  '#1e1e2e',
  foreground:  '#cdd6f4',
  cursor:      '#f5c2e7',
  black:       '#45475a',
  red:         '#f38ba8',
  green:       '#a6e3a1',
  yellow:      '#f9e2af',
  blue:        '#89b4fa',
  magenta:     '#f5c2e7',
  cyan:        '#94e2d5',
  white:       '#bac2de',
  brightBlack: '#585b70',
  brightWhite: '#a6adc8',
}

export function useTerminal(
  paneId: Ref<string>,
  containerRef: Ref<HTMLElement | null>,
  isActive?: Ref<boolean>,
) {
  const store = useTmuxStore()
  const term = ref<Terminal | null>(null)
  const fitAddon = new FitAddon()
  const searchAddon = new SearchAddon()
  let resizeObserver: ResizeObserver | null = null
  let cleanupFns: (() => void)[] = []

  function mount() {
    if (!containerRef.value || !paneId.value) {
      console.warn('[oxmux] terminal mount skipped: no container or paneId')
      return
    }

    console.log('[oxmux] mounting terminal for pane', paneId.value,
      'container:', containerRef.value.offsetWidth, 'x', containerRef.value.offsetHeight)

    const t = new Terminal({
      allowProposedApi: true,
      scrollback: 10_000,
      fastScrollModifier: 'shift',
      theme: OXMUX_THEME,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", monospace',
      fontSize: 13,
      lineHeight: 1.2,
      cursorBlink: true,
      cursorStyle: 'block',
      macOptionIsMeta: true,
    })

    t.loadAddon(fitAddon)
    t.loadAddon(searchAddon)
    t.loadAddon(new WebLinksAddon())
    t.open(containerRef.value)

    // ── Clipboard: Ctrl+C (copy) and Ctrl+V (paste) ──────────────────
    t.attachCustomKeyEventHandler((ev: KeyboardEvent) => {
      if (ev.type !== 'keydown') return true
      const isMod = ev.ctrlKey || ev.metaKey

      // Ctrl+C: copy selection OR send SIGINT
      if (isMod && (ev.key === 'c' || ev.key === 'C')) {
        if (ev.shiftKey || t.getSelection()) {
          const selection = t.getSelection()
          if (selection) {
            navigator.clipboard.writeText(selection).catch(() => {})
            t.clearSelection()
          }
          return false
        }
        return true // no selection → \x03 SIGINT
      }

      // Ctrl+V: paste — return false to prevent xterm sending \x16
      // The native browser paste event will fire on xterm's textarea
      if (isMod && (ev.key === 'v' || ev.key === 'V')) {
        return false
      }

      return true
    })

    // Listen for native paste event on xterm's textarea
    // When attachCustomKeyEventHandler returns false, xterm does NOT call
    // preventDefault(), so the browser fires the paste event natively.
    const xtermEl = containerRef.value.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement | null

    const handlePaste = (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData('text/plain')
      if (text && paneId.value) {
        e.preventDefault()
        e.stopPropagation()
        store.sendInput(paneId.value, new TextEncoder().encode(text))
        return
      }
      // If clipboardData is empty (synthetic event), try clipboard API as fallback
      navigator.clipboard.readText()
        .then(clipText => {
          if (clipText && paneId.value) {
            store.sendInput(paneId.value, new TextEncoder().encode(clipText))
          }
        })
        .catch(() => {})
      e.preventDefault()
    }

    // Attach to xterm's textarea (primary) and container (fallback)
    if (xtermEl) {
      xtermEl.addEventListener('paste', handlePaste)
      cleanupFns.push(() => xtermEl.removeEventListener('paste', handlePaste))
    }
    containerRef.value.addEventListener('paste', handlePaste)
    cleanupFns.push(() => containerRef.value?.removeEventListener('paste', handlePaste))

    // Right-click paste
    const ctxHandler = async (e: MouseEvent) => {
      e.preventDefault()
      try {
        const text = await navigator.clipboard.readText()
        if (text && paneId.value) {
          store.sendInput(paneId.value, new TextEncoder().encode(text))
        }
      } catch { /* ignore */ }
    }
    containerRef.value.addEventListener('contextmenu', ctxHandler)
    cleanupFns.push(() => containerRef.value?.removeEventListener('contextmenu', ctxHandler as EventListener))

    // ── Layout ────────────────────────────────────────────────────────
    requestAnimationFrame(() => {
      fitAddon.fit()
      console.log('[oxmux] terminal fitted:', t.cols, 'x', t.rows)
    })

    term.value = t

    // ── I/O ───────────────────────────────────────────────────────────
    t.onData((data: string) => {
      store.sendInput(paneId.value, new TextEncoder().encode(data))
    })

    t.onBinary((data: string) => {
      const bytes = Uint8Array.from(data, c => c.charCodeAt(0))
      store.sendInput(paneId.value, bytes)
    })

    store.subscribePane(paneId.value, (data: Uint8Array) => {
      t.write(data)
    })

    // ── Resize ────────────────────────────────────────────────────────
    let resizeTimer: ReturnType<typeof setTimeout>
    resizeObserver = new ResizeObserver(() => {
      clearTimeout(resizeTimer)
      resizeTimer = setTimeout(() => {
        fitAddon.fit()
        store.sendResize(paneId.value, t.cols, t.rows)
      }, 50)
    })
    resizeObserver.observe(containerRef.value)

    setTimeout(() => {
      fitAddon.fit()
      store.sendResize(paneId.value, t.cols, t.rows)
    }, 100)

    // ── Welcome + auto-focus ──────────────────────────────────────────
    t.write('\r\n\x1b[90m[oxmux] connecting to pane ' + paneId.value + '...\x1b[0m\r\n')

    // Auto-focus when terminal is active
    if (isActive?.value) {
      setTimeout(() => t.focus(), 200)
    }
  }

  function focus() {
    term.value?.focus()
  }

  function search(query: string, options = {}) {
    searchAddon.findNext(query, options)
  }

  function dispose() {
    resizeObserver?.disconnect()
    for (const fn of cleanupFns) fn()
    cleanupFns = []
    store.unsubscribePane(paneId.value)
    term.value?.dispose()
    term.value = null
  }

  onMounted(() => {
    nextTick(() => mount())
  })
  onUnmounted(dispose)

  // Auto-focus when isActive changes to true
  if (isActive) {
    watch(isActive, (active) => {
      if (active) {
        nextTick(() => term.value?.focus())
      }
    })
  }

  return { term, focus, search, dispose }
}
