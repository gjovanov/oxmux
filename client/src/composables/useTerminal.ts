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
  lastSentCols: number
  lastSentRows: number
  resizeSettling: boolean // true while waiting for SIGWINCH redraw data after resize
  resizeIdleTimer: ReturnType<typeof setTimeout> | undefined
  reattachGuard: boolean // suppress ResizeObserver during view switch (VS Code approach)
}

const terminalRegistry = new Map<string, TerminalEntry>()

/** Dispose a terminal and remove from registry */
function disposeEntry(paneId: string) {
  const entry = terminalRegistry.get(paneId)
  if (!entry) return

  clearTimeout(entry.resizeIdleTimer)
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

/** Send resize only if terminal dimensions actually changed */
function sendResizeIfChanged(entry: TerminalEntry, store: ReturnType<typeof useTmuxStore>): boolean {
  const { cols, rows } = entry.terminal
  if (cols === entry.lastSentCols && rows === entry.lastSentRows) return false
  entry.lastSentCols = cols
  entry.lastSentRows = rows
  store.sendResize(entry.paneId, cols, rows)
  return true
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

      // ResizeObserver for subsequent size changes (sidebar drag, browser resize)
      let resizeTimer: ReturnType<typeof setTimeout>
      existing.resizeObserver = new ResizeObserver(() => {
        clearTimeout(resizeTimer)
        resizeTimer = setTimeout(() => {
          existing.fitAddon.fit()
          if (sendResizeIfChanged(existing, store)) {
            existing.resizeSettling = true
            clearTimeout(existing.resizeIdleTimer)
            existing.resizeIdleTimer = setTimeout(() => {
              existing.resizeSettling = false
            }, 2000)
          }
        }, 50)
      })
      existing.resizeObserver.observe(containerRef.value)

      // After DOM settles, re-subscribe + resize to force SIGWINCH redraw.
      // Re-subscribe triggers the agent to send SIGWINCH (via run-shell),
      // ensuring Claude Code redraws its TUI at the ❯ prompt — even when
      // terminal dimensions haven't changed between views.
      // For the server path, re-subscribe creates a fresh broadcast receiver.
      requestAnimationFrame(() => {
        existing.fitAddon.fit()
        // Subscribe first (agent registers pane + fires SIGWINCH)
        store.sendSubscribe(pid)
        // Always send resize (bypass dedup) — if dimensions differ from
        // last resize, tmux sends a natural SIGWINCH at the new size
        const { cols, rows } = existing.terminal
        existing.lastSentCols = cols
        existing.lastSentRows = rows
        store.sendResize(pid, cols, rows)
        // Enable auto-scroll for SIGWINCH redraw data
        existing.resizeSettling = true
        clearTimeout(existing.resizeIdleTimer)
        existing.resizeIdleTimer = setTimeout(() => {
          existing.resizeSettling = false
        }, 2000)
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

    // CRITICAL ORDER: handler → subscribe → resize
    // Subscribe MUST come before resize so the agent's subscribed_panes set
    // is populated before the resize-triggered SIGWINCH redraw data arrives.
    // Without this, the agent discards the initial redraw output.
    store.registerPaneHandler(pid, (data: Uint8Array) => {
      t.write(data)
      // During resize settling, keep the viewport pinned to the bottom
      // so multi-chunk SIGWINCH redraws remain anchored at the ❯ prompt.
      const entry = terminalRegistry.get(pid)
      if (entry?.resizeSettling) {
        t.scrollToBottom()
        clearTimeout(entry.resizeIdleTimer)
        entry.resizeIdleTimer = setTimeout(() => {
          entry.resizeSettling = false
        }, 200)
      }
    })
    fitAddon.fit()
    store.sendSubscribe(pid)
    store.sendResize(pid, t.cols, t.rows)

    // Resize observer for subsequent size changes (50ms debounce — with
    // async input on the agent, the output channel drains continuously,
    // so small debounce gives snappy visual feedback without corruption)
    let resizeTimer: ReturnType<typeof setTimeout>
    const resizeObserver = new ResizeObserver(() => {
      clearTimeout(resizeTimer)
      resizeTimer = setTimeout(() => {
        fitAddon.fit()
        const entry = terminalRegistry.get(pid)
        if (entry && sendResizeIfChanged(entry, store)) {
          entry.resizeSettling = true
          clearTimeout(entry.resizeIdleTimer)
          entry.resizeIdleTimer = setTimeout(() => {
            entry.resizeSettling = false
          }, 2000)
        }
      }, 50)
    })
    resizeObserver.observe(containerRef.value)

    // Second fit after layout settles (container might not have final dimensions yet)
    requestAnimationFrame(() => {
      fitAddon.fit()
      const entry = terminalRegistry.get(pid)
      if (entry) {
        sendResizeIfChanged(entry, store)
        // Enable scroll-to-bottom for initial SIGWINCH redraw data.
        // The resize + subscribe just sent will trigger a redraw from the
        // remote; auto-scroll ensures the ❯ prompt is visible.
        entry.resizeSettling = true
        entry.resizeIdleTimer = setTimeout(() => {
          entry.resizeSettling = false
        }, 2000)
      }
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
      lastSentCols: t.cols,
      lastSentRows: t.rows,
      resizeSettling: false,
      resizeIdleTimer: undefined,
      reattachGuard: false,
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
      // Only disconnect resizeObserver if no other component is using this terminal.
      // If refCount > 0, another component already reattached and created a new observer.
      // Disconnecting here would kill the NEW observer.
      if (entry.refCount <= 0) {
        entry.resizeObserver?.disconnect()
        entry.resizeObserver = null
      }
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
