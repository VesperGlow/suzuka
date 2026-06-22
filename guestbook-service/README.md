# Guestbook service

Minimal HTTP and SQLite service for the site's single guestbook.
Building `github.com/mattn/go-sqlite3` requires CGO and a C compiler.

```bash
CGO_ENABLED=1 go build -o guestbook-service .
./guestbook-service
```

The service listens on `127.0.0.1:8787` and stores data in
`data/guestbook.db`, relative to its working directory. Override these defaults
when needed:

```bash
GUESTBOOK_ADDR=127.0.0.1:8787 \
GUESTBOOK_DB_PATH=/var/lib/suzuka-guestbook/guestbook.db \
./guestbook-service
```

Caddy is expected to strip `/api/guestbook` before proxying, so the service
itself exposes `GET /messages` and `POST /messages`.
