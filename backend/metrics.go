package main

import (
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"mime"
	"net/http"
	"strconv"
	"strings"
	"time"
)

const (
	// 阅读数 / 喜欢这类计数写接口的限流：正常浏览也会触发，因此放得比留言宽松，
	// 仅用于挡住单 IP 的刷量；真正的防自刷靠前端 localStorage 节流。
	counterBurst  = 60
	counterWindow = time.Minute
)

// counterResponse 是阅读数 / 喜欢计数接口的统一返回体。
type counterResponse struct {
	Path  string `json:"path"`
	Count int64  `json:"count"`
}

// handleCounter 返回某张计数表（page_views / reactions）的读写处理器。
// table 是代码内常量，不来自用户输入，可安全拼入 SQL。
func (a *app) handleCounter(table string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		switch r.Method {
		case http.MethodGet:
			a.readCounter(w, r, table)
		case http.MethodPost:
			a.bumpCounter(w, r, table)
		default:
			w.Header().Set("Allow", "GET, POST")
			writeError(w, http.StatusMethodNotAllowed, "method not allowed")
		}
	}
}

func (a *app) readCounter(w http.ResponseWriter, r *http.Request, table string) {
	pagePath := strings.TrimSpace(r.URL.Query().Get("path"))
	if !validPostURL(pagePath) {
		writeError(w, http.StatusBadRequest, "path must be a relative /posts/ path")
		return
	}

	var count int64
	err := a.db.QueryRowContext(r.Context(),
		fmt.Sprintf("SELECT count FROM %s WHERE path = ?", table), pagePath).Scan(&count)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		writeError(w, http.StatusInternalServerError, "unable to load count")
		return
	}
	writeJSON(w, http.StatusOK, counterResponse{Path: pagePath, Count: count})
}

func (a *app) bumpCounter(w http.ResponseWriter, r *http.Request, table string) {
	if a.counterLimiter != nil && !a.counterLimiter.allow(clientIP(r)) {
		w.Header().Set("Retry-After", strconv.Itoa(int(counterWindow.Seconds())))
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

	var payload struct {
		Path string `json:"path"`
	}
	if err := decoder.Decode(&payload); err != nil {
		writeError(w, http.StatusBadRequest, "invalid request body")
		return
	}
	if err := ensureJSONEnd(decoder); err != nil {
		writeError(w, http.StatusBadRequest, "invalid request body")
		return
	}

	pagePath := strings.TrimSpace(payload.Path)
	if !validPostURL(pagePath) {
		writeError(w, http.StatusBadRequest, "path must be a relative /posts/ path")
		return
	}

	var count int64
	err = a.db.QueryRowContext(r.Context(), fmt.Sprintf(`
INSERT INTO %s (path, count) VALUES (?, 1)
ON CONFLICT(path) DO UPDATE SET count = count + 1
RETURNING count`, table), pagePath).Scan(&count)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "unable to update count")
		return
	}
	writeJSON(w, http.StatusOK, counterResponse{Path: pagePath, Count: count})
}

// handleSummary 返回全站累计的阅读量与喜欢数，供「关于」页展示。只读，无需限流。
func (a *app) handleSummary(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		w.Header().Set("Allow", "GET")
		writeError(w, http.StatusMethodNotAllowed, "method not allowed")
		return
	}
	var summary struct {
		Views     int64 `json:"views"`
		Reactions int64 `json:"reactions"`
	}
	if err := a.db.QueryRowContext(r.Context(),
		"SELECT COALESCE(SUM(count), 0) FROM page_views").Scan(&summary.Views); err != nil {
		writeError(w, http.StatusInternalServerError, "unable to load summary")
		return
	}
	if err := a.db.QueryRowContext(r.Context(),
		"SELECT COALESCE(SUM(count), 0) FROM reactions").Scan(&summary.Reactions); err != nil {
		writeError(w, http.StatusInternalServerError, "unable to load summary")
		return
	}
	writeJSON(w, http.StatusOK, summary)
}
