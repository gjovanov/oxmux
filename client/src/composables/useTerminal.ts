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

// ── Global Terminal Registry ──────────────────────────────────────────
// Prevents duplicate terminals when switching between single/mashed views.
// Each pane gets ONE terminal instance that survives view changes.
interface TerminalEntry {
  terminal: Terminal
  fitAddon: FitAddon
  paneId: string
  attachedTo: HTMLElement | null
  resizeObserver: ResizeObserver | null
  cleanupFns: (() => void)[]
  refCount: number // how many components are using this terminal
}

const terminalRegistry = new Map<string, TerminalEntry>()

/** Dispose a terminal and remove from registry */
function disposeEntry(paneId: string) {
  const entry = terminalRegistry.get(paneId)
  if (!entry) return

  entry.resizeObserver?.disconnect()
  for (const fn of entry.cleanupFns) fn()
  const store = useTmuxStore()
  store.unsubscribePane(paneId)
  entry.terminal.dispose()
  terminalRegistry.delete(paneId)
}

/** Get current terminal dimensions for a pane */
export function getTerminalSize(paneId: string): { cols: number; rows: number } | null {
  const entry = terminalRegistry.get(paneId)
  if (!entry) return null
  return { cols: entry.terminal.cols, rows: entry.terminal.rows }
}

/** Full terminal reset after transport switch (clears parser state, modes, screen) */
export function resetTerminal(paneId: string) {
  const entry = terminalRegistry.get(paneId)
  if (entry) {
    // Full reset: clears parser state, cursor, modes, scrollback — fixes:
    // - Partial UTF-8 sequences from old transport (U+FFFD errors)
    // - Stuck alternate screen buffer from Claude Code TUI
    // - Input mode corruption causing duplicate characters
    entry.terminal.reset()
  }
}

/** Dispose all terminals (e.g., on logout) */
export function disposeAllTerminals() {
  for (const paneId of [...terminalRegistry.keys()]) {
    disposeEntry(paneId)
  }
}

export function useTerminal(
  paneId: Ref<string>,
  containerRef: Ref<HTMLElement | null>,
  isActive?: Ref<boolean>,
) {
  const store = useTmuxStore()
  const term = ref<Terminal | null>(null)

  function mount() {
    if (!containerRef.value || !paneId.value) return

    const pid = paneId.value

    // Check if terminal already exists in registry (view switch reuse)
    const existing = terminalRegistry.get(pid)
    if (existing) {
      // Reattach existing terminal to new container
      existing.refCount++
      existing.resizeObserver?.disconnect()

      // Move terminal DOM to new container
      if (existing.attachedTo !== containerRef.value) {
        containerRef.value.innerHTML = ''
        const xtermEl = existing.terminal.element
        if (xtermEl) {
          containerRef.value.appendChild(xtermEl)
        }
        existing.attachedTo = containerRef.value
      }

      // Re-setup resize observer on new container
      let resizeTimer: ReturnType<typeof setTimeout>
      existing.resizeObserver = new ResizeObserver(() => {
        clearTimeout(resizeTimer)
        resizeTimer = setTimeout(() => {
          existing.fitAddon.fit()
          store.sendResize(pid, existing.terminal.cols, existing.terminal.rows)
        }, 50)
      })
      existing.resizeObserver.observe(containerRef.value)

      // Fit to new container and send resize.
      // DO NOT sendSubscribe here — the agent's sub handler sends capture-pane
      // (plain text format) which corrupts Claude Code's TUI rendering.
      // The resize alone triggers SIGWINCH via control mode stdin, which is
      // sufficient for Claude Code to redraw at the new dimensions.
      requestAnimationFrame(() => {
        existing.fitAddon.fit()
        store.sendResize(pid, existing.terminal.cols, existing.terminal.rows)
      })

      term.value = existing.terminal
      return
    }

    // Responsive font size: mobile 11px, tablet 12px, desktop 13px
    const vw = window.innerWidth
    const fontSize = vw <= 767 ? 11 : vw <= 1023 ? 12 : 13
    const lineHeight = vw <= 767 ? 1.15 : 1.2

    const t = new Terminal({
      allowProposedApi: true,
      scrollback: 10_000,
      fastScrollModifier: 'shift',
      theme: OXMUX_THEME,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", monospace',
      fontSize,
      lineHeight,
      cursorBlink: true,
      cursorStyle: 'block',
      macOptionIsMeta: true,
    })

    const fitAddon = new FitAddon()
    const searchAddon = new SearchAddon()
    const cleanupFns: (() => void)[] = []

    t.loadAddon(fitAddon)
    t.loadAddon(searchAddon)
    t.loadAddon(new WebLinksAddon())
    t.open(containerRef.value)

    // Ctrl+C: copy selection. Ctrl+V: let browser paste natively.
    t.attachCustomKeyEventHandler((ev: KeyboardEvent) => {
      if (ev.type !== 'keydown') return true
      const isMod = ev.ctrlKey || ev.metaKey

      if (isMod && (ev.key === 'c' || ev.key === 'C')) {
        if (ev.shiftKey || t.getSelection()) {
          const selection = t.getSelection()
          if (selection) {
            navigator.clipboard.writeText(selection).catch(() => {})
            t.clearSelection()
          }
          return false
        }
        return true
      }

      if (isMod && (ev.key === 'v' || ev.key === 'V')) {
        return false
      }

      return true
    })

    // Right-click paste
    const ctxHandler = async (e: MouseEvent) => {
      e.preventDefault()
      try {
        const text = await navigator.clipboard.readText()
        if (text && pid) store.sendInput(pid, new TextEncoder().encode(text))
      } catch { /* ignore */ }
    }
    containerRef.value.addEventListener('contextmenu', ctxHandler)
    cleanupFns.push(() => containerRef.value?.removeEventListener('contextmenu', ctxHandler as EventListener))

    // I/O — only ONE handler per terminal, stored in registry
    t.onData((data: string) => {
      store.sendInput(pid, new TextEncoder().encode(data))
    })

    t.onBinary((data: string) => {
      store.sendInput(pid, Uint8Array.from(data, c => c.charCodeAt(0)))
    })

    // CRITICAL ORDER: handler → resize → subscribe
    // Agent sends: clear screen + capture-pane (with \r\n) + SIGWINCH
    store.registerPaneHandler(pid, (data: Uint8Array) => {
      t.write(data)
    })
    fitAddon.fit()
    store.sendResize(pid, t.cols, t.rows)
    store.sendSubscribe(pid)

    // Resize observer for subsequent size changes
    let resizeTimer: ReturnType<typeof setTimeout>
    const resizeObserver = new ResizeObserver(() => {
      clearTimeout(resizeTimer)
      resizeTimer = setTimeout(() => {
        fitAddon.fit()
        store.sendResize(pid, t.cols, t.rows)
      }, 50)
    })
    resizeObserver.observe(containerRef.value)

    // Second fit after layout settles (container might not have final dimensions yet)
    requestAnimationFrame(() => {
      fitAddon.fit()
      store.sendResize(pid, t.cols, t.rows)
    })

    // Don't write anything to the terminal — the agent sends a clear screen
    // followed by SIGWINCH-triggered content from the running app

    // Register in global registry
    terminalRegistry.set(pid, {
      terminal: t,
      fitAddon,
      paneId: pid,
      attachedTo: containerRef.value,
      resizeObserver,
      cleanupFns,
      refCount: 1,
    })

    term.value = t

    if (isActive?.value) {
      setTimeout(() => t.focus(), 200)
    }
  }

  function focus() {
    term.value?.focus()
  }

  function search(query: string, options = {}) {
    const entry = paneId.value ? terminalRegistry.get(paneId.value) : null
    if (entry) {
      // Use searchAddon if available
    }
  }

  function detach() {
    const pid = paneId.value
    if (!pid) return

    const entry = terminalRegistry.get(pid)
    if (entry) {
      entry.refCount--
      entry.resizeObserver?.disconnect()
      entry.resizeObserver = null
      // Don't dispose — terminal stays in registry for reuse
      // Only dispose if refCount drops to 0 AND a timeout passes
      // (in case a new component mounts for the same pane)
      if (entry.refCount <= 0) {
        setTimeout(() => {
          const current = terminalRegistry.get(pid)
          if (current && current.refCount <= 0) {
            // No one reattached — dispose
            disposeEntry(pid)
          }
        }, 500)
      }
    }
    term.value = null
  }

  onMounted(() => nextTick(() => mount()))
  onUnmounted(detach)

  if (isActive) {
    watch(isActive, (active) => {
      if (active) nextTick(() => term.value?.focus())
    })
  }

  return { term, focus, search, dispose: detach }
}
