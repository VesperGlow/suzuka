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

nginx strips `/api/guestbook` before proxying, so the service itself exposes:

- `GET|POST /messages` — guestbook. `GET` supports cursor pagination with
  `?limit=50&before_id=<id>` and returns `messages`, `next_before_id`, and
  `total_count`. Omitting both parameters keeps the legacy array response.
- `GET|POST /views` — per-post read counts (`?path=/posts/.../`)
- `GET|POST /reactions` — per-post likes
- `GET /summary` — site-wide totals for views and reactions

The service only accepts `X-Forwarded-For` from loopback or private-network
peers. The reverse proxy must overwrite, rather than append to, any incoming
client header before forwarding the request. Database schema changes are
versioned with SQLite `PRAGMA user_version` and applied at startup. The
database file and schema are unchanged from the previous Go implementation;
existing data carries over as-is.

Back up the `/data` volume regularly with a SQLite-aware snapshot or backup
tool. Copying only `guestbook.db` while the service is running can miss data
that is still in the WAL file.
