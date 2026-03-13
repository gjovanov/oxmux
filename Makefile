.PHONY: dev dev-server dev-client build build-agent test test-e2e test-load lint clean docker-up docker-down

# ─── Development ──────────────────────────────────────────────────────────────
dev:
	@make -j2 dev-server dev-client

dev-server:
	cd server && cargo watch -x run

dev-client:
	cd client && npm run dev

# ─── Build ────────────────────────────────────────────────────────────────────
build:
	cd client && npm run build
	cargo build --release --package oxmux-server

build-agent:
	cargo build --release --package oxmux-agent

# ─── Testing ──────────────────────────────────────────────────────────────────
test:
	cargo test --workspace

test-e2e:
	cd e2e && npx playwright test

test-load:
	k6 run e2e/load/ws-load.js

lint:
	cargo clippy --workspace -- -D warnings
	cd client && npm run lint

# ─── Docker ───────────────────────────────────────────────────────────────────
docker-up:
	docker compose up --build

docker-up-dev:
	docker compose --profile dev up

docker-down:
	docker compose down

# ─── Utilities ────────────────────────────────────────────────────────────────
gen-turn-creds:
	@echo "Generating TURN credentials for user=$(USER)..."
	@cargo run --package oxmux-server --bin gen-turn -- $(USER)

clean:
	cargo clean
	rm -rf client/node_modules client/dist server/static e2e/node_modules
