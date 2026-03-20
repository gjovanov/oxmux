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

    // ── DIAGNOSTIC: log every key event and paste event ──────────────
    t.attachCustomKeyEventHandler((ev: KeyboardEvent) => {
      if (ev.type === 'keydown') {
        const isMod = ev.ctrlKey || ev.metaKey
        // Log all modifier keys and arrow keys
        if (isMod || ev.key.startsWith('Arrow')) {
          console.log('[oxmux-key]', ev.type, ev.key, 'ctrl:', ev.ctrlKey, 'meta:', ev.metaKey, 'shift:', ev.shiftKey)
        }

        // Ctrl+C: copy selection
        if (isMod && (ev.key === 'c' || ev.key === 'C')) {
          if (ev.shiftKey || t.getSelection()) {
            const selection = t.getSelection()
            console.log('[oxmux-copy] selection:', selection?.slice(0, 50))
            if (selection) {
              navigator.clipboard.writeText(selection).catch(e => console.warn('[oxmux-copy] failed:', e))
              t.clearSelection()
            }
            return false
          }
          console.log('[oxmux-key] Ctrl+C → SIGINT (no selection)')
          return true
        }

        // Ctrl+V: paste
        if (isMod && (ev.key === 'v' || ev.key === 'V')) {
          console.log('[oxmux-paste] Ctrl+V intercepted, returning false to let browser paste')
          return false
        }
      }
      return true
    })

    // Log what onData receives (first 20 chars, hex for control chars)
    const origOnData = t.onData((data: string) => {
      const hex = Array.from(data).map(c => {
        const code = c.charCodeAt(0)
        return code < 32 ? `\\x${code.toString(16).padStart(2, '0')}` : c
      }).join('')
      if (data.length <= 3) {
        console.log('[oxmux-onData]', JSON.stringify(data), '→ hex:', hex)
      }
      store.sendInput(paneId.value, new TextEncoder().encode(data))
    })

    // Listen for paste events everywhere
    const xtermEl = containerRef.value.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement | null
    console.log('[oxmux-init] xterm textarea found:', !!xtermEl)

    const handlePaste = (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData('text/plain')
      console.log('[oxmux-paste] paste event fired on', (e.target as HTMLElement)?.className, 'text:', text?.slice(0, 50), 'clipboardData:', !!e.clipboardData)
      if (text && paneId.value) {
        e.preventDefault()
        e.stopPropagation()
        store.sendInput(paneId.value, new TextEncoder().encode(text))
        return
      }
      // Fallback: clipboard API
      navigator.clipboard.readText()
        .then(clipText => {
          console.log('[oxmux-paste] clipboard API returned:', clipText?.slice(0, 50))
          if (clipText && paneId.value) {
            store.sendInput(paneId.value, new TextEncoder().encode(clipText))
          }
        })
        .catch(err => console.warn('[oxmux-paste] clipboard API failed:', err))
      e.preventDefault()
    }

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
        console.log('[oxmux-paste] right-click clipboard:', text?.slice(0, 50))
        if (text && paneId.value) {
          store.sendInput(paneId.value, new TextEncoder().encode(text))
        }
      } catch (err) { console.warn('[oxmux-paste] right-click failed:', err) }
    }
    containerRef.value.addEventListener('contextmenu', ctxHandler)
    cleanupFns.push(() => containerRef.value?.removeEventListener('contextmenu', ctxHandler as EventListener))

    // ── Layout ────────────────────────────────────────────────────────
    requestAnimationFrame(() => {
      fitAddon.fit()
      console.log('[oxmux] terminal fitted:', t.cols, 'x', t.rows)
    })

    term.value = t

    // ── I/O (onData is registered above with logging) ──────────────
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
