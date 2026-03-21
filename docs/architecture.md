# Architecture

## System Overview

Oxmux provides 5 transport paths for connecting a browser terminal to a remote tmux session. All transports carry the same binary MessagePack protocol.

```mermaid
graph TB
    subgraph Browser["Browser (Vue 3 + xterm.js)"]
        UI[Terminal UI]
        WS_C[WebSocket]
        QUIC_C[WebTransport]
        WebRTC_C[WebRTC DC]
    end

    subgraph Server["Oxmux Server (K8s)"]
        WS_H[WS Handler]
        SM[Session Manager]
        SSH_T[SSH Transport]
        DB[(SQLite)]
        AR[Agent Registry]
    end

    subgraph Host["Remote Host"]
        Agent[oxmux-agent]
        TMUX[tmux]
        Shell[bash / claude]
    end

    subgraph COTURN["COTURN Relay"]
        TURN[TURN/TURNS]
    end

    UI --> WS_C --> WS_H
    UI --> QUIC_C --> Agent
    UI --> WebRTC_C --> Agent

    WS_H --> SM --> SSH_T -->|SSH| TMUX
    SM --> DB
    SM --> AR

    Agent -->|tmux -CC| TMUX
    TMUX --> Shell

    WebRTC_C -.->|ICE| TURN
```

## Dual-Transport Model

The browser maintains TWO transport layers simultaneously:

| Layer | Transport | Purpose | Always Active |
|-------|-----------|---------|---------------|
| **Control Plane** | WebSocket | Session CRUD, agent management, transport upgrades | Yes |
| **Data Plane** | QUIC P2P or WebRTC P2P | Pane I/O (input, output, resize, subscribe) | Only when P2P active |

```mermaid
sequenceDiagram
    participant B as Browser
    participant WS as WS (Control)
    participant P2P as P2P (Data)
    participant S as Server
    participant A as Agent

    B->>WS: sess_connect, agent_install
    S-->>WS: sess_connected, agent_status

    Note over B,A: P2P Upgrade
    B->>WS: transport_upgrade
    S-->>WS: transport_upgrade_ready
    B->>P2P: WebRTC/QUIC connect to Agent

    Note over B,A: Dual-transport active
    B->>P2P: sub, i, r (data)
    A-->>P2P: o (output)
    B->>WS: sess_refresh (control)

    Note over B,A: P2P drops → auto SSH fallback
    P2P--xB: closed
    B->>WS: sub (re-subscribe via SSH)
```

## WebRTC P2P Flow (Browser-as-Offerer)

Browser creates the offer (with DataChannel), agent creates the answer. This pattern ensures Chrome generates ICE candidates.

```mermaid
sequenceDiagram
    participant B as Browser
    participant Q as QUIC Signaling
    participant A as Agent

    B->>Q: WebTransport connect + auth
    B->>Q: sess_connect (session name)
    A-->>Q: sess_connected

    B->>B: pc = new RTCPeerConnection()
    B->>B: dc = pc.createDataChannel('oxmux')
    B->>B: offer = pc.createOffer()
    B->>B: pc.setLocalDescription(offer)

    B->>Q: webrtc_offer (SDP)

    loop Trickle ICE
        B->>Q: webrtc_ice (candidate)
    end

    A->>A: pc.setRemoteDescription(offer)
    A->>A: answer = pc.createAnswer()
    A->>A: Wait for ICE gathering complete
    A-->>Q: webrtc_answer (SDP + all candidates)

    B->>B: pc.setRemoteDescription(answer)
    B-->>B: ICE connected
    B-->>B: DataChannel opened

    Note over B,A: Terminal I/O via DataChannel
    B->>A: {t:'i', pane:'%0', data:[bytes]}
    A-->>B: {t:'o', pane:'%0', data:[bytes]}
```

## Multi-Session Architecture

### Qualified Pane IDs

tmux pane IDs (`%0`, `%1`) are only unique per tmux server. Multiple sessions on different hosts share the same IDs. Qualified pane IDs scope them globally:

```
{managedSessionId}::{tmuxPaneId}
e.g., "abc-123::%0"
```

### Per-Session P2P

Each connected session can have its own independent P2P connection:

```mermaid
graph LR
    subgraph Store
        ST[sessionTrees<br/>Map‹sid, trees›]
        P2P[p2pConnections<br/>Map‹sid, conn›]
        PH[paneHandlers<br/>Map‹qid, handler›]
    end

    subgraph "Session A (mars)"
        A_P2P[WebRTC P2P]
        A_TMUX[tmux %0, %1]
    end

    subgraph "Session B (zeus)"
        B_P2P[QUIC P2P]
        B_TMUX[tmux %0, %1]
    end

    P2P -->|sid_a| A_P2P --> A_TMUX
    P2P -->|sid_b| B_P2P --> B_TMUX
```

### Mashed View (NxN Grid)

```mermaid
graph TB
    subgraph "MashedView (2x2)"
        C1["MashedCell<br/>mars %0 (bash)<br/>WebRTC P2P"]
        C2["MashedCell<br/>mars %1 (claude)<br/>WebRTC P2P"]
        C3["MashedCell<br/>zeus %0 (bash)<br/>QUIC P2P"]
        C4["Empty Cell<br/>+ Add pane"]
    end

    subgraph "Terminal Registry"
        TR[Global Map‹paneId, Terminal›<br/>Survives view switches]
    end

    C1 & C2 & C3 --> TR
```

## Terminal Registry

Prevents duplicate event handlers when switching between single and mashed views:

```mermaid
stateDiagram-v2
    [*] --> Created: component mounts
    Created --> Attached: xterm.open(container)
    Attached --> Reattached: view switch (reuse DOM)
    Reattached --> Attached: refit + resize
    Attached --> GracePeriod: component unmounts
    GracePeriod --> Attached: remount within 500ms
    GracePeriod --> Disposed: no remount → cleanup
    Disposed --> [*]
```

## Agent Deployment

```mermaid
sequenceDiagram
    participant B as Browser
    participant S as Server
    participant H as Remote Host

    B->>S: agent_install (session_id)
    S-->>B: agent_status: installing

    S->>H: SSH connect
    S->>H: SCP oxmux-agent (musl static binary)
    S->>H: Create systemd service
    S->>H: systemctl start oxmux-agent

    loop Health Check (5x, 2s apart)
        S->>H: systemctl is-active
    end

    S->>S: Register in Agent Registry
    S-->>B: agent_status: online

    Note over B,H: Ready for P2P upgrade
    B->>S: transport_upgrade (webrtc_p2p)
    S-->>B: transport_upgrade_ready<br/>(agent-mars.oxmux.app:4433, JWT)
```

## Transport Fallback Chain

```mermaid
graph TD
    A[WebRTC P2P] -->|DC close| B[QUIC P2P]
    B -->|WebTransport error| C[WS → SSH]
    C -->|WS reconnect| D[Auto-restore sessions]

    style A fill:#fab387,color:#1e1e2e
    style B fill:#cba6f7,color:#1e1e2e
    style C fill:#89b4fa,color:#1e1e2e
    style D fill:#a6e3a1,color:#1e1e2e
```

## tmux Integration

### Control Mode
- `tmux -CC attach -t <session>` via `script -q` for PTY allocation
- `%output <pane_id> <data>` events parsed and broadcast per pane
- `window-size latest` allows resize beyond control mode client's 80x24
- `send-keys -H <hex bytes>` with each byte as separate argument

### Pane Sizing
- Browser: FitAddon calculates cols/rows from container dimensions
- Client sends `{t:'r', pane:'%0', cols:130, rows:30}` on resize
- Agent/server runs `tmux resize-pane -t %0 -x 130 -y 30`
- `SIGWINCH` propagated to shell automatically by tmux
