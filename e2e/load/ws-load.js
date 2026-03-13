import ws from 'k6/ws'
import { check, sleep } from 'k6'
import { Counter, Trend } from 'k6/metrics'

const msgReceived = new Counter('ws_msgs_received')
const msgLatency  = new Trend('ws_msg_latency_ms', true)

export const options = {
  scenarios: {
    // 100 concurrent clients all subscribing to same pane
    concurrent_subscribers: {
      executor: 'constant-vus',
      vus: 100,
      duration: '2m',
    },
    // Ramp up to 500 clients
    ramp_up: {
      executor: 'ramping-vus',
      startVUs: 0,
      stages: [
        { duration: '30s', target: 100 },
        { duration: '60s', target: 500 },
        { duration: '30s', target: 0 },
      ],
      startTime: '3m',
    },
  },
  thresholds: {
    ws_msgs_received:    ['count>1000'],
    ws_msg_latency_ms:   ['p(95)<200'],
    ws_connecting_time:  ['p(95)<500'],
  },
}

export default function () {
  const url = `ws://${__ENV.OXMUX_HOST ?? 'localhost:8080'}/ws`

  const res = ws.connect(url, {}, function (socket) {
    socket.on('open', () => {
      // Subscribe to pane %1
      const subMsg = new Uint8Array([/* msgpack encoded {t:'sub',pane:'%1'} */
        0x83, 0xa1, 0x74, 0xa3, 0x73, 0x75, 0x62,
        0xa4, 0x70, 0x61, 0x6e, 0x65, 0xa2, 0x25, 0x31
      ])
      socket.sendBinary(subMsg.buffer)
    })

    socket.on('message', (data) => {
      msgReceived.add(1)
    })

    socket.on('binaryMessage', (data) => {
      msgReceived.add(1)
    })

    socket.on('error', (e) => {
      console.error('WS error:', e.error())
    })

    // Ping/pong latency measurement every 5s
    let pingInterval = setInterval(() => {
      const ts = Date.now()
      const pingMsg = new Uint8Array([
        0x82, 0xa1, 0x74, 0xa4, 0x70, 0x69, 0x6e, 0x67,
        0xa2, 0x74, 0x73, 0xcf,
        ...new Array(8).fill(0) // timestamp placeholder
      ])
      socket.sendBinary(pingMsg.buffer)
    }, 5000)

    socket.setTimeout(() => {
      clearInterval(pingInterval)
      socket.close()
    }, 30_000)
  })

  check(res, { 'connected': (r) => r && r.status === 101 })
  sleep(1)
}
