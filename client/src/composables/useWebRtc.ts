/**
 * WebRTC DataChannel connection composable.
 *
 * Uses RTCPeerConnection to connect to the oxmux server (Transport #3)
 * or directly to an oxmux-agent (Transport #5).
 *
 * Signaling is done via the existing WebSocket connection.
 * Once the DataChannel is established, it carries the same MessagePack protocol.
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
 * Create a WebRTC DataChannel connection via signaling.
 *
 * @param iceConfig - ICE servers (STUN/TURN) from /api/ice-config
 * @param sendSignal - Function to send signaling messages via WS
 * @param onSignal - Register handler for incoming signaling messages via WS
 */
export async function connectWebRtc(
  iceConfig: IceConfig,
  sendSignal: (payload: Record<string, unknown>) => void,
  onSignal: (handler: (payload: Record<string, unknown>) => void) => void,
): Promise<WebRtcConnection> {
  return new Promise((resolve, reject) => {
    const pc = new RTCPeerConnection({
      iceServers: iceConfig.iceServers.map(s => ({
        urls: s.urls,
        username: s.username,
        credential: s.credential,
      })),
    })

    let messageHandler: ((msg: Record<string, unknown>) => void) | null = null
    let closeHandler: (() => void) | null = null
    let dataChannel: RTCDataChannel | null = null

    // Create DataChannel
    const dc = pc.createDataChannel('oxmux', {
      ordered: true,
      protocol: 'msgpack',
    })

    dc.binaryType = 'arraybuffer'

    dc.onopen = () => {
      console.log('[oxmux-webrtc] DataChannel opened')
      dataChannel = dc
      resolve({
        send: (msg: Record<string, unknown>) => {
          if (dc.readyState === 'open') {
            dc.send(encode(msg))
          }
        },
        close: () => {
          dc.close()
          pc.close()
        },
        onMessage: (handler) => { messageHandler = handler },
        onClose: (handler) => { closeHandler = handler },
      })
    }

    dc.onmessage = (ev: MessageEvent) => {
      try {
        const data = new Uint8Array(ev.data)
        const msg = decode(data) as Record<string, unknown>
        messageHandler?.(msg)
      } catch (e) {
        console.warn('[oxmux-webrtc] decode error:', e)
      }
    }

    dc.onclose = () => {
      console.log('[oxmux-webrtc] DataChannel closed')
      closeHandler?.()
    }

    dc.onerror = (ev) => {
      console.error('[oxmux-webrtc] DataChannel error:', ev)
    }

    // ICE candidate handling
    pc.onicecandidate = (ev) => {
      if (ev.candidate) {
        console.log('[oxmux-webrtc] sending browser ICE candidate:', ev.candidate.candidate)
        sendSignal({
          type: 'ice_candidate',
          candidate: ev.candidate.candidate,
          sdp_mid: ev.candidate.sdpMid,
          sdp_mline_index: ev.candidate.sdpMLineIndex,
        })
      } else {
        console.log('[oxmux-webrtc] ICE gathering complete')
      }
    }

    pc.oniceconnectionstatechange = () => {
      console.log('[oxmux-webrtc] ICE connection state:', pc.iceConnectionState)
    }

    pc.onicegatheringstatechange = () => {
      console.log('[oxmux-webrtc] ICE gathering state:', pc.iceGatheringState)
    }

    pc.onconnectionstatechange = () => {
      console.log('[oxmux-webrtc] connection state:', pc.connectionState)
      if (pc.connectionState === 'failed' || pc.connectionState === 'disconnected') {
        closeHandler?.()
      }
    }

    // Handle incoming signaling messages
    onSignal(async (payload) => {
      console.log('[oxmux-webrtc] signal received:', payload.type, payload)
      if (payload.type === 'answer') {
        await pc.setRemoteDescription(new RTCSessionDescription({
          type: 'answer',
          sdp: payload.sdp as string,
        }))
      } else if (payload.type === 'ice_candidate') {
        await pc.addIceCandidate(new RTCIceCandidate({
          candidate: payload.candidate as string,
          sdpMid: payload.sdp_mid as string | undefined,
          sdpMLineIndex: payload.sdp_mline_index as number | undefined,
        }))
      }
    })

    // Create and send offer
    pc.createOffer()
      .then(offer => pc.setLocalDescription(offer))
      .then(() => {
        sendSignal({
          type: 'offer',
          sdp: pc.localDescription?.sdp,
        })
      })
      .catch(reject)

    // Timeout
    setTimeout(() => {
      if (!dataChannel) {
        reject(new Error('WebRTC connection timeout'))
        pc.close()
      }
    }, 30000)
  })
}

/**
 * Check if the browser supports WebRTC DataChannels.
 */
export function isWebRtcSupported(): boolean {
  return typeof RTCPeerConnection !== 'undefined'
}
