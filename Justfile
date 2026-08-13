# rustweb stack — Docker Compose with a friendlier face.

# List available recipes (bare `just` hits this first)
default:
	@just --list

# Dev loop (foreground): build the dev image, then cargo watch live-reloads on every save
dev:
	docker compose up --build dev

# Stop the stack; containers are removed, the data volumes are kept
down:
	docker compose down

# Stop the stack and delete all data volumes (postgres data + rebuild cache).
# Also the fix for slow rebuilds after a dep change or toolchain bump: the
# next `just dev` re-seeds the rebuild cache from the image.
remove:
	docker compose down -v
alias rm := remove

# Tail logs from all services
logs:
	docker compose logs -f

# Restart the stack; the container then runs headless — use `just dev` for the
# foreground loop again.
restart:
	docker compose restart

# Open a shell in the dev container (works even when dev is stopped).
# Note: `docker compose run` does not apply `ports`, so localhost:3000 is not
# forwarded from this shell — run the app with `cargo run` inside it instead.
sh:
	docker compose run --rm dev sh

# Run the test suite in the dev image (needs the postgres service up:
# `docker compose up -d postgres` first)
test:
	docker compose run --rm dev cargo test --workspace

# The full CI pipeline in one shot, in the order CI runs it: fmt, clippy,
# docs, tidy, tests. Needs the postgres service up (`docker compose up -d
# postgres` first); Tidy itself needs no database, but the tests do.
verify:
	docker compose run --rm dev sh -c 'cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features && cargo run -p tidy && cargo test --workspace --all-features'

# Run the architecture checker (slice boundaries, wire-format DTOs, OpenAPI
# drift — see tools/tidy). No database needed.
tidy:
	docker compose run --rm dev cargo run -p tidy

# Regenerate src/features/declared_paths.rs after adding/renaming a route.
tidy-update:
	docker compose run --rm dev cargo run -p tidy -- --update
