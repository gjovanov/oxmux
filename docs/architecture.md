# Architecture

## System Overview

Oxmux provides 5 transport paths for connecting a browser terminal to a remote tmux session. All transports carry the same binary MessagePack protocol.

```mermaid
graph TB
    subgraph Browser["Browser (Vue 3 + xterm.js)"]
        UI[Terminal UI]
        WS_C[WebSocket Client]
        QUIC_C[WebTransport Client]
        WEBRTC_C[RTCPeerConnection]
    end

    subgraph Server["oxmux-server (Rust + Axum)"]
        WS_S[WS Handler]
        QUIC_S[QUIC Listener]
        WEBRTC_S[WebRTC Listener]
        SM[Session Manager]
        SSH[SSH Transport<br/>russh]
        DB[(SQLite)]
    end

    subgraph Agent["oxmux-agent (Rust)"]
        QUIC_A[QUIC Listener]
        WEBRTC_A[WebRTC Listener]
        PTY_A[tmux Manager]
    end

    subgraph Host["Remote Host"]
        TMUX[tmux]
        CLAUDE[Claude Code]
    end

    %% Transport 1: WS → SSH
    UI --> WS_C
    WS_C -->|"1. WebSocket"| WS_S
    WS_S --> SM
    SM --> SSH
    SSH -->|SSH| TMUX

    %% Transport 2: QUIC → SSH
    QUIC_C -->|"2. QUIC"| QUIC_S
    QUIC_S --> SM

    %% Transport 3: WebRTC → SSH
    WEBRTC_C -->|"3. WebRTC"| WEBRTC_S
    WEBRTC_S --> SM

    %% Transport 4: QUIC → Agent P2P
    QUIC_C -.->|"4. QUIC P2P"| QUIC_A
    QUIC_A --> PTY_A
    PTY_A --> TMUX

    %% Transport 5: WebRTC → Agent P2P
    WEBRTC_C -.->|"5. WebRTC P2P"| WEBRTC_A
    WEBRTC_A --> PTY_A

    SM --> DB
    TMUX --> CLAUDE

    style Browser fill:#1e1e2e,color:#cdd6f4,stroke:#89b4fa
    style Server fill:#181825,color:#cdd6f4,stroke:#a6e3a1
    style Agent fill:#181825,color:#cdd6f4,stroke:#cba6f7
    style Host fill:#11111b,color:#cdd6f4,stroke:#f9e2af
```

## Transport Comparison

| # | Name | Data Path | Latency | Requires | Use Case |
|---|------|-----------|---------|----------|----------|
| 1 | WS → SSH | Browser → Server → Host | High | Nothing extra | Default, always works |
| 2 | QUIC → SSH | Browser → Server → Host | Medium | QUIC port on server | Mobile, lossy networks |
| 3 | WebRTC → SSH | Browser → Server → Host | Medium | TURN/STUN | NAT traversal to server |
| 4 | QUIC → Agent | Browser → Agent direct | Low | Agent on host | Best performance |
| 5 | WebRTC → Agent | Browser → Agent direct | Low | Agent + TURN | P2P through any NAT |

## Session Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Created: create session
    Created --> Connecting: connect
    Connecting --> Connected: transport established
    Connecting --> Error: connection failed
    Connected --> Disconnected: disconnect / network loss
    Connected --> Error: transport error
    Disconnected --> Connecting: reconnect
    Error --> Connecting: retry
    Error --> [*]: delete
    Disconnected --> [*]: delete
    Created --> [*]: delete

    state Connected {
        [*] --> Streaming
        Streaming --> Resizing: terminal resize
        Resizing --> Streaming: resize applied
    }
```

## Transport 1: WebSocket → SSH

The default transport. Browser connects via WSS, server SSHes to the remote host.

```mermaid
sequenceDiagram
    participant B as Browser
    participant S as oxmux-server
    participant H as Remote Host

    B->>S: WSS upgrade (?token=jwt)
    S->>S: Validate JWT
    S-->>B: WS connected

    B->>S: CreateSession {name, ssh_config}
    S->>S: Store in SQLite
    S-->>B: SessionCreated

    B->>S: ConnectSession {id}
    S->>H: SSH connect (russh)
    S->>H: SSH auth (key/password)
    S->>H: exec: tmux new-session -d -s <name>
    S->>H: exec: tmux list-panes (query state)
    S->>H: exec: tmux -CC attach (control mode)
    S-->>B: SessionConnected {tmux_sessions}

    B->>S: Subscribe {pane: "%0"}
    Note over S,H: Control mode streams %output events

    loop PTY I/O
        H-->>S: %output %0 <data>
        S->>S: ControlModeParser → broadcast
        S-->>B: Output {pane: "%0", data}
        B->>B: xterm.js write(data)

        B->>S: Input {pane: "%0", data}
        S->>H: send-keys -t %0 -H <hex>
    end
```

## Transport 2: QUIC → SSH

Browser uses WebTransport API for QUIC connection to server. Server SSHes to host.

```mermaid
sequenceDiagram
    participant B as Browser
    participant S as oxmux-server
    participant H as Remote Host

    B->>S: QUIC connect (WebTransport)
    B->>S: Stream 0: Auth {token: jwt}
    S->>S: Validate JWT
    S-->>B: AuthOk

    B->>S: Stream 0: ConnectSession {id}
    S->>H: SSH connect + tmux -CC attach
    S-->>B: SessionConnected

    loop PTY I/O (multiplexed streams)
        H-->>S: %output %0 <data>
        S-->>B: Stream 1: Output {pane, data}
        B->>S: Stream 1: Input {pane, data}
        S->>H: send-keys -H <hex>
    end
```

## Transport 4: QUIC → Agent (P2P)

Browser connects directly to oxmux-agent via QUIC. No server in data path.

```mermaid
sequenceDiagram
    participant B as Browser
    participant S as oxmux-server
    participant A as oxmux-agent

    Note over A,S: Agent registered via QUIC heartbeat

    B->>S: ListAgents
    S-->>B: [{agent_id, host, quic_port}]

    B->>S: RequestAgentToken {agent_id}
    S-->>B: {token: short-lived-jwt}

    B->>A: QUIC connect (WebTransport)
    B->>A: Auth {token}
    A->>A: Verify JWT (shared secret with server)
    A-->>B: AuthOk

    B->>A: ConnectSession {name}
    A->>A: tmux new-session / attach
    A-->>B: SessionConnected {tmux_sessions}

    loop PTY I/O (direct P2P)
        A-->>B: Output {pane, data}
        B->>A: Input {pane, data}
    end
```

## Transport 5: WebRTC → Agent (P2P)

Browser connects directly to agent via WebRTC DataChannel. Server only relays signaling.

```mermaid
sequenceDiagram
    participant B as Browser
    participant S as oxmux-server
    participant T as TURN Server
    participant A as oxmux-agent

    B->>S: RequestIceConfig
    S-->>B: {iceServers: [{urls, credential}]}

    B->>B: Create RTCPeerConnection
    B->>S: Signal {type: offer, sdp, to: agent_id}
    S->>A: Relay offer
    A->>A: Create RTCPeerConnection
    A->>S: Signal {type: answer, sdp, to: browser_id}
    S-->>B: Relay answer

    loop ICE
        B->>S: Signal {type: ice_candidate}
        S->>A: Relay candidate
        A->>S: Signal {type: ice_candidate}
        S-->>B: Relay candidate
    end

    Note over B,A: DataChannel established (P2P or via TURN)

    B->>A: DataChannel: ConnectSession {name}
    A-->>B: DataChannel: SessionConnected

    loop PTY I/O (direct P2P)
        A-->>B: DataChannel: Output {pane, data}
        B->>A: DataChannel: Input {pane, data}
    end
```

## Module Structure

```mermaid
graph LR
    subgraph Server
        main[main.rs]
        auth[auth/]
        db[db/]
        ws[ws/]
        session[session/]
        transport[transport/]
        tmux[tmux/]
        claude[claude/]
        webrtc[webrtc/]
        quic[quic/]
        pty[pty/]
    end

    main --> auth
    main --> ws
    main --> quic
    ws --> session
    quic --> session
    session --> transport
    transport --> tmux
    transport --> claude
    transport --> pty
    session --> db
    auth --> db
    ws --> webrtc

    style auth fill:#e65100,color:white
    style ws fill:#1976d2,color:white
    style quic fill:#7b1fa2,color:white
    style session fill:#ff9800,color:white
    style transport fill:#2e7d32,color:white
    style tmux fill:#455a64,color:white
    style claude fill:#0097a7,color:white
    style db fill:#78909c,color:white
```

## Database Schema

```mermaid
erDiagram
    users {
        text id PK
        text username UK
        text password_hash
        text created_at
        text updated_at
    }

    sessions {
        text id PK
        text user_id FK
        text name
        text transport_config "JSON"
        text status
        text error
        text created_at
        text updated_at
    }

    users ||--o{ sessions : "owns"
```

## Agent Deployment Flow

```mermaid
sequenceDiagram
    participant U as User (Browser)
    participant S as oxmux-server
    participant H as Remote Host

    U->>S: InstallAgent {host, ssh_config}
    S->>H: SSH connect
    S->>H: scp oxmux-agent binary
    S->>H: Create systemd unit
    S->>H: systemctl enable --now oxmux-agent
    S-->>U: AgentInstalling

    H->>S: QUIC connect (agent registration)
    H->>S: Heartbeat {agent_id, version, capabilities}
    S->>S: Register agent in memory
    S-->>U: AgentOnline {agent_id, version}

    Note over U,H: Browser can now use Transport 4 or 5
```

## tmux Control Mode

The server uses `tmux -CC attach` (control mode) for both state management and PTY I/O streaming:

```
┌─────────────────────────────────────────────┐
│ SSH Channel (persistent)                     │
│                                              │
│ stdin  → tmux commands                       │
│   send-keys -t %0 -H 1b 5b 41              │
│   resize-pane -t %0 -x 120 -y 35           │
│                                              │
│ stdout ← control mode notifications         │
│   %output %0 \033[1;32mhello\033[0m         │
│   %session-changed $0 my-session            │
│   %layout-change @0 ab12,120x35,0,0,0      │
│   %window-add @1                            │
└─────────────────────────────────────────────┘
```

Output bytes are octal-escaped by tmux. The `ControlModeParser` decodes them and broadcasts to per-pane `broadcast::channel`s. Each subscribed browser client receives the same bytes via their chosen transport.
