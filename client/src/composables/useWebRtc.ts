/**
 * WebRTC DataChannel connection composable.
 * Browser-as-offerer with trickle ICE for fast connection:
 *   1. Browser creates DataChannel + offer, sends IMMEDIATELY (0 candidates)
 *   2. Browser trickles candidates as they arrive
 *   3. Agent receives offer, creates answer with all candidates, sends back
 *   4. ICE checking starts in parallel with browser's gathering
 */
import { encode, decode } from '@msgpack/msgpack'

export interface WebRtcConnection {
  send: (msg: Record<string, unknown>) => void
  close: () => void
  onMessage: (handler: (msg: Record<string, unknown>) => void) => void
  onClose: (handler: () => void) => void
}

export interface IceConfig {
  iceServers: Array<{
    urls: string[]
    username?: string
    credential?: string
  }>
}

export async function connectWebRtc(
  iceConfig: IceConfig,
  sendSignal: (payload: Record<string, unknown>) => void,
  onSignal: (handler: (payload: Record<string, unknown>) => void) => void,
): Promise<WebRtcConnection> {
  return new Promise((resolve, reject) => {
    console.log('[oxmux-webrtc] creating PC (browser=offerer, trickle ICE), ICE servers:', iceConfig.iceServers.length)
    const pc = new RTCPeerConnection({
      iceServers: iceConfig.iceServers,
      iceTransportPolicy: 'all',
    })

    let messageHandler: ((msg: Record<string, unknown>) => void) | null = null
    let closeHandler: (() => void) | null = null
    let dataChannel: RTCDataChannel | null = null
    let candidateCount = 0

    // Browser creates the DataChannel (browser is offerer)
    const dc = pc.createDataChannel('oxmux')
    dc.binaryType = 'arraybuffer'

    dc.onopen = () => {
      console.log('[oxmux-webrtc] DataChannel opened')
      dc.bufferedAmountLowThreshold = 65536 // 64KB
      dataChannel = dc
      resolve({
        send: (msg) => {
          if (dc.readyState !== 'open') return
          // Backpressure: skip if buffer is full (prevents SCTP overflow)
          if (dc.bufferedAmount > 1024 * 1024) {
            console.warn('[oxmux-webrtc] DC buffer full, dropping message')
            return
          }
          dc.send(encode(msg))
        },
        close: () => { dc.close(); pc.close() },
        onMessage: (h) => { messageHandler = h },
        onClose: (h) => { closeHandler = h },
      })
    }

    dc.onerror = (ev) => {
      console.error('[oxmux-webrtc] DataChannel error:', ev)
    }

    dc.onmessage = (ev: MessageEvent) => {
      try {
        messageHandler?.(decode(new Uint8Array(ev.data)) as Record<string, unknown>)
      } catch (e) {
        console.warn('[oxmux-webrtc] decode error:', e)
      }
    }

    dc.onclose = () => {
      console.log('[oxmux-webrtc] DataChannel closed')
      closeHandler?.()
    }

    pc.oniceconnectionstatechange = () => {
      console.log('[oxmux-webrtc] ICE:', pc.iceConnectionState)
    }

    pc.onconnectionstatechange = () => {
      console.log('[oxmux-webrtc] conn:', pc.connectionState)
      if (pc.connectionState === 'failed' || pc.connectionState === 'disconnected') {
        closeHandler?.()
      }
    }

    // Trickle ICE: send each candidate to agent as it arrives
    pc.addEventListener('icecandidate', (ev) => {
      if (ev.candidate) {
        candidateCount++
        console.log('[oxmux-webrtc] trickle #' + candidateCount + ':',
          ev.candidate.type, ev.candidate.protocol, ev.candidate.address || '(mDNS)')
        sendSignal({ type: 'ice', candidate: ev.candidate.toJSON() })
      } else {
        console.log('[oxmux-webrtc] gathering done:', candidateCount, 'candidates')
      }
    })

    // Handle answer from agent
    onSignal(async (payload) => {
      try {
        if (payload.type === 'answer') {
          const answerSdp = payload.sdp as string
          console.log('[oxmux-webrtc] received answer with',
            (answerSdp.match(/a=candidate/g) || []).length, 'candidates')
          await pc.setRemoteDescription(new RTCSessionDescription({
            type: 'answer', sdp: answerSdp,
          }))
          console.log('[oxmux-webrtc] remote description set — ICE checking')
        }
      } catch (e) {
        console.error('[oxmux-webrtc] error setting answer:', e)
      }
    })

    // Create and send offer IMMEDIATELY (trickle ICE — don't wait for gathering)
    ;(async () => {
      try {
        const offer = await pc.createOffer()
        await pc.setLocalDescription(offer)

        // Send offer right away with 0 candidates — candidates trickle separately
        const sdp = pc.localDescription?.sdp || ''
        console.log('[oxmux-webrtc] sending offer (trickle ICE, candidates follow)')
        sendSignal({ type: 'offer', sdp })
      } catch (e) {
        console.error('[oxmux-webrtc] error creating offer:', e)
        reject(e)
      }
    })()

    // Outer timeout
    setTimeout(() => {
      if (!dataChannel) {
        console.log('[oxmux-webrtc] timeout — ICE:', pc.iceConnectionState,
          'conn:', pc.connectionState, 'candidates:', candidateCount)
        reject(new Error('WebRTC connection timeout'))
        pc.close()
      }
    }, 30000)
  })
}

export function isWebRtcSupported(): boolean {
  return typeof RTCPeerConnection !== 'undefined'
}
