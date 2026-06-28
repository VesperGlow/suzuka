package main

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log"
	"mime"
	"net/http"
	"os"
	"os/signal"
	"path"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

// app 持有服务共享的依赖：数据库、时间源，以及留言与计数接口各自的限流器。
type app struct {
	db             *sql.DB
	now            func() time.Time
	logger         *log.Logger
	limiter        *rateLimiter
	counterLimiter *rateLimiter
}

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	addr := envOrDefault("GUESTBOOK_ADDR", "127.0.0.1:8787")
	dbPath := envOrDefault("GUESTBOOK_DB_PATH", filepath.Join("data", "guestbook.db"))

	db, err := openDatabase(dbPath)
	if err != nil {
		return err
	}
	defer db.Close()

	server := &http.Server{
		Addr:              addr,
		Handler:           newApp(db),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       10 * time.Second,
		WriteTimeout:      15 * time.Second,
		IdleTimeout:       60 * time.Second,
		MaxHeaderBytes:    32 << 10,
	}

	log.Printf("guestbook service listening on %s (database: %s)", addr, dbPath)
	if err := serveUntilSignal(server); err != nil {
		return fmt.Errorf("serve HTTP: %w", err)
	}
	return nil
}

// serveUntilSignal 在收到终止信号时停止接收新请求，并给在途请求留出完成时间。
func serveUntilSignal(server *http.Server) error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	serveErr := make(chan error, 1)
	go func() { serveErr <- server.ListenAndServe() }()

	select {
	case err := <-serveErr:
		if errors.Is(err, http.ErrServerClosed) {
			return nil
		}
		return err
	case <-ctx.Done():
		log.Printf("shutdown signal received")
	}

	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := server.Shutdown(shutdownCtx); err != nil {
		if closeErr := server.Close(); closeErr != nil {
			log.Printf("force close HTTP server: %v", closeErr)
		}
		return fmt.Errorf("graceful shutdown: %w", err)
	}

	if err := <-serveErr; !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	return nil
}

func envOrDefault(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}

// newApp 组装服务依赖并返回对外的 HTTP 处理器。
func newApp(db *sql.DB) http.Handler {
	a := &app{
		db:             db,
		now:            time.Now,
		logger:         log.Default(),
		limiter:        newRateLimiter(postBurst, postWindow, time.Now),
		counterLimiter: newRateLimiter(counterBurst, counterWindow, time.Now),
	}
	return a.rootHandler()
}

// rootHandler 决定最终对外的处理器形态：
//   - 设置了 GUESTBOOK_STATIC_DIR 时进入「单容器」模式：根路径托管 Hugo 静态产物，
//     留言板 API 收敛到 /api/guestbook/ 前缀下（与前端 fetch 的路径一致，
//     由本进程剥掉前缀，省掉外层 nginx 反代）。
//   - 未设置时退回纯 API 模式（本地开发或仅后端镜像），行为与之前完全一致。
func (a *app) rootHandler() http.Handler {
	api := a.handler()
	staticDir := strings.TrimSpace(os.Getenv("GUESTBOOK_STATIC_DIR"))
	if staticDir == "" {
		return api
	}

	mux := http.NewServeMux()
	mux.Handle("/api/guestbook/", http.StripPrefix("/api/guestbook", api))
	mux.Handle("/", newStaticHandler(staticDir))
	return mux
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

// newStaticHandler 在根路径托管 Hugo 产物。运行用的 scratch 镜像里没有
// /etc/mime.types，这里手动补几个 Go 内建表缺失、但站点会用到的扩展名，
// 避免字体 / manifest / feed 被当成 application/octet-stream 下发。
func newStaticHandler(dir string) http.Handler {
	for ext, typ := range map[string]string{
		".woff":        "font/woff",
		".woff2":       "font/woff2",
		".ttf":         "font/ttf",
		".otf":         "font/otf",
		".ico":         "image/x-icon",
		".webmanifest": "application/manifest+json",
		".xml":         "application/xml",
	} {
		_ = mime.AddExtensionType(ext, typ)
	}

	fileServer := http.FileServer(http.Dir(dir))
	notFoundPage := filepath.Join(dir, "404.html")

	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			w.Header().Set("Allow", "GET, HEAD")
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}

		// 复刻 FileServer 的定位逻辑：先 path.Clean 收敛掉 ".." 防目录穿越，
		// 命中目录时回退到其 index.html。命中则交给 FileServer（自带
		// Last-Modified / Range / 条件请求）；未命中则返回 Hugo 的 404.html，
		// 与原先静态托管的体验保持一致。
		target := filepath.Join(dir, filepath.FromSlash(path.Clean("/"+r.URL.Path)))
		if info, err := os.Stat(target); err == nil && info.IsDir() {
			target = filepath.Join(target, "index.html")
		}
		if _, err := os.Stat(target); err != nil {
			serveNotFound(w, r, notFoundPage)
			return
		}

		w.Header().Set("X-Content-Type-Options", "nosniff")
		fileServer.ServeHTTP(w, r)
	})
}

// serveNotFound 输出站点自带的 404 页面并带上 404 状态码；
// 页面缺失时退回标准库的纯文本 404。
func serveNotFound(w http.ResponseWriter, r *http.Request, page string) {
	w.Header().Set("X-Content-Type-Options", "nosniff")
	body, err := os.ReadFile(page)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.WriteHeader(http.StatusNotFound)
	if r.Method != http.MethodHead {
		_, _ = w.Write(body)
	}
}
