/**
 * WebTransport (QUIC) connection composable.
 *
 * Uses the browser's WebTransport API to connect to the oxmux server
 * via QUIC instead of WebSocket. Provides the same MessagePack protocol.
 *
 * Transport #2 (QUIC → SSH) and #4 (QUIC → Agent).
 */
import { encode, decode } from '@msgpack/msgpack'

export interface QuicConnection {
  send: (msg: Record<string, unknown>) => void
  close: () => void
  onMessage: (handler: (msg: Record<string, unknown>) => void) => void
  onClose: (handler: () => void) => void
}

/**
 * Connect to the server via WebTransport (QUIC).
 *
 * @param url - WebTransport URL, e.g. "https://oxmux.app:4433"
 * @param token - JWT auth token
 */
export async function connectQuic(url: string, token: string, certHash?: ArrayBuffer): Promise<QuicConnection> {
  // @ts-ignore — WebTransport may not be in all TS type defs
  const opts: any = {}
  if (certHash) {
    // Pin self-signed cert for P2P agent connections
    opts.serverCertificateHashes = [{
      algorithm: 'sha-256',
      value: certHash,
    }]
  }
  const transport = new WebTransport(url, opts)
  await transport.ready

  // Open a bidirectional stream
  const stream = await transport.createBidirectionalStream()
  const writer = stream.writable.getWriter()
  const reader = stream.readable.getReader()

  let messageHandler: ((msg: Record<string, unknown>) => void) | null = null
  let closeHandler: (() => void) | null = null

  // Send auth message
  const authMsg = encode({ t: 'auth', token })
  await writer.write(authMsg)

  // Read auth response (raw msgpack, no length prefix)
  const authResp = await reader.read()
  if (authResp.done) throw new Error('Connection closed during auth')
  const authResult = decode(authResp.value) as Record<string, unknown>
  if (authResult.t !== 'auth_ok') throw new Error('Auth failed: ' + JSON.stringify(authResult))

  // Start reading messages in background
  ;(async () => {
    try {
      while (true) {
        const { value, done } = await reader.read()
        if (done) break
        if (!value || value.length === 0) continue

        // Each stream read contains a complete msgpack message
        try {
          const msg = decode(value) as Record<string, unknown>
          messageHandler?.(msg)
        } catch (e) {
          console.warn('[oxmux-quic] decode error:', e)
        }
      }
    } catch (e) {
      console.warn('[oxmux-quic] read loop ended:', e)
    }
    closeHandler?.()
  })()

  return {
    send: (msg: Record<string, unknown>) => {
      const encoded = encode(msg)
      // No length prefix for sends — server reads raw from stream
      writer.write(encoded).catch(() => {})
    },
    close: () => {
      transport.close()
    },
    onMessage: (handler) => { messageHandler = handler },
    onClose: (handler) => { closeHandler = handler },
  }
}

/**
 * Check if the browser supports WebTransport.
 */
export function isWebTransportSupported(): boolean {
  return typeof (globalThis as any).WebTransport !== 'undefined'
}
