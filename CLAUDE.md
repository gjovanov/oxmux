# CLAUDE.md — Oxmux AI Context

## Project Overview

Oxmux is a Claude Code fleet manager — a tmux web client that provides structured dashboards for managing remote Claude Code sessions. Built with Rust (server + agent) and Vue 3 (client).

## Monorepo Structure

```
oxmux/
├── server/    Rust: Axum WS server, SSH (russh), QUIC (quinn), PTY, tmux control mode
├── client/    Vue 3: xterm.js terminal UI + structured Claude Code dashboard
├── agent/     Rust: standalone binary deployed on remote machines for QUIC/WebRTC P2P
└── e2e/       Playwright tests
```

## Key Architecture Decisions

### Transport Hierarchy
1. **WebRTC DataChannel P2P** (preferred) — browser connects directly to oxmux-agent via QUIC/DTLS. COTURN relay only for ICE fallback.
2. **QUIC server↔agent** (server-side) — relay server connects to agent via quinn QUIC, not SSH.
3. **WebSocket SSH fallback** — when no agent installed, relay server uses russh to SSH into remote host.

### TURN Credentials
- COTURN at `coturn.roomler.live` uses **shared secret HMAC-SHA1** (time-limited)
- `username = "<unix_timestamp + ttl>:<user_id>"`
- `password = base64(HMAC-SHA1(COTURN_AUTH_SECRET, username))`
- Generated server-side in `server/src/webrtc/turn.rs`, sent to client on WebRTC session init
- TTL: 86400s (24h). Never expose `COTURN_AUTH_SECRET` to browser.
- ICE servers: all 3 workers: mars (198.51.100.10), zeus (198.51.100.20), jupiter (198.51.100.30) on ports 3478/5349

### Claude Code Integration
- `claude --output-format stream-json` emits JSONL
- Parser in `server/src/claude/parser.rs` (Rust) and forwarded as structured `ServerMsg::ClaudeEvent`
- Vue renders `ClaudePane.vue` for structured view vs `TerminalPane.vue` for raw PTY
- Mode is auto-detected from process name in tmux pane

### PTY / tmux
- `portable-pty` crate for PTY management
- tmux control mode (`tmux -CC`) for structured state (sessions/windows/panes/layout)
- One `broadcast::channel` per pane — all subscribed WS clients receive same bytes

### Protocol
- Binary MessagePack frames over WebSocket (not JSON)
- `server/src/ws/protocol.rs` defines `ServerMsg` and `ClientMsg` enums

## Environment Variables

All config comes from `.env` (dev) or Kubernetes Secret (prod). See `.env.example` for all vars.
Critical vars: `COTURN_AUTH_SECRET`, `COTURN_REALM`, `QUIC_CERT_PATH`, `QUIC_KEY_PATH`.

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

## Code Style (Vue 3 / TypeScript)

- Composition API only (no Options API)
- Pinia stores for all shared state
- `useWebSocket` composable handles all WS lifecycle
- `useTerminal` composable wraps xterm.js per pane
- Binary MessagePack via `@msgpack/msgpack`

## Testing Notes

- Rust integration tests spin up real tmux with isolated socket (`-S /tmp/test-<uuid>.sock`)
- SSH integration uses `testcontainers-rs` with `linuxserver/openssh-server`
- E2E tests require `docker compose up` running
- xterm.js accessibility layer (`data-testid="terminal-accessible-output"`) used for Playwright assertions
