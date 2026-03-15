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
  let streamBuf = new Uint8Array(0)
  const concatBuffers = (a: Uint8Array, b: Uint8Array) => {
    const c = new Uint8Array(a.length + b.length)
    c.set(a, 0)
    c.set(b, a.length)
    return c
  }
  ;(async () => {
    try {
      while (true) {
        const { value, done } = await reader.read()
        if (done) break
        if (!value || value.length === 0) continue

        // Messages are length-prefixed: 4-byte big-endian length + msgpack
        streamBuf = concatBuffers(streamBuf, value)
        while (streamBuf.length >= 4) {
          const len = new DataView(streamBuf.buffer, streamBuf.byteOffset, 4).getUint32(0)
          if (streamBuf.length < 4 + len) break
          const msgBytes = streamBuf.slice(4, 4 + len)
          streamBuf = streamBuf.slice(4 + len)
          try {
            const msg = decode(msgBytes) as Record<string, unknown>
            messageHandler?.(msg)
          } catch (e) {
            console.warn('[oxmux-quic] decode error:', e)
          }
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
      // Length-prefix: 4-byte big-endian length + msgpack payload
      const frame = new Uint8Array(4 + encoded.length)
      new DataView(frame.buffer).setUint32(0, encoded.length)
      frame.set(new Uint8Array(encoded), 4)
      writer.write(frame).catch(() => {})
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
