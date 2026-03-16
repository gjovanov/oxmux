/**
 * WebRTC DataChannel connection composable.
 * Uses full vanilla ICE (no trickle) — all candidates embedded in SDP.
 * Agent creates offer, browser creates answer.
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

/**
 * Connect via WebRTC. Agent sends offer, browser sends answer.
 * Full vanilla ICE: both sides wait for gathering complete before sending SDP.
 */
export async function connectWebRtc(
  iceConfig: IceConfig,
  sendSignal: (payload: Record<string, unknown>) => void,
  onSignal: (handler: (payload: Record<string, unknown>) => void) => void,
): Promise<WebRtcConnection> {
  return new Promise((resolve, reject) => {
    const servers = [
      { urls: ['stun:stun.l.google.com:19302'] },
      ...iceConfig.iceServers,
    ]
    console.log('[oxmux-webrtc] creating PC, ICE servers:', servers.length)
    const pc = new RTCPeerConnection({ iceServers: servers })

    let messageHandler: ((msg: Record<string, unknown>) => void) | null = null
    let closeHandler: (() => void) | null = null
    let dataChannel: RTCDataChannel | null = null

    // Browser receives DataChannel from agent
    pc.ondatachannel = (ev) => {
      const dc = ev.channel
      dc.binaryType = 'arraybuffer'
      console.log('[oxmux-webrtc] DataChannel received:', dc.label)

      dc.onopen = () => {
        console.log('[oxmux-webrtc] DataChannel opened')
        dataChannel = dc
        resolve({
          send: (msg) => { if (dc.readyState === 'open') dc.send(encode(msg)) },
          close: () => { dc.close(); pc.close() },
          onMessage: (h) => { messageHandler = h },
          onClose: (h) => { closeHandler = h },
        })
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

    // Helper: wait for ICE gathering complete
    const waitForGathering = (): Promise<void> => new Promise((res) => {
      if (pc.iceGatheringState === 'complete') return res()
      const check = () => { if (pc.iceGatheringState === 'complete') res() }
      pc.addEventListener('icegatheringstatechange', check)
      setTimeout(res, 15000) // 15s max wait
    })

    // Handle agent's offer → create answer with all candidates
    onSignal(async (payload) => {
      try {
        if (payload.type === 'offer') {
          console.log('[oxmux-webrtc] received offer, creating answer...')

          await pc.setRemoteDescription(new RTCSessionDescription({
            type: 'offer',
            sdp: payload.sdp as string,
          }))

          const answer = await pc.createAnswer()
          await pc.setLocalDescription(answer)

          // Wait for ICE gathering complete (vanilla ICE)
          console.log('[oxmux-webrtc] waiting for ICE gathering...')
          await waitForGathering()

          const sdp = pc.localDescription?.sdp || ''
          const candidates = (sdp.match(/a=candidate/g) || []).length
          console.log('[oxmux-webrtc] sending answer with', candidates, 'candidates')

          // Send answer with all candidates embedded in SDP
          sendSignal({ type: 'answer', sdp })
        }
      } catch (e) {
        console.error('[oxmux-webrtc] error:', e)
      }
    })

    // Tell agent we're ready
    console.log('[oxmux-webrtc] sending ready')
    sendSignal({ type: 'ready' })

    // Timeout
    setTimeout(() => {
      if (!dataChannel) {
        console.log('[oxmux-webrtc] timeout — ICE:', pc.iceConnectionState, 'conn:', pc.connectionState)
        reject(new Error('WebRTC connection timeout'))
        pc.close()
      }
    }, 60000)
  })
}

export function isWebRtcSupported(): boolean {
  return typeof RTCPeerConnection !== 'undefined'
}
