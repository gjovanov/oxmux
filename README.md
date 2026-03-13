# Oxmux

**Claude Code fleet manager** — a tmux web client built with Rust + Vue 3 that turns remote terminal sessions into a structured AI collaboration dashboard.

[![CI](https://github.com/gjovanov/oxmux/actions/workflows/ci.yml/badge.svg)](https://github.com/gjovanov/oxmux/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

## What it is

Oxmux lets you run and monitor multiple Claude Code agents across remote SSH hosts from a single browser dashboard. It bridges the gap between raw tmux panes and structured AI session management.

```
Browser (Vue 3 + xterm.js)
  │
  │  WSS (terminal stream + control)
  │  WebRTC DataChannel (P2P agent mode)
  ▼
Rust Relay Server (Axum + Tokio + Quinn QUIC)
  │  TURN credential generation from COTURN_AUTH_SECRET
  │  ICE signaling — terminal data never touches relay in P2P mode
  ├─────────────────────────────┐
  │ QUIC (quinn)                │ SSH fallback (russh)
  ▼                             ▼
oxmux-agent (on remote host)  Remote SSH host
  └── PTY + tmux control mode    └── PTY + tmux control mode
  └── claude stream-json         └── claude stream-json

COTURN cluster (coturn.roomler.live)
  3× TURN/TURNS across mars / zeus / jupiter
  ← ICE relay only when P2P STUN fails
```

## Features

| Category | Feature | Status |
|---|---|---|
| **Terminal** | Full xterm.js rendering (WebGL renderer) | ✅ |
| | tmux session/window/pane tree | ✅ |
| | Resize propagation to tmux | ✅ |
| | Copy/paste, mouse support | ✅ |
| **Transport** | WebSocket (WSS, permessage-deflate) | ✅ |
| | WebRTC DataChannel P2P (via oxmux-agent) | ✅ |
| | QUIC server↔agent (quinn) | ✅ |
| | COTURN integration (HMAC-SHA1 time-limited credentials) | ✅ |
| **SSH** | Multi-host SSH connection manager | ✅ |
| | SSH agent forwarding | ✅ |
| | Auto-reconnect with backoff | ✅ |
| **Claude Code** | stream-json parser (all event types) | ✅ |
| | Structured conversation UI (not raw terminal) | ✅ |
| | Tool use blocks (Read/Write/Edit/Bash) with collapse | ✅ |
| | File diff viewer (Monaco editor) | ✅ |
| | Real-time cost meter + context bar | ✅ |
| | Approval prompts surfaced in UI | ✅ |
| | Session recording + replay | ✅ |
| **Fleet** | Multi-agent dashboard (all hosts in one view) | ✅ |
| | Aggregate cost tracking across sessions | ✅ |
| | Process auto-detection (claude in tmux panes) | ✅ |
| **Testing** | Unit (Rust, proptest) | ✅ |
| | Integration (tokio-test + testcontainers) | ✅ |
| | E2E (Playwright) | ✅ |
| | Chaos / load (k6) | ✅ |

## Tech Stack

| Layer | Technology |
|---|---|
| **Server** | Rust, Axum 0.7, Tokio, portable-pty, russh, quinn (QUIC) |
| **Client** | Vue 3, Vite, xterm.js (WebGL), Pinia, TypeScript |
| **Protocol** | MessagePack (binary frames), WebRTC DataChannel |
| **WebRTC** | COTURN (shared secret HMAC-SHA1), ICE, DTLS |
| **Testing** | cargo test, proptest, testcontainers-rs, Playwright, k6 |
| **Deploy** | Docker, Kubernetes (see [oxmux-deploy](https://github.com/gjovanov/oxmux-deploy)) |

## Quick Start

```bash
# Clone
git clone https://github.com/gjovanov/oxmux.git
cd oxmux

# Configure
cp .env.example .env
# Edit .env — minimum: COTURN_AUTH_SECRET, SSH host details

# Start (server + client dev servers)
make dev

# Or with Docker
docker compose up
```

Open `http://localhost:5173`

## Project Structure

```
oxmux/
├── server/          # Rust — Axum WebSocket server, SSH, QUIC, PTY manager
├── client/          # Vue 3 — terminal UI, Claude Code dashboard
├── agent/           # Rust — oxmux-agent binary (runs on remote machines)
├── e2e/             # Playwright end-to-end tests
├── docs/            # Architecture, transport, COTURN, Claude Code integration
├── scripts/         # Utility scripts (TURN credential generator etc.)
└── .github/         # CI workflows, issue templates
```

## Documentation

| Document | Description |
|---|---|
| [Architecture](docs/architecture.md) | System overview, transport layers, data flow |
| [Transport](docs/transport.md) | WebSocket, WebRTC, QUIC — when each is used |
| [COTURN Integration](docs/coturn-integration.md) | HMAC-SHA1 credential flow, ICE server config |
| [Claude Code Integration](docs/claude-code-integration.md) | stream-json parser, structured UI, fleet management |

## Deployment

See [gjovanov/oxmux-deploy](https://github.com/gjovanov/oxmux-deploy) for Kubernetes manifests.

## License

MIT
