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

// newApp 组装服务依赖并返回 HTTP 处理器。
func newApp(db *sql.DB) http.Handler {
	a := &app{
		db:             db,
		now:            time.Now,
		limiter:        newRateLimiter(postBurst, postWindow, time.Now),
		counterLimiter: newRateLimiter(counterBurst, counterWindow, time.Now),
	}
	return a.handler()
}

// handler 组装路由：留言、阅读数、喜欢，以及「关于」页的汇总数据。
// 用 Go 1.22+ ServeMux 的「方法 + 路径」模式注册，方法不匹配时由标准库
// 自动返回 405 并填好 Allow 头，无需在各处理器里手动 switch。
func (a *app) handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /messages", a.listMessages)
	mux.HandleFunc("POST /messages", a.createMessage)
	mux.HandleFunc("GET /views", a.readCounterHandler("page_views"))
	mux.HandleFunc("POST /views", a.bumpCounterHandler("page_views"))
	mux.HandleFunc("GET /reactions", a.readCounterHandler("reactions"))
	mux.HandleFunc("POST /reactions", a.bumpCounterHandler("reactions"))
	mux.HandleFunc("GET /summary", a.handleSummary)
	return securityHeaders(mux)
}

func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Cache-Control", "no-store")
		next.ServeHTTP(w, r)
	})
}
