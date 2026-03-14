# CLAUDE.md — Oxmux AI Context

## Project Overview

Oxmux is a Claude Code fleet manager — a tmux web client that provides structured dashboards for managing remote Claude Code sessions. Built with Rust (server + agent) and Vue 3 (client).

## Monorepo Structure

```
oxmux/
├── server/    Rust: Axum HTTP/WS/QUIC/WebRTC server, SSH (russh), PTY, tmux control mode
├── client/    Vue 3: xterm.js terminal UI + structured Claude Code dashboard
├── agent/     Rust: lightweight binary deployed on remote hosts for P2P QUIC/WebRTC
├── e2e/       Playwright E2E tests
└── docs/      Architecture, API, UI, deployment documentation
```

## 5-Transport Architecture

All transports carry the same binary MessagePack protocol. The user always gets a tmux terminal — only the data path changes.

### Server-Relayed (SSH backend)
1. **WS → SSH** — Browser →WebSocket→ Server →SSH→ Host →tmux (default, works everywhere)
2. **QUIC → SSH** — Browser →QUIC→ Server →SSH→ Host →tmux (low latency, 0-RTT)
3. **WebRTC → SSH** — Browser →WebRTC→ Server →SSH→ Host →tmux (NAT traversal)

### Agent-Direct (P2P, no SSH)
4. **QUIC → Agent** — Browser →QUIC→ Agent →tmux (lowest latency, no server relay)
5. **WebRTC → Agent** — Browser →WebRTC→ Agent →tmux (P2P through any NAT)

### Session Configuration
```rust
pub struct SessionConfig {
    pub name: String,
    pub browser_transport: BrowserTransport,  // WS, QUIC, WebRTC
    pub backend_transport: BackendTransport,  // SSH or Agent
}
```

### TURN Credentials
- COTURN at `coturn.roomler.live` uses **shared secret HMAC-SHA1** (time-limited)
- `username = "<unix_timestamp + ttl>:<user_id>"`
- `password = base64(HMAC-SHA1(COTURN_AUTH_SECRET, username))`
- Generated server-side in `server/src/webrtc/turn.rs`
- TTL: 86400s (24h). Never expose `COTURN_AUTH_SECRET` to browser.
- ICE servers: mars, zeus, jupiter on ports 3478/5349

### Claude Code Integration
- `claude --output-format stream-json` emits JSONL
- Parser in `server/src/claude/parser.rs` → forwarded as `ServerMsg::ClaudeEvent`
- Vue renders `ClaudePane.vue` (structured) vs `TerminalPane.vue` (raw PTY)
- Mode auto-detected from process name in tmux pane

### PTY / tmux
- tmux control mode (`tmux -CC attach`) for structured state + live output streaming
- `%output` events decoded from octal escaping → broadcast per pane
- `send-keys -H` for hex-encoded input to panes
- One `broadcast::channel` per pane — all subscribed clients receive same bytes

### Protocol
- Binary MessagePack frames (not JSON)
- `server/src/ws/protocol.rs` defines `ServerMsg` and `ClientMsg` enums
- Same protocol used over WS, QUIC streams, and WebRTC DataChannels

### Auth
- JWT tokens (argon2 password hashing, 7-day expiry)
- WS: `?token=<jwt>` query param
- QUIC/WebRTC: JWT in first message
- Sessions scoped by user_id (SQLite persistence)

### Agent
- Lightweight binary deployed on remote hosts
- Manages tmux locally (no SSH needed)
- QUIC listener for server or browser connections
- Auto-deployed via SSH from server, or manually installed
- Self-signed TLS certs for QUIC

## Environment Variables

All config from `.env` (dev) or Kubernetes Secret (prod). See `.env.example`.
Critical vars: `OXMUX_JWT_SECRET`, `COTURN_AUTH_SECRET`, `COTURN_REALM`, `QUIC_CERT_PATH`, `QUIC_KEY_PATH`, `DATABASE_URL`.

## Commands

```bash
# Development
make dev          # start server (cargo watch) + client (vite) concurrently
make dev:server   # server only
make dev:client   # client only

# Testing
make test         # unit + integration tests
make test:e2e     # Playwright E2E
make test:load    # k6 load test

# Build
make build        # build server binary + client assets
make build:agent  # build oxmux-agent binary

# Docker
docker compose up              # full stack
docker compose up server       # server only
```

## Code Style (Rust)

- `anyhow::Result` for all fallible functions
- `tracing` for all logging (not `println!` or `eprintln!`)
- `tokio::sync::broadcast` for pane output fan-out
- `dashmap::DashMap` for concurrent maps (not `Mutex<HashMap>`)
- All public types `Serialize + Deserialize` via serde
- Error types: `thiserror` for library-style errors
- `async-trait` for async trait methods

## Code Style (Vue 3 / TypeScript)

- Composition API only (no Options API)
- Pinia stores for all shared state
- `useTerminal` composable wraps xterm.js per pane
- Binary MessagePack via `@msgpack/msgpack`
- Transport composables: `useWebSocket`, `useQuic`, `useWebRtc`

## Testing

- **Rust integration tests** (`server/tests/`) — real SSH to mars (94.130.141.98), id_secunet key
- **E2E tests** (`e2e/tests/`) — Playwright against https://oxmux.app, per-transport test files
- **Load tests** (`e2e/load/`) — k6 WebSocket load test
- xterm.js accessibility layer (`data-testid="terminal-accessible-output"`) for Playwright assertions
- E2E tests take screenshots at each step for visual verification

## Key Files

| Area | Files |
|------|-------|
| Transport types | `server/src/session/types.rs` |
| Transport trait | `server/src/session/transport.rs` |
| SSH backend | `server/src/session/ssh_transport.rs` |
| Session manager | `server/src/session/manager.rs` |
| WS handler | `server/src/ws/handler.rs` |
| Protocol | `server/src/ws/protocol.rs` |
| Auth | `server/src/auth/handler.rs`, `server/src/auth/jwt.rs` |
| Database | `server/src/db/repo.rs` |
| tmux parser | `server/src/tmux/control.rs` |
| Claude parser | `server/src/claude/parser.rs` |
| TURN creds | `server/src/webrtc/turn.rs` |
| Terminal UI | `client/src/composables/useTerminal.ts` |
| Stores | `client/src/stores/tmux.ts`, `client/src/stores/auth.ts` |
