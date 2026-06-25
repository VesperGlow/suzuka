package main

import (
	"database/sql"
	"errors"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// app 持有服务共享的依赖：数据库、时间源，以及留言与计数接口各自的限流器。
type app struct {
	db             *sql.DB
	now            func() time.Time
	limiter        *rateLimiter
	counterLimiter *rateLimiter
}

func main() {
	addr := envOrDefault("GUESTBOOK_ADDR", "127.0.0.1:8787")
	dbPath := envOrDefault("GUESTBOOK_DB_PATH", filepath.Join("data", "guestbook.db"))

	db, err := openDatabase(dbPath)
	if err != nil {
		log.Fatal(err)
	}
	defer db.Close()

	server := &http.Server{
		Addr:              addr,
		Handler:           newApp(db),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       10 * time.Second,
		WriteTimeout:      15 * time.Second,
		IdleTimeout:       60 * time.Second,
	}

	log.Printf("guestbook service listening on %s (database: %s)", addr, dbPath)
	if err := server.ListenAndServe(); !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
}

func envOrDefault(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}

// newApp 组装路由：留言、阅读数、喜欢，以及「关于」页的汇总数据。
func newApp(db *sql.DB) http.Handler {
	a := &app{
		db:             db,
		now:            time.Now,
		limiter:        newRateLimiter(postBurst, postWindow, time.Now),
		counterLimiter: newRateLimiter(counterBurst, counterWindow, time.Now),
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/messages", a.handleMessages)
	mux.HandleFunc("/views", a.handleCounter("page_views"))
	mux.HandleFunc("/reactions", a.handleCounter("reactions"))
	mux.HandleFunc("/summary", a.handleSummary)
	return securityHeaders(mux)
}

func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Cache-Control", "no-store")
		next.ServeHTTP(w, r)
	})
}
