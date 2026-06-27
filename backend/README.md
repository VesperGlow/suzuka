# Site backend

Minimal HTTP + SQLite service powering the site's interactive bits:
the guestbook, per-post view counts, likes, and the about-page totals.
Uses the pure-Go `modernc.org/sqlite` driver, so no CGO or C compiler is
needed and the binary cross-compiles cleanly.

```bash
CGO_ENABLED=0 go build -o backend .
./backend
```

The service listens on `127.0.0.1:8787` and stores data in
`data/guestbook.db`, relative to its working directory. Override these defaults
when needed:

```bash
GUESTBOOK_ADDR=127.0.0.1:8787 \
GUESTBOOK_DB_PATH=/var/lib/suzuka/backend.db \
./backend
```

nginx strips `/api/guestbook` before proxying, so the service itself exposes:

- `GET|POST /messages` — guestbook
- `GET|POST /views` — per-post read counts (`?path=/posts/.../`)
- `GET|POST /reactions` — per-post likes
- `GET /summary` — site-wide totals for views and reactions
