# Site backend

Minimal HTTP + SQLite service powering the site's interactive bits:
the guestbook, per-post view counts, likes, and the about-page totals.
Written in Rust (axum + rusqlite); the `bundled` SQLite is compiled into
the crate, so no system SQLite is needed and a musl build yields a fully
static binary.

```bash
cargo build --release
./target/release/backend
```

The service listens on `127.0.0.1:8787` and stores data in
`data/guestbook.db`, relative to its working directory. Override these defaults
when needed:

```bash
GUESTBOOK_ADDR=127.0.0.1:8787 \
GUESTBOOK_DB_PATH=/var/lib/suzuka/backend.db \
./target/release/backend
```

`GUESTBOOK_STATIC_DIR` picks between two routing modes (see `src/server.rs`):

- **Unset** (pure API): routes are exposed directly at the paths below, e.g.
  `GET /messages`. Note that the site's frontend always fetches under the
  `/api/guestbook/` prefix, so this mode alone cannot serve the guestbook UI —
  to exercise the frontend against the backend locally, use the
  single-container mode below (`GUESTBOOK_STATIC_DIR=../public cargo run`
  from this directory, then browse `http://127.0.0.1:8787/`).
- **Set to a directory** (single-container production mode, as run by the
  root `Containerfile`): the backend also serves that directory's static
  files at `/`, and the same routes are nested under `/api/guestbook/`
  (e.g. `GET /api/guestbook/messages`). The crate strips that prefix itself
  via axum's `.nest()` — no nginx (or any reverse proxy) rewrite is involved.

Routes (shown without the `/api/guestbook` prefix; add it back in
single-container mode):

- `GET|POST /messages` — guestbook. `GET` supports cursor pagination with
  `?limit=50&before_id=<id>` and returns `messages`, `next_before_id`, and
  `total_count`. Omitting both parameters keeps the legacy array response.
- `GET|POST /views` — per-post read counts (`?path=/posts/.../`)
- `GET|POST /reactions` — per-post likes
- `GET /summary` — site-wide totals for views and reactions
- `GET /healthz` — liveness probe; runs `SELECT 1` against the database

API requests are logged to stdout, one line each (UTC timestamp, client IP,
method, path, status, duration); errors go to stderr with the same timestamp
format. Static file requests are not logged.

The service only accepts `X-Forwarded-For` from loopback or private-network
peers. The reverse proxy must overwrite, rather than append to, any incoming
client header before forwarding the request. Database schema changes are
versioned with SQLite `PRAGMA user_version` and applied at startup. The
database file and schema are unchanged from the previous Go implementation;
existing data carries over as-is.

Back up the `/data` volume regularly with a SQLite-aware snapshot or backup
tool. Copying only `guestbook.db` while the service is running can miss data
that is still in the WAL file.
