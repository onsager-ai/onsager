.PHONY: build test lint fmt check clean dev ui docker release

# ── Coding ──────────────────────────────────────────────────
build:
	cd rust && cargo build

dev: build
	cd rust && cargo run --bin synodic -- --help

# ── QA ──────────────────────────────────────────────────────
test:
	cd rust && cargo test

e2e:
	cd rust && cargo test --test e2e

lint:
	cd rust && cargo clippy --all-targets -- -D warnings

fmt:
	cd rust && cargo fmt --all

fmt-check:
	cd rust && cargo fmt --all -- --check

check: fmt-check lint test
	@echo "All checks passed."

# ── UI ──────────────────────────────────────────────────────
ui:
	cd packages/ui && pnpm install && pnpm run build

ui-dev:
	cd packages/ui && pnpm install && pnpm run dev

# ── Delivery ────────────────────────────────────────────────
release:
	cd rust && cargo build --release

docker:
	docker build -f docker/Dockerfile -t synodic:latest .

serve: release
	cd rust && cargo run --release --bin synodic-http

# ── Cleanup ─────────────────────────────────────────────────
clean:
	cd rust && cargo clean
