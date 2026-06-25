# Site backend

Minimal HTTP + SQLite service powering the site's interactive bits:
the guestbook, per-post view counts, likes, and the about-page totals.
Building `github.com/mattn/go-sqlite3` requires CGO and a C compiler.

```bash
CGO_ENABLED=1 go build -o api .
./api
```

The service listens on `127.0.0.1:8787` and stores data in
`data/guestbook.db`, relative to its working directory. Override these defaults
when needed:

```bash
GUESTBOOK_ADDR=127.0.0.1:8787 \
GUESTBOOK_DB_PATH=/var/lib/suzuka/api.db \
./api
```

nginx strips `/api/guestbook` before proxying, so the service itself exposes:

- `GET|POST /messages` — guestbook
- `GET|POST /views` — per-post read counts (`?path=/posts/.../`)
- `GET|POST /reactions` — per-post likes
- `GET /summary` — site-wide totals for views and reactions
