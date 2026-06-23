package main

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"mime"
	"net"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	_ "github.com/mattn/go-sqlite3"
)

const (
	maxRequestBytes = 16 << 10
	// 每个来源 IP 在 postWindow 内最多允许 postBurst 次留言提交。
	postBurst  = 5
	postWindow = time.Minute
)

type message struct {
	ID        int64  `json:"id"`
	Name      string `json:"name"`
	Email     string `json:"email,omitempty"`
	Website   string `json:"website,omitempty"`
	Content   string `json:"content"`
	RefTitle  string `json:"ref_title,omitempty"`
	RefURL    string `json:"ref_url,omitempty"`
	CreatedAt string `json:"created_at"`
}

type app struct {
	db      *sql.DB
	now     func() time.Time
	limiter *rateLimiter
}

// rateLimiter 是一个按 key 的滑动窗口限流器，用于挡住对写接口的滥用。
// 注意：客户端 IP 取自 X-Forwarded-For，依赖上游反向代理正确设置该头，
// 该头可被伪造，因此这只是基础的防滥用，不是安全边界。
type rateLimiter struct {
	mu        sync.Mutex
	limit     int
	window    time.Duration
	now       func() time.Time
	hits      map[string][]time.Time
	lastSweep time.Time
}

func newRateLimiter(limit int, window time.Duration, now func() time.Time) *rateLimiter {
	return &rateLimiter{
		limit:     limit,
		window:    window,
		now:       now,
		hits:      make(map[string][]time.Time),
		lastSweep: now(),
	}
}

// allow 记录一次访问并返回是否在限额内。
func (rl *rateLimiter) allow(key string) bool {
	rl.mu.Lock()
	defer rl.mu.Unlock()

	now := rl.now()
	cutoff := now.Add(-rl.window)

	if now.Sub(rl.lastSweep) >= rl.window {
		rl.sweep(cutoff)
		rl.lastSweep = now
	}

	recent := rl.hits[key][:0]
	for _, t := range rl.hits[key] {
		if t.After(cutoff) {
			recent = append(recent, t)
		}
	}
	if len(recent) >= rl.limit {
		rl.hits[key] = recent
		return false
	}
	rl.hits[key] = append(recent, now)
	return true
}

// sweep 删除窗口内已无访问记录的 key，避免 map 无限增长。
func (rl *rateLimiter) sweep(cutoff time.Time) {
	for key, times := range rl.hits {
		kept := times[:0]
		for _, t := range times {
			if t.After(cutoff) {
				kept = append(kept, t)
			}
		}
		if len(kept) == 0 {
			delete(rl.hits, key)
		} else {
			rl.hits[key] = kept
		}
	}
}

func clientIP(r *http.Request) string {
	if forwarded := r.Header.Get("X-Forwarded-For"); forwarded != "" {
		if comma := strings.IndexByte(forwarded, ','); comma >= 0 {
			forwarded = forwarded[:comma]
		}
		if ip := strings.TrimSpace(forwarded); ip != "" {
			return ip
		}
	}
	if host, _, err := net.SplitHostPort(r.RemoteAddr); err == nil {
		return host
	}
	return r.RemoteAddr
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

func openDatabase(path string) (*sql.DB, error) {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o750); err != nil {
		return nil, fmt.Errorf("create database directory: %w", err)
	}

	db, err := sql.Open("sqlite3", path+"?_busy_timeout=5000&_journal_mode=WAL&_foreign_keys=on")
	if err != nil {
		return nil, fmt.Errorf("open database: %w", err)
	}
	db.SetMaxOpenConns(1)

	const schema = `
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    email TEXT NOT NULL DEFAULT '',
    website TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    ref_title TEXT NOT NULL DEFAULT '',
    ref_url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at DESC, id DESC);`
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return nil, fmt.Errorf("initialize database: %w", err)
	}
	return db, nil
}

func newApp(db *sql.DB) http.Handler {
	a := &app{db: db, now: time.Now, limiter: newRateLimiter(postBurst, postWindow, time.Now)}
	mux := http.NewServeMux()
	mux.HandleFunc("/messages", a.handleMessages)
	return securityHeaders(mux)
}

func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Cache-Control", "no-store")
		next.ServeHTTP(w, r)
	})
}

func (a *app) handleMessages(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/messages" {
		writeError(w, http.StatusNotFound, "not found")
		return
	}

	switch r.Method {
	case http.MethodGet:
		a.listMessages(w, r)
	case http.MethodPost:
		a.createMessage(w, r)
	default:
		w.Header().Set("Allow", "GET, POST")
		writeError(w, http.StatusMethodNotAllowed, "method not allowed")
	}
}

func (a *app) listMessages(w http.ResponseWriter, r *http.Request) {
	rows, err := a.db.QueryContext(r.Context(), `
SELECT id, name, email, website, content, ref_title, ref_url, created_at
FROM messages
ORDER BY created_at DESC, id DESC
LIMIT 500`)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "unable to load messages")
		return
	}
	defer rows.Close()

	messages := make([]message, 0)
	for rows.Next() {
		var item message
		var privateEmail string
		if err := rows.Scan(&item.ID, &item.Name, &privateEmail, &item.Website, &item.Content, &item.RefTitle, &item.RefURL, &item.CreatedAt); err != nil {
			writeError(w, http.StatusInternalServerError, "unable to load messages")
			return
		}
		messages = append(messages, item)
	}
	if err := rows.Err(); err != nil {
		writeError(w, http.StatusInternalServerError, "unable to load messages")
		return
	}

	writeJSON(w, http.StatusOK, messages)
}

func (a *app) createMessage(w http.ResponseWriter, r *http.Request) {
	if a.limiter != nil && !a.limiter.allow(clientIP(r)) {
		w.Header().Set("Retry-After", strconv.Itoa(int(postWindow.Seconds())))
		writeError(w, http.StatusTooManyRequests, "too many requests, please try again later")
		return
	}

	mediaType, _, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if err != nil || mediaType != "application/json" {
		writeError(w, http.StatusUnsupportedMediaType, "content type must be application/json")
		return
	}

	r.Body = http.MaxBytesReader(w, r.Body, maxRequestBytes)
	decoder := json.NewDecoder(r.Body)
	decoder.DisallowUnknownFields()

	var item message
	if err := decoder.Decode(&item); err != nil {
		writeError(w, http.StatusBadRequest, "invalid request body")
		return
	}
	if err := ensureJSONEnd(decoder); err != nil {
		writeError(w, http.StatusBadRequest, "invalid request body")
		return
	}

	item.ID = 0
	item.CreatedAt = ""
	item.Name = strings.TrimSpace(item.Name)
	item.Email = strings.TrimSpace(item.Email)
	item.Website = strings.TrimSpace(item.Website)
	item.Content = strings.TrimSpace(item.Content)
	item.RefTitle = strings.TrimSpace(item.RefTitle)
	item.RefURL = strings.TrimSpace(item.RefURL)
	if err := validateMessage(item); err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}

	item.CreatedAt = a.now().UTC().Format(time.RFC3339Nano)
	result, err := a.db.ExecContext(r.Context(), `
INSERT INTO messages (name, email, website, content, ref_title, ref_url, created_at)
VALUES (?, ?, ?, ?, ?, ?, ?)`, item.Name, item.Email, item.Website, item.Content, item.RefTitle, item.RefURL, item.CreatedAt)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "unable to save message")
		return
	}
	item.ID, err = result.LastInsertId()
	if err != nil {
		writeError(w, http.StatusInternalServerError, "unable to save message")
		return
	}
	item.Email = ""

	writeJSON(w, http.StatusCreated, item)
}

func ensureJSONEnd(decoder *json.Decoder) error {
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return errors.New("request body must contain one JSON object")
	}
	return nil
}

func validateMessage(item message) error {
	if item.Name == "" {
		return errors.New("name is required")
	}
	if item.Content == "" {
		return errors.New("content is required")
	}
	if (item.RefTitle == "") != (item.RefURL == "") {
		return errors.New("ref_title and ref_url must be provided together")
	}
	if item.RefURL != "" && !validPostURL(item.RefURL) {
		return errors.New("ref_url must be a relative /posts/ path")
	}

	limits := []struct {
		name  string
		value string
		max   int
	}{
		{"name", item.Name, 40},
		{"email", item.Email, 120},
		{"website", item.Website, 200},
		{"content", item.Content, 2000},
		{"ref_title", item.RefTitle, 200},
		{"ref_url", item.RefURL, 300},
	}
	for _, field := range limits {
		if !utf8.ValidString(field.value) {
			return fmt.Errorf("%s must be valid UTF-8", field.name)
		}
		if utf8.RuneCountInString(field.value) > field.max {
			return fmt.Errorf("%s must be at most %d characters", field.name, field.max)
		}
	}
	return nil
}

func validPostURL(value string) bool {
	parsed, err := url.Parse(value)
	if err != nil || parsed.IsAbs() || parsed.Host != "" || parsed.RawQuery != "" || parsed.Fragment != "" {
		return false
	}
	cleaned := path.Clean(parsed.Path)
	return strings.HasPrefix(value, "/posts/") && strings.HasPrefix(cleaned, "/posts/")
}

func writeError(w http.ResponseWriter, status int, text string) {
	writeJSON(w, status, map[string]string{"error": text})
}

func writeJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(value); err != nil {
		log.Printf("write JSON response: %v", err)
	}
}
