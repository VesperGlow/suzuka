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
GUESTBOOK_ADMIN_TOKEN=$(openssl rand -hex 32) \
./target/release/backend
```

`GUESTBOOK_ADMIN_TOKEN` is optional: when set it enables the
`DELETE /messages/:id` moderation route described below; when unset the
service runs exactly as before, with no admin surface at all.

## Email notifications for new messages

Optional. When `GUESTBOOK_SMTP_USER` and `GUESTBOOK_SMTP_PASSWORD` are both
set, the service emails the site owner whenever a guestbook message is
saved. The email includes the visitor's (otherwise private) email address
and a ready-to-paste `curl` delete command for that message id.

```bash
GUESTBOOK_SMTP_USER=you@gmail.com \
GUESTBOOK_SMTP_PASSWORD=<16-char app password> \
./target/release/backend
```

- `GUESTBOOK_SMTP_RELAY` — SMTP host, defaults to `smtp.gmail.com`.
  Connects on port 465 (implicit TLS/SMTPS); make sure the VPS allows
  outbound 465.
- `GUESTBOOK_NOTIFY_TO` — recipient, defaults to `GUESTBOOK_SMTP_USER`
  (i.e. you mail yourself).
- For Gmail the password must be an **App Password** (Google Account →
  Security → 2-Step Verification → App passwords), not the account
  password. Gmail rewrites `From` to the authenticated account.

Delivery is best-effort: mail is sent from a detached task after the row
is committed, failures only go to stderr, and a global fuse caps
notifications at 12 emails per hour (see `src/notify.rs`) so a spam wave
cannot flood the inbox — excess messages are still stored, just not
emailed. TLS root certificates are compiled into the binary
(`webpki-roots`), so this works from the `scratch` image without a CA
bundle.

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

- `GET|POST /messages` — guestbook. `GET` uses cursor pagination
  (`?limit=50&before_id=<id>`, both optional; limit defaults to 50) and
  returns `messages`, `next_before_id`, and `total_count`.
- `DELETE /messages/:id` — admin-only moderation, enabled by setting
  `GUESTBOOK_ADMIN_TOKEN`; without it the route answers 404. There is no
  admin UI — manage messages with curl:

  ```bash
  curl -X DELETE -H "Authorization: Bearer $GUESTBOOK_ADMIN_TOKEN" \
    https://<site>/api/guestbook/messages/<id>
  ```

  Returns 204 on success, 404 for unknown ids, 401 for a missing or wrong
  token. Failed auth attempts share the guestbook POST rate limiter (per
  IP), so the token cannot be brute-forced quickly; authenticated requests
  are not rate-limited.
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

## Backups

Copying `guestbook.db` while the service is running can miss data that is
still in the WAL file, or even produce an unopenable copy. The service
therefore snapshots its own database once a day via `VACUUM INTO` (see
`src/backup.rs`): a consistent, self-contained database file named like
`guestbook-20260716-120000.db`, written to `<db dir>/backups/` (override
with `GUESTBOOK_BACKUP_DIR`), keeping the newest 7. With snapshots in
place, backing up is just copying/rsyncing the whole data directory — the
live db file in the copy may be torn, but the snapshots never are.

To restore: stop the service, replace `guestbook.db` with a snapshot file
(delete any leftover `-wal`/`-shm` files), start the service.
