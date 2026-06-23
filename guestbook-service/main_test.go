package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func testApp(t *testing.T) http.Handler {
	t.Helper()
	db, err := openDatabase(filepath.Join(t.TempDir(), "guestbook.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	a := &app{db: db, now: func() time.Time {
		return time.Date(2026, 6, 22, 12, 0, 0, 0, time.UTC)
	}}
	mux := http.NewServeMux()
	mux.HandleFunc("/messages", a.handleMessages)
	return securityHeaders(mux)
}

func TestCreateAndListMessages(t *testing.T) {
	handler := testApp(t)
	body := `{"name":"Suzuka","email":"","website":"","content":"hello <b>world</b>","ref_title":"An article","ref_url":"/posts/example/"}`
	request := httptest.NewRequest(http.MethodPost, "/messages", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusCreated {
		t.Fatalf("POST status = %d, body = %s", response.Code, response.Body.String())
	}

	request = httptest.NewRequest(http.MethodGet, "/messages", nil)
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("GET status = %d, body = %s", response.Code, response.Body.String())
	}
	var messages []message
	if err := json.NewDecoder(response.Body).Decode(&messages); err != nil {
		t.Fatal(err)
	}
	if len(messages) != 1 || messages[0].Content != "hello <b>world</b>" || messages[0].RefURL != "/posts/example/" || messages[0].Email != "" {
		t.Fatalf("unexpected messages: %#v", messages)
	}
}

func TestRateLimit(t *testing.T) {
	now := time.Date(2026, 6, 22, 12, 0, 0, 0, time.UTC)
	db, err := openDatabase(filepath.Join(t.TempDir(), "guestbook.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	a := &app{db: db, now: func() time.Time { return now }, limiter: newRateLimiter(postBurst, postWindow, func() time.Time { return now })}
	mux := http.NewServeMux()
	mux.HandleFunc("/messages", a.handleMessages)
	handler := securityHeaders(mux)

	post := func() int {
		body := `{"name":"Suzuka","content":"hello"}`
		request := httptest.NewRequest(http.MethodPost, "/messages", strings.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("X-Forwarded-For", "203.0.113.7")
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		return response.Code
	}

	for i := 0; i < postBurst; i++ {
		if code := post(); code != http.StatusCreated {
			t.Fatalf("request %d status = %d", i+1, code)
		}
	}
	if code := post(); code != http.StatusTooManyRequests {
		t.Fatalf("over-limit status = %d, want 429", code)
	}

	now = now.Add(postWindow + time.Second)
	if code := post(); code != http.StatusCreated {
		t.Fatalf("after window status = %d, want 201", code)
	}
}

func TestValidation(t *testing.T) {
	handler := testApp(t)
	tests := []struct {
		name string
		body string
	}{
		{"missing name", `{"name":"","content":"hello"}`},
		{"missing content", `{"name":"Suzuka","content":""}`},
		{"name too long", `{"name":"` + strings.Repeat("界", 41) + `","content":"hello"}`},
		{"content too long", `{"name":"Suzuka","content":"` + strings.Repeat("a", 2001) + `"}`},
		{"reference title only", `{"name":"Suzuka","content":"hello","ref_title":"Article"}`},
		{"external reference", `{"name":"Suzuka","content":"hello","ref_title":"Article","ref_url":"https://example.com/posts/a/"}`},
		{"non-post reference", `{"name":"Suzuka","content":"hello","ref_title":"Article","ref_url":"/about/"}`},
		{"traversing reference", `{"name":"Suzuka","content":"hello","ref_title":"Article","ref_url":"/posts/../about/"}`},
		{"unknown field", `{"name":"Suzuka","content":"hello","admin":true}`},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			request := httptest.NewRequest(http.MethodPost, "/messages", bytes.NewBufferString(test.body))
			request.Header.Set("Content-Type", "application/json")
			response := httptest.NewRecorder()
			handler.ServeHTTP(response, request)
			if response.Code != http.StatusBadRequest {
				t.Fatalf("status = %d, body = %s", response.Code, response.Body.String())
			}
		})
	}
}
