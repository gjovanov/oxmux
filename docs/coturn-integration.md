# COTURN Integration

Oxmux integrates with the COTURN cluster deployed via
[k8s-cluster-multi](https://github.com/gjovanov/k8s-cluster-multi).

## Cluster Overview

Three COTURN pods running with `hostNetwork: true`, one per Hetzner worker:

| Host    | Public IP        | TURN Port | TURNS Port | Worker VM      |
|---------|-----------------|-----------|------------|----------------|
| mars    | 198.51.100.10   | 3478      | 5349       | 10.10.10.11    |
| zeus    | 198.51.100.20   | 3478      | 5349       | 10.10.20.11    |
| jupiter | 198.51.100.30   | 3478      | 5349       | 10.10.30.11    |

Realm: `coturn.roomler.live`
Alt-TLS: `:443` via SNI proxy on mars

## Authentication: HMAC-SHA1 Shared Secret

COTURN is configured with `use-auth-secret` + `static-auth-secret`. This means
clients never use a static username/password — instead, the Oxmux server generates
short-lived credentials on demand.

### Credential Generation (Rust)

```rust
// server/src/webrtc/turn.rs
let expiry = unix_timestamp_now() + ttl;
let username = format!("{}:{}", expiry, user_id);
let credential = base64(HMAC-SHA1(COTURN_AUTH_SECRET, username));
```

### ICE Server Config sent to browser

```json
{
  "ice_servers": [
    {
      "urls": ["stun:198.51.100.10:3478", "stun:198.51.100.20:3478", "stun:198.51.100.30:3478"]
    },
    {
      "urls": [
        "turn:198.51.100.10:3478", "turn:198.51.100.20:3478", "turn:198.51.100.30:3478",
        "turns:198.51.100.10:5349", "turns:198.51.100.20:5349", "turns:198.51.100.30:5349"
      ],
      "username": "1735000000:user-abc123",
      "credential": "base64encodedhmac=="
    }
  ]
}
```

## Security Properties

- `COTURN_AUTH_SECRET` is **never sent to the browser** — only the derived
  time-limited credential is
- Credentials expire after `COTURN_TTL` seconds (default: 86400 = 24h)
- Each WebRTC session gets a fresh credential scoped to the user's session ID
- TURNS (TLS) is used for `turns:` URIs to prevent credential interception

## Environment Variables

```bash
COTURN_AUTH_SECRET=<your shared secret from k8s-cluster-multi .env>
COTURN_REALM=coturn.roomler.live
COTURN_TTL=86400
COTURN_SERVERS=198.51.100.10:3478,198.51.100.20:3478,198.51.100.30:3478
COTURN_TLS_SERVERS=198.51.100.10:5349,198.51.100.20:5349,198.51.100.30:5349
```

In Kubernetes these are stored in the `oxmux` Secret (see
[oxmux-deploy](https://github.com/gjovanov/oxmux-deploy)).

## ICE Candidate Flow

```
Browser ──STUN request──► COTURN (attempts P2P hole-punch)
         ──TURN alloc ──► COTURN (relay if P2P fails)
                              │
                    iptables DNAT on host
                              │
                         COTURN pod
                         (hostNetwork, worker-N)
```

When the oxmux-agent is installed on a remote machine and WebRTC P2P succeeds
via STUN, **no terminal data flows through COTURN** — the relay is only used as
ICE fallback for NAT traversal failure (corporate firewalls, symmetric NAT, etc.).
