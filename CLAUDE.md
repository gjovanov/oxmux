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

## Architecture

### Dual-Transport Model

WS is the **control plane** (always alive): session CRUD, agent management, transport upgrades.
P2P (QUIC/WebRTC) is the **data plane** (optional): pane I/O, subscribe, resize.

When P2P is active, WS filters out pane output (agent sends via P2P). When P2P drops, panes auto-resubscribe via WS for seamless SSH fallback.

### 5-Transport Architecture

All transports carry the same binary MessagePack protocol. The user always gets a tmux terminal — only the data path changes.

#### Server-Relayed (SSH backend)
1. **WS → SSH** — Browser →WebSocket→ Server →SSH→ Host →tmux (default, works everywhere)
2. **QUIC → SSH** — Browser →QUIC→ Server →SSH→ Host →tmux (low latency, 0-RTT)
3. **WebRTC → SSH** — Browser →WebRTC→ Server →SSH→ Host →tmux (NAT traversal)

#### Agent-Direct (P2P, no SSH)
4. **QUIC → Agent** — Browser →QUIC→ Agent →tmux (lowest latency, no server relay)
5. **WebRTC → Agent** — Browser →WebRTC→ Agent →tmux (P2P through any NAT)

### WebRTC P2P Flow (Browser-as-Offerer)

1. Browser creates `RTCPeerConnection` + `createDataChannel('oxmux')` + `createOffer()`
2. Browser trickles ICE candidates via QUIC signaling channel
3. Agent receives offer → `set_remote_description` → `create_answer` (vanilla ICE, all candidates embedded)
4. Browser sets remote answer → ICE connectivity checks
5. DataChannel opens on both sides → terminal I/O flows over DataChannel
6. Output: agent's tmux control mode → `%output` events → DataChannel → browser xterm.js
7. Input: browser xterm.js `onData` → DataChannel → agent `tmux send-keys -H`

### Multi-Session Architecture

- **Qualified Pane IDs**: `{sessionId}::{paneId}` (e.g., `abc-123::%0`) — globally unique across hosts
- **Session Trees**: `Map<sessionId, TmuxSessionInfo[]>` — per-session tmux state
- **P2P Connection Registry**: `Map<sessionId, P2PConnection>` — independent P2P per session
- **Mashed View**: NxN grid of terminals from multiple sessions simultaneously

### Terminal Registry

Global `Map<paneId, TerminalEntry>` survives view switches (single ↔ mashed). When a component mounts for a pane that already has a terminal, it reattaches the DOM instead of creating a new one. Prevents duplicate event handlers.

### TURN Credentials
- COTURN at `coturn.roomler.live` uses **shared secret HMAC-SHA1** (time-limited)
- `username = "<unix_timestamp + ttl>:<user_id>"`
- `password = base64(HMAC-SHA1(COTURN_AUTH_SECRET, username))`
- Generated server-side in `server/src/webrtc/turn.rs`
- TTL: 86400s (24h). Never expose `COTURN_AUTH_SECRET` to browser
- TURNS via `turns:coturn.roomler.live:443?transport=tcp` (valid TLS cert)

### Claude Code Integration
- `claude --output-format stream-json` emits JSONL
- Parser in `server/src/claude/parser.rs` → forwarded as `ServerMsg::ClaudeEvent`
- Vue renders `ClaudePane.vue` (structured) vs `TerminalPane.vue` (raw PTY)
- Mode auto-detected from process name in tmux pane

### PTY / tmux
- tmux control mode (`tmux -CC attach`) for structured state + live output streaming
- `%output` events decoded from octal escaping → broadcast per pane
- `send-keys -H` for hex-encoded input (each byte as separate arg)
- `window-size latest` + `aggressive-resize on` for proper terminal sizing
- One `broadcast::channel` per pane — all subscribed clients receive same bytes

### Protocol
- Binary MessagePack frames (not JSON)
- `server/src/ws/protocol.rs` defines `ServerMsg` and `ClientMsg` enums
- Same protocol used over WS, QUIC streams, and WebRTC DataChannels
- Optional `session_id` on pane messages for multi-session disambiguation

### Auth
- JWT tokens (argon2 password hashing, 7-day expiry)
- WS: `?token=<jwt>` query param
- QUIC/WebRTC: JWT in first message
- Sessions scoped by user_id (SQLite persistence)
- Ephemeral key upload for SSH auth (in-memory, per-session)

### Agent
- Lightweight musl-static binary deployed on remote hosts
- Manages tmux locally (no SSH needed)
- QUIC/WebTransport listener (wtransport) for browser connections
- WebRTC answerer (webrtc-rs v0.17) with SettingEngine NAT 1:1 IP mapping
- Auto-deployed via SSH from server (SCP + systemd)
- Per-host DNS: `agent-mars.oxmux.app`, `agent-zeus.oxmux.app` (wildcard TLS cert)
- `PUBLIC_IP` env var for ICE candidate generation

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
cd client && npx vitest run  # 30 unit tests (paneId, store, mashedView, terminal)
make test:e2e     # Playwright E2E (paste, resize, mashed view)
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
- `tmux send-keys -H` with each hex byte as separate `cmd.arg()`

## Code Style (Vue 3 / TypeScript)

- Composition API only (no Options API)
- Pinia stores for all shared state
- `useTerminal` composable with global terminal registry
- Binary MessagePack via `@msgpack/msgpack`
- Transport composables: `useQuic`, `useWebRtc`
- Qualified pane IDs: `qualifyPaneId()` / `parseQualifiedPaneId()`
- Vitest for unit tests, Playwright for E2E

## Testing

- **Unit tests** (`client/src/__tests__/`) — 30 tests: paneId, store, mashedView, terminal
- **Rust tests** (`server/tests/`) — SSH session integration tests
- **E2E tests** (`e2e/tests/`) — Playwright: paste, resize, mashed view, transport fallback
- **Load tests** (`e2e/load/`) — k6 WebSocket load test
- xterm.js row scraping for E2E assertions (`.xterm-rows > div`)

## Key Files

| Area | Files |
|------|-------|
| Transport types | `server/src/session/types.rs` |
| Transport trait | `server/src/session/transport.rs` |
| SSH backend | `server/src/session/ssh_transport.rs` |
| Session manager | `server/src/session/manager.rs` |
| WS handler | `server/src/ws/handler.rs` |
| Protocol | `server/src/ws/protocol.rs` |
| Session handler | `server/src/ws/session_handler.rs` |
| Auth | `server/src/auth/handler.rs`, `server/src/auth/jwt.rs` |
| Database | `server/src/db/repo.rs` |
| tmux parser | `server/src/tmux/control.rs` |
| Claude parser | `server/src/claude/parser.rs` |
| TURN creds | `server/src/webrtc/turn.rs` |
| Agent QUIC/WebRTC | `agent/src/quic/server.rs` |
| Agent tmux mgr | `agent/src/tmux_manager.rs` |
| Terminal composable | `client/src/composables/useTerminal.ts` |
| WebRTC composable | `client/src/composables/useWebRtc.ts` |
| QUIC composable | `client/src/composables/useQuic.ts` |
| Tmux store | `client/src/stores/tmux.ts` |
| Auth store | `client/src/stores/auth.ts` |
| Qualified pane IDs | `client/src/utils/paneId.ts` |
| Mashed view | `client/src/components/MashedView.vue` |
| Mashed cell | `client/src/components/MashedCell.vue` |
| Terminal pane | `client/src/components/TerminalPane.vue` |
| Session sidebar | `client/src/components/SessionSidebar.vue` |
| App layout | `client/src/App.vue` |
