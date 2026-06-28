package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"mime"
	"net/http"
	"net/mail"
	"net/url"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
)

const (
	// 每个来源 IP 在 postWindow 内最多允许 postBurst 次留言提交。
	postBurst          = 5
	postWindow         = time.Minute
	messagePageSize    = 50
	maxMessagePageSize = 100
	legacyMessageLimit = 500
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

type messagePage struct {
	Messages     []message `json:"messages"`
	NextBeforeID int64     `json:"next_before_id,omitempty"`
	TotalCount   int64     `json:"total_count"`
}

func (a *app) listMessages(w http.ResponseWriter, r *http.Request) {
	paginated := r.URL.Query().Has("limit") || r.URL.Query().Has("before_id")
	limit, beforeID := legacyMessageLimit, int64(0)
	if paginated {
		var err error
		limit, beforeID, err = messagePageParams(r)
		if err != nil {
			writeError(w, http.StatusBadRequest, err.Error())
			return
		}
	}

	query := `
SELECT id, name, email, website, content, ref_title, ref_url, created_at
FROM messages
ORDER BY id DESC
LIMIT ?`
	args := []any{limit + 1}
	if beforeID > 0 {
		query = `
SELECT id, name, email, website, content, ref_title, ref_url, created_at
FROM messages
WHERE id < ?
ORDER BY id DESC
LIMIT ?`
		args = []any{beforeID, limit + 1}
	}

	rows, err := a.db.QueryContext(r.Context(), query, args...)
	if err != nil {
		a.writeInternalError(w, r, "unable to load messages", err)
		return
	}
	defer rows.Close()

	messages := make([]message, 0)
	for rows.Next() {
		var item message
		var privateEmail string
		if err := rows.Scan(&item.ID, &item.Name, &privateEmail, &item.Website, &item.Content, &item.RefTitle, &item.RefURL, &item.CreatedAt); err != nil {
			a.writeInternalError(w, r, "unable to load messages", err)
			return
		}
		messages = append(messages, item)
	}
	if err := rows.Err(); err != nil {
		a.writeInternalError(w, r, "unable to load messages", err)
		return
	}
	if err := rows.Close(); err != nil {
		a.writeInternalError(w, r, "unable to load messages", err)
		return
	}

	if !paginated {
		if len(messages) > limit {
			messages = messages[:limit]
		}
		writeJSON(w, http.StatusOK, messages)
		return
	}

	page := messagePage{Messages: messages}
	if len(messages) > limit {
		page.Messages = messages[:limit]
		page.NextBeforeID = messages[limit-1].ID
	}
	if err := a.db.QueryRowContext(r.Context(), "SELECT COUNT(*) FROM messages").Scan(&page.TotalCount); err != nil {
		a.writeInternalError(w, r, "unable to load messages", err)
		return
	}

	writeJSON(w, http.StatusOK, page)
}

func messagePageParams(r *http.Request) (limit int, beforeID int64, err error) {
	limit = messagePageSize
	if value := strings.TrimSpace(r.URL.Query().Get("limit")); value != "" {
		limit, err = strconv.Atoi(value)
		if err != nil || limit < 1 || limit > maxMessagePageSize {
			return 0, 0, fmt.Errorf("limit must be between 1 and %d", maxMessagePageSize)
		}
	}
	if value := strings.TrimSpace(r.URL.Query().Get("before_id")); value != "" {
		beforeID, err = strconv.ParseInt(value, 10, 64)
		if err != nil || beforeID < 1 {
			return 0, 0, errors.New("before_id must be a positive integer")
		}
	}
	return limit, beforeID, nil
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
		a.writeInternalError(w, r, "unable to save message", err)
		return
	}
	item.ID, err = result.LastInsertId()
	if err != nil {
		a.writeInternalError(w, r, "unable to save message", err)
		return
	}
	item.Email = ""

	writeJSON(w, http.StatusCreated, item)
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
	if item.Email != "" {
		address, err := mail.ParseAddress(item.Email)
		if err != nil || address.Address != item.Email {
			return errors.New("email must be a valid email address")
		}
	}
	if item.Website != "" {
		parsed, err := url.Parse(item.Website)
		if err != nil {
			return errors.New("website must be an http or https URL")
		}
		scheme := strings.ToLower(parsed.Scheme)
		if (scheme != "http" && scheme != "https") || parsed.Hostname() == "" || parsed.User != nil {
			return errors.New("website must be an http or https URL")
		}
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
