# rustweb

A minimal Rust web API template using vertical slice architecture (VSA).

## Stack

- **Rust 2024** (toolchain pinned in `rust-toolchain.toml`)
- **axum 0.8** — HTTP routing, extractors, middleware
- **SeaORM + Postgres** — entities, transactions, migrations (chrono timestamps)
- **tower-sessions + Postgres store** — session-based auth; the session table
  shares the same pool as SeaORM
- **argon2** — password hashing (argon2id)
- **tower-governor** — per-IP rate limiting (GCRA) on `/auth/*`
- **utoipa** — OpenAPI spec generated from handler attributes, served at `/openapi.json`
- **tracing / tower-http** — request logging via `TraceLayer`
- **proptest + tower::oneshot** — property tests and in-memory DB slice tests

## Layout

```
src/
  auth.rs            CurrentUser extractor, session helpers, reset-token helpers
  config.rs          typed Config loaded from the environment at startup
  mail.rs            Mailer trait + LogMailer (dev mailer; logs reset links)
  features/          one folder per slice: mod.rs wiring + handler (HTTP); service.rs when there's domain logic
    auth/            register, login, logout, me, password-reset request/confirm
  entities/          SeaORM entities and relations
  router.rs          OpenAPI-aware router assembly, session + rate-limit middleware
  error.rs           HTTP errors (slice boundary)
  service_error.rs   domain errors (knows nothing about HTTP)
  state.rs           shared AppState (DB connection, mailer, config)
  main.rs            config, DB connect + migrate, server bootstrap
migration/           SeaORM migration crate, run at startup
```

Slice boundaries: handler → service → DB. Handlers map `ServiceError` to `HttpError`; only services touch SeaORM and validation.

## Configuration

All configuration is read from the environment once at startup by
`Config::from_env`; malformed or out-of-range values fail fast rather than
silently defaulting. See `.env.example` for the full list.

| Variable | Default | Meaning |
| --- | --- | --- |
| `DATABASE_URL` | — (required) | Postgres connection string |
| `PORT` | `3000` | TCP port to listen on |
| `COOKIE_SECURE` | `false` | Mark the session cookie `Secure` (set behind TLS) |
| `AUTH_RATE_LIMIT_PER_SECOND` | `5` | Per-IP refill rate on `/auth/*` |
| `AUTH_RATE_LIMIT_BURST` | `10` | Per-IP burst size on `/auth/*` |
| `PASSWORD_RESET_TOKEN_TTL_MINUTES` | `60` | Lifetime of a reset token (1–1440) |
| `PUBLIC_BASE_URL` | `http://localhost:3000` | Origin for reset links in emails |

## Auth

Session-based auth with server-side sessions in Postgres (`tower_sessions` +
`tower-sessions-sqlx-store`, sharing the SeaORM pool). The `users.password_hash`
column holds an argon2id hash and is never serialized — handlers return a
`UserResponse` DTO instead of the entity.

- `POST /auth/register`, `POST /auth/login` — issue a signed `id` session
  cookie (rotated on login to prevent fixation)
- `POST /auth/logout` — destroys the session server-side and clears the cookie
- `GET /auth/me` — the authenticated user
- Protected handlers take `CurrentUser` (401 without a valid session);
  ownership rules (e.g. updating/deleting yourself) are checked per slice
- Routes live under `/me/...` — user identity comes from the session, not the
  path

### Password reset

- `POST /auth/password-reset/request` `{email}` — always answers 202 for
  well-formed requests, whether or not the email exists, so the endpoint
  cannot be used to enumerate accounts by status code. Unknown emails
  silently send nothing. Response timing still differs (only known emails
  incur a DB write and a mailer call); the per-IP rate limit keeps the
  difference impractical to exploit.
- `POST /auth/password-reset/confirm` `{token, password}` — 204 on success.

Details worth knowing:

- Tokens are 32 random bytes, sha256-hashed at rest (the raw token exists only
  in the email); a leaked `password_reset_tokens` table yields nothing usable.
- Single-use, default 1h TTL (`PASSWORD_RESET_TOKEN_TTL_MINUTES`), and each
  new request invalidates any previous outstanding token for that user.
- Invalid and expired tokens get the same 400 so the endpoint cannot
  distinguish them.
- The dev mailer (`LogMailer`) logs a ready-to-paste `curl` for the confirm
  endpoint instead of sending mail — **it must be replaced with a real
  transport before real users arrive**, or every reset token ends up in the
  logs. Swap `Arc::new(LogMailer)` in `main.rs` for an SMTP/API mailer.
- Sessions issued before a reset stay valid: tower-sessions has no
  per-user session index, so revoking them needs a session-table redesign.
- Users that predate the auth migration have an empty `password_hash` and
  cannot log in — password reset is exactly the recovery path for them.

### Deliberate duplication

Validation is duplicated per slice (`register` vs `update_user` vs
`forgot_password`, for example) on purpose: slices are expected to diverge —
register may gain fields update never has, and vice versa — so sharing up
front couples slices for a divergence that may never come. Duplication costs a
few lines; premature sharing costs a refactor. Extract shared validation only
when a third slice actually needs it (rule of three). Password-reset pushes
the email-format check to three slices, but the three slices validate
different subsets, so they stay separate until one check genuinely
converges.

### Rate limiting

Every `/auth/*` route (register, login, logout, me, password reset) sits
behind a per-IP GCRA bucket (tower-governor), throttling credential-stuffing
and enumeration while the rest of the API stays open. Responses carry
`x-ratelimit-*`/`retry-after` headers. Tuned via
`AUTH_RATE_LIMIT_PER_SECOND`/`AUTH_RATE_LIMIT_BURST`; the key is the TCP peer
IP (`ConnectInfo` — see the graceful shutdown note below).

Behind a reverse proxy the peer is the proxy, so all of its users share one
bucket. Configure the proxy to overwrite client-supplied `X-Forwarded-For`
and swap in tower-governor's `SmartIpKeyExtractor` when that matters.

Dev notes:

- Cookie attributes: HttpOnly, SameSite=Strict (both defaults), 14-day
  inactivity expiry (`Expiry::OnInactivity`), `Secure` via `COOKIE_SECURE`.
- No CSRF token yet: with SameSite=Strict and same-origin SPA dev this is not
  exploitable; if the SPA moves to another origin, add `tower-http`'s
  `CsrfLayer` and set SameSite=None + Secure.
- `tower-sessions` is pinned to 0.14.x — 0.15's core is ahead of
  `tower-sessions-sqlx-store` and won't compile against it.
- Login does not equalize timing between "unknown email" and "wrong password"
  (a valid hash costs one argon2 verify more). Cheap to add when it matters.

## Running

```sh
export DATABASE_URL=postgres://rustweb:rustweb@localhost:5432/rustweb   # or copy .env.example… to your shell
docker compose up -d postgres   # start the database
cargo run
```

The app runs migrations at startup and listens on `0.0.0.0:$PORT`. The
OpenAPI spec is at `http://localhost:3000/openapi.json`.

The server shuts down gracefully on SIGINT/SIGTERM: in-flight requests are
drained, then the pool closes. (The rate limiter keys on `ConnectInfo`, which
is only populated via `into_make_service_with_connect_info` — keep that call
when adding listeners.)

### Docker dev loop

Requires Docker (with compose v2) and [`just`](https://github.com/casey/just).

```sh
just dev   # build the dev image, then live-reload on every save (Ctrl-C to stop)
```

`just dev` runs a `dev` service that bind-mounts `src/`, `migration/`, and the manifests into a toolchain image and runs `cargo watch -x run`: each save recompiles just the crate tree (deps are prebuilt into the image, mold links it) and restarts the app.

Notes:

- The dev service runs the app against Postgres (a `postgres` service, data
  persisted in the `pg_data` volume; postgres 18 keeps data directly in
  `/var/lib/postgresql`, which is what the volume mounts). Set `PORT` in
  `.env` to change the published port (default 3000); `POSTGRES_PORT` does the
  same for postgres (default 5432).
- A src-only save rebuilds just the two workspace crates. Editing `Cargo.toml`/`Cargo.lock` re-resolves and rebuilds the changed deps in the container and invalidates the image's dep-cache layer, so the next `just dev` takes minutes.
- The `/app/target` volume keeps the in-container rebuild cache across container recreations; after a dep change or toolchain bump, `just remove` drops it (and the `pg_data` DB) so the next `just dev` re-seeds from the image.
- The first start touches the mounted sources so cargo rebuilds over the baked stubs; this compiles both workspace crates for real (the image primes them as stubs) and bumps file mtimes on the host once (harmless — git tracks content). A build-id in the `dev_target` volume — the image's toolchain + lockfile + profile fingerprint — skips the touch on later restarts, and re-seeding that volume clears it, so `just remove` never leaves a stale stub. If the image is rebuilt while the volume persists, the dev loop warns once and rebuilds lazily (the shadowed image prime can't be re-seeded from inside the container).
- `just sh` opens a shell in a fresh dev container, no loop running required. The image carries the repo's toolchain spec (`rust-toolchain.toml`), including clippy and rustfmt.

## Tests

```sh
cargo test --workspace --all-features
```

`tools/tidy` (see AGENTS.md) is the architecture checker; CI runs it between
clippy and the tests. `just tidy-update` regenerates the OpenAPI path list
after a route changes.

Every slice has service-level and handler-level tests against a fresh scratch
Postgres database (created per test from `DATABASE_URL`, dropped on teardown —
a failed drop fails the test, so leaks never rot silently); run
`docker compose up -d postgres` first. CI (a postgres service, `cargo fmt`,
`clippy -D warnings`, rustdoc, tests) enforces `-D warnings`.

Test requests pretend to come from `127.0.0.1` (`test_support::request`), the
key the rate limiter counts on; `request_from` speaks as other IPs for the
rate-limit tests. Tests use a config with generous limits so unrelated tests
never trip the limiter.

## Deliberately excluded

- **Swagger UI** — raw `/openapi.json` is the only spec surface
- **Roles/permissions** — ownership checks only; add a `role` column and a
  `require_role` helper when a second role appears
- **Email verification / real mail transport** — `LogMailer` logs reset links
  in dev; the `Mailer` trait in `src/mail.rs` is the seam for SMTP/API
- **CSRF tokens** — see the auth section; SameSite=Strict covers same-origin
