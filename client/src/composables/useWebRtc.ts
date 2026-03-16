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
/**
 * Connect via WebRTC following the parakeet-rs pattern:
 * Agent creates offer → browser creates answer.
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
    console.log('[oxmux-webrtc] creating PC with ICE servers:', servers.length)
    const pc = new RTCPeerConnection({ iceServers: servers })

    // Add a dummy audio transceiver to force Chrome to gather ICE candidates
    // (DataChannel-only offers may not trigger ICE gathering in some Chrome versions)
    try {
      pc.addTransceiver('audio', { direction: 'recvonly' })
      console.log('[oxmux-webrtc] added dummy audio transceiver for ICE')
    } catch (e) {
      console.warn('[oxmux-webrtc] could not add transceiver:', e)
    }

    let messageHandler: ((msg: Record<string, unknown>) => void) | null = null
    let closeHandler: (() => void) | null = null
    let dataChannel: RTCDataChannel | null = null

    // Agent creates the DataChannel — browser receives it via ondatachannel
    pc.ondatachannel = (ev) => {
      const dc = ev.channel
      dc.binaryType = 'arraybuffer'
      console.log('[oxmux-webrtc] DataChannel received:', dc.label)

      dc.onopen = () => {
        console.log('[oxmux-webrtc] DataChannel opened')
        dataChannel = dc
        resolve({
          send: (msg: Record<string, unknown>) => {
            if (dc.readyState === 'open') dc.send(encode(msg))
          },
          close: () => { dc.close(); pc.close() },
          onMessage: (handler) => { messageHandler = handler },
          onClose: (handler) => { closeHandler = handler },
        })
      }

      dc.onmessage = (ev: MessageEvent) => {
        try {
          const msg = decode(new Uint8Array(ev.data)) as Record<string, unknown>
          messageHandler?.(msg)
        } catch (e) {
          console.warn('[oxmux-webrtc] decode error:', e)
        }
      }

      dc.onclose = () => {
        console.log('[oxmux-webrtc] DataChannel closed')
        closeHandler?.()
      }
    }

    // ICE candidates — send to agent via signaling
    pc.onicecandidate = (ev) => {
      if (ev.candidate) {
        console.log('[oxmux-webrtc] sending browser ICE:', ev.candidate.candidate.slice(0, 60))
        sendSignal({
          type: 'ice_candidate',
          candidate: ev.candidate.toJSON(),
        })
      }
    }

    pc.oniceconnectionstatechange = () => {
      console.log('[oxmux-webrtc] ICE state:', pc.iceConnectionState)
    }

    pc.onconnectionstatechange = () => {
      console.log('[oxmux-webrtc] connection:', pc.connectionState)
      if (pc.connectionState === 'failed' || pc.connectionState === 'disconnected') {
        closeHandler?.()
      }
    }

    // Queue ICE candidates until remote description is set
    let remoteDescSet = false
    const pendingCandidates: any[] = []

    onSignal(async (payload) => {
      try {
        if (payload.type === 'offer') {
          const sdp = payload.sdp as string
          console.log('[oxmux-webrtc] received offer from agent, SDP lines:', sdp.split('\r\n').length)
          console.log('[oxmux-webrtc] offer fingerprint:', sdp.match(/a=fingerprint:.*/)?.[0])
          console.log('[oxmux-webrtc] offer ice-ufrag:', sdp.match(/a=ice-ufrag:.*/)?.[0])
          console.log('[oxmux-webrtc] offer sctp:', sdp.match(/m=application.*/)?.[0])
          console.log('[oxmux-webrtc] FULL OFFER SDP:\n' + sdp)
          await pc.setRemoteDescription(new RTCSessionDescription({
            type: 'offer',
            sdp,
          }))
          console.log('[oxmux-webrtc] remote desc set OK, signalingState:', pc.signalingState)

          const answer = await pc.createAnswer()
          await pc.setLocalDescription(answer)
          console.log('[oxmux-webrtc] local desc (answer) set, signalingState:', pc.signalingState)

          remoteDescSet = true

          // Process queued candidates AFTER both descriptions are set
          for (const c of pendingCandidates) {
            try {
              await pc.addIceCandidate(new RTCIceCandidate(c))
              console.log('[oxmux-webrtc] added queued candidate:', c.candidate?.slice(0, 50))
            } catch (e) {
              console.error('[oxmux-webrtc] failed to add candidate:', e, c)
            }
          }
          console.log('[oxmux-webrtc] processed', pendingCandidates.length, 'queued candidates')
          pendingCandidates.length = 0
          const answerSdp = pc.localDescription?.sdp || ''
          console.log('[oxmux-webrtc] answer created, signalingState:', pc.signalingState,
            'localCandidates:', (answerSdp.match(/a=candidate/g) || []).length)
          console.log('[oxmux-webrtc] answer sctp:', answerSdp.match(/m=application.*/)?.[0])
          sendSignal({ type: 'answer', sdp: answerSdp })

          // Check state after 3s
          setTimeout(() => {
            console.log('[oxmux-webrtc] 3s: ICE:', pc.iceConnectionState,
              'gathering:', pc.iceGatheringState, 'conn:', pc.connectionState)
          }, 3000)
        } else if (payload.type === 'ice_candidate') {
          const c = payload.candidate as any
          const candidate = typeof c === 'string'
            ? { candidate: c, sdpMid: '0', sdpMLineIndex: 0 }
            : c
          if (remoteDescSet) {
            await pc.addIceCandidate(new RTCIceCandidate(candidate))
          } else {
            pendingCandidates.push(candidate)
          }
        }
      } catch (e) {
        console.error('[oxmux-webrtc] signaling error:', e)
      }
    })

    // Tell agent we're ready — agent will create offer
    console.log('[oxmux-webrtc] sending ready signal')
    sendSignal({ type: 'ready' })

    // Timeout (60s for DTLS + ICE to complete)
    setTimeout(() => {
      if (!dataChannel) {
        console.log('[oxmux-webrtc] timeout — final ICE:', pc.iceConnectionState, 'conn:', pc.connectionState)
        reject(new Error('WebRTC connection timeout'))
        pc.close()
      }
    }, 60000)
  })
}

/**
 * Check if the browser supports WebRTC DataChannels.
 */
export function isWebRtcSupported(): boolean {
  return typeof RTCPeerConnection !== 'undefined'
}
