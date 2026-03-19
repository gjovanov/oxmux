# ─── Stage 1: Build Rust server ───────────────────────────────────────────────
FROM rust:1.91-slim AS server-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev musl-tools && rm -rf /var/lib/apt/lists/* && \
    rustup target add x86_64-unknown-linux-musl

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
RUN touch server/src/main.rs agent/src/main.rs && \
    cargo build --release --package oxmux-server && \
    cargo build --release --package oxmux-agent --target x86_64-unknown-linux-musl

# ─── Stage 2: Build Vue 3 client ──────────────────────────────────────────────
FROM node:22-slim AS client-builder

WORKDIR /build
COPY client/package.json client/package-lock.json* ./
RUN npm install --no-audit --no-fund

COPY client/ ./
RUN npx vite build

# ─── Stage 3: Runtime ─────────────────────────────────────────────────────────
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates tmux openssh-client openssl && \
    rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash oxmux

WORKDIR /app
COPY --from=server-builder /build/target/release/oxmux-server ./
COPY --from=server-builder /build/target/x86_64-unknown-linux-musl/release/oxmux-agent ./
COPY --from=client-builder /server/static ./static

USER oxmux
EXPOSE 8080 4433/udp

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["./oxmux-server"]
