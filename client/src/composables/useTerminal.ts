import { ref, onMounted, onUnmounted, nextTick, type Ref } from 'vue'
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
  containerRef: Ref<HTMLElement | null>
) {
  const store = useTmuxStore()
  const term = ref<Terminal | null>(null)
  const fitAddon = new FitAddon()
  const searchAddon = new SearchAddon()
  let resizeObserver: ResizeObserver | null = null

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

    // Skip WebGL addon — use default canvas renderer for reliability
    // WebGL can fail silently in headless browsers, VMs, and some GPUs

    // Delay fit to ensure container has layout dimensions
    requestAnimationFrame(() => {
      fitAddon.fit()
      console.log('[oxmux] terminal fitted:', t.cols, 'x', t.rows)
    })

    term.value = t

    // Input → server
    t.onData((data: string) => {
      store.sendInput(paneId.value, new TextEncoder().encode(data))
    })

    // Binary input (paste etc.)
    t.onBinary((data: string) => {
      const bytes = Uint8Array.from(data, c => c.charCodeAt(0))
      store.sendInput(paneId.value, bytes)
    })

    // Subscribe to pane output from server
    store.subscribePane(paneId.value, (data: Uint8Array) => {
      t.write(data)
    })

    // Resize observer — debounced 50ms
    let resizeTimer: ReturnType<typeof setTimeout>
    resizeObserver = new ResizeObserver(() => {
      clearTimeout(resizeTimer)
      resizeTimer = setTimeout(() => {
        fitAddon.fit()
        store.sendResize(paneId.value, t.cols, t.rows)
      }, 50)
    })
    resizeObserver.observe(containerRef.value)

    // Initial resize after a short delay
    setTimeout(() => {
      fitAddon.fit()
      store.sendResize(paneId.value, t.cols, t.rows)
    }, 100)

    // Write a welcome message to confirm rendering works
    t.write('\r\n\x1b[90m[oxmux] connecting to pane ' + paneId.value + '...\x1b[0m\r\n')
  }

  function focus() {
    term.value?.focus()
  }

  function search(query: string, options = {}) {
    searchAddon.findNext(query, options)
  }

  function dispose() {
    resizeObserver?.disconnect()
    store.unsubscribePane(paneId.value)
    term.value?.dispose()
    term.value = null
  }

  onMounted(() => {
    // Use nextTick to ensure the DOM is fully rendered
    nextTick(() => {
      mount()
    })
  })
  onUnmounted(dispose)

  return { term, focus, search, dispose }
}
