# API Reference

## Authentication

All API requests (except `/health` and auth endpoints) require authentication.

### REST Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/api/auth/register` | None | Create account |
| `POST` | `/api/auth/login` | None | Login, get JWT |
| `GET` | `/api/auth/me` | Bearer/Query | Validate token, get user info |
| `GET` | `/ws` | Query `?token=` | WebSocket upgrade |
| `GET` | `/api/ice-config` | None | TURN/STUN credentials |
| `GET` | `/health` | None | Health check |

### Register

```http
POST /api/auth/register
Content-Type: application/json

{
  "username": "gjovanov",
  "password": "my_password"
}
```

Response:
```json
{
  "token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9...",
  "user": {
    "id": "84f5f44a-524b-42fc-a3d0-599fdd1a0517",
    "username": "gjovanov"
  }
}
```

### Login

```http
POST /api/auth/login
Content-Type: application/json

{
  "username": "gjovanov",
  "password": "my_password"
}
```

Same response format as register.

### Me

```http
GET /api/auth/me?token=<jwt>
```

Response:
```json
{
  "id": "84f5f44a-524b-42fc-a3d0-599fdd1a0517",
  "username": "gjovanov"
}
```

### ICE Config

```http
GET /api/ice-config?user=<user_id>
```

Response:
```json
{
  "ice_servers": [
    {
      "urls": ["stun:198.51.100.10:3478", "stun:198.51.100.20:3478"]
    },
    {
      "urls": ["turn:198.51.100.10:3478", "turns:198.51.100.10:5349"],
      "username": "1773527150:user123",
      "credential": "1ddOUuHEtmmbLUGbJbW9X/DS63w="
    }
  ]
}
```

## WebSocket Protocol

Binary MessagePack frames over `wss://oxmux.app/ws?token=<jwt>`.

### Client → Server Messages

| Tag (`t`) | Name | Fields | Description |
|-----------|------|--------|-------------|
| `sub` | Subscribe | `pane: string` | Subscribe to pane output |
| `unsub` | Unsubscribe | `pane: string` | Unsubscribe from pane |
| `i` | Input | `pane: string, data: bytes` | Send keyboard/paste input |
| `r` | Resize | `pane: string, cols: u16, rows: u16` | Terminal resize |
| `cmd` | TmuxCommand | `command: string` | Raw tmux command |
| `ice_req` | IceRequest | `peer_id: string` | Request TURN credentials |
| `sig` | Signal | `peer_id: string, payload: json` | WebRTC signaling |
| `claude_in` | ClaudeInput | `session_id: string, prompt: string` | Inject Claude prompt |
| `ping` | Ping | `ts: u64` | Latency measurement |
| `sess_create` | CreateSession | `name, transport` | Create session |
| `sess_list` | ListSessions | — | List all user sessions |
| `sess_connect` | ConnectSession | `session_id: string` | Start transport |
| `sess_disconnect` | DisconnectSession | `session_id: string` | Stop transport |
| `sess_update` | UpdateSession | `session_id, name?` | Update metadata |
| `sess_delete` | DeleteSession | `session_id: string` | Delete session |
| `sess_refresh` | RefreshSession | `session_id: string` | Refresh tmux state |

### Server → Client Messages

| Tag (`t`) | Name | Fields | Description |
|-----------|------|--------|-------------|
| `o` | Output | `pane: string, data: bytes` | Raw PTY output |
| `s` | State | `sessions: TmuxSessionInfo[]` | Full tmux state dump |
| `e` | TmuxEvent | `event: TmuxEventMsg` | Incremental tmux event |
| `c` | ClaudeEvent | `session_id, event` | Structured Claude event |
| `ca` | ClaudeAccumulator | `session_id, state` | Cost/file summary |
| `ice` | IceConfig | `peer_id, config` | TURN credentials |
| `sig` | Signal | `peer_id, payload` | WebRTC signaling relay |
| `err` | Error | `code: string, message: string` | Error response |
| `pong` | Pong | `ts: u64` | Latency response |
| `sess_list` | SessionList | `sessions: ManagedSession[]` | User's sessions |
| `sess_created` | SessionCreated | `session: ManagedSession` | Session created |
| `sess_updated` | SessionUpdated | `session: ManagedSession` | Session updated |
| `sess_deleted` | SessionDeleted | `session_id: string` | Session deleted |
| `sess_connected` | SessionConnected | `session: ManagedSession` | Transport established |
| `sess_disconnected` | SessionDisconnected | `session: ManagedSession` | Transport stopped |

### Transport Configuration

```typescript
// Browser transport (how browser talks to server/agent)
type BrowserTransport = 'websocket' | 'quic' | 'webrtc'

// Backend transport (how server/agent reaches tmux)
interface SshBackend {
  type: 'ssh'
  host: string
  port: number        // default: 22
  user: string
  auth: SshAuth
}

interface AgentBackend {
  type: 'agent'
  agent_id: string
}

type SshAuth =
  | { method: 'agent' }
  | { method: 'password', password: string }
  | { method: 'private_key', path: string, passphrase?: string }
```

### tmux State Types

```typescript
interface TmuxSessionInfo {
  id: string          // e.g., "$0"
  name: string        // e.g., "mars1"
  windows: TmuxWindowInfo[]
}

interface TmuxWindowInfo {
  id: string          // e.g., "@0"
  name: string        // e.g., "bash"
  index: number
  layout: string      // tmux layout string
  panes: TmuxPaneInfo[]
}

interface TmuxPaneInfo {
  id: string          // e.g., "%0"
  index: number
  cols: number
  rows: number
  currentCommand: string
  isActive: boolean
  isClaude: boolean   // auto-detected from process name
}
```

### ManagedSession

```typescript
interface ManagedSession {
  id: string
  name: string
  transport: TransportConfig
  status: 'created' | 'connecting' | 'connected' | 'reconnecting' | 'disconnected' | 'error'
  error?: string
  tmux_sessions: TmuxSessionInfo[]
}
```

## QUIC Protocol

Same MessagePack protocol as WebSocket, carried over QUIC streams (WebTransport API):

| Stream | Direction | Content |
|--------|-----------|---------|
| Stream 0 | Bidirectional | Control: auth, session CRUD, signaling |
| Stream N | Bidirectional | Per-pane PTY I/O (binary) |

Auth is performed in the first message on Stream 0:
```
{t: "auth", token: "<jwt>"}
```

## Agent QUIC Protocol

Same protocol as browser QUIC, but with additional agent-specific messages:

| Message | Direction | Description |
|---------|-----------|-------------|
| `agent_register` | Agent → Server | Register with capabilities |
| `agent_heartbeat` | Agent → Server | Periodic health check |
| `agent_deploy_ack` | Agent → Server | Confirm deployment success |
