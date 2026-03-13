# ─── Stage 1: Build Rust server ───────────────────────────────────────────────
FROM rust:1.82-slim AS server-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY server/Cargo.toml server/
COPY agent/Cargo.toml agent/

# Cache deps layer
RUN mkdir -p server/src agent/src && \
    echo "fn main(){}" > server/src/main.rs && \
    echo "fn main(){}" > agent/src/main.rs && \
    cargo build --release --package oxmux-server && \
    rm -rf server/src agent/src

COPY server/src server/src
COPY agent/src agent/src
RUN touch server/src/main.rs && \
    cargo build --release --package oxmux-server

# ─── Stage 2: Build Vue 3 client ──────────────────────────────────────────────
FROM node:22-slim AS client-builder

WORKDIR /build
COPY client/package.json client/package-lock.json* ./
RUN npm ci

COPY client/ ./
RUN npm run build

# ─── Stage 3: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates tmux && \
    rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash oxmux

WORKDIR /app
COPY --from=server-builder /build/target/release/oxmux-server ./
COPY --from=client-builder /build/dist ./static

USER oxmux
EXPOSE 8080 4433/udp

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["./oxmux-server"]
