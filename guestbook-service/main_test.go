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
	now := func() time.Time { return time.Date(2026, 6, 22, 12, 0, 0, 0, time.UTC) }
	a := &app{db: db, now: now, counterLimiter: newRateLimiter(counterBurst, counterWindow, now)}
	mux := http.NewServeMux()
	mux.HandleFunc("/messages", a.handleMessages)
	mux.HandleFunc("/views", a.handleCounter("page_views"))
	mux.HandleFunc("/reactions", a.handleCounter("reactions"))
	mux.HandleFunc("/summary", a.handleSummary)
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

func TestCounters(t *testing.T) {
	handler := testApp(t)

	bump := func(endpoint, path string) counterResponse {
		t.Helper()
		body, _ := json.Marshal(map[string]string{"path": path})
		request := httptest.NewRequest(http.MethodPost, endpoint, bytes.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("POST %s status = %d, body = %s", endpoint, response.Code, response.Body.String())
		}
		var out counterResponse
		if err := json.NewDecoder(response.Body).Decode(&out); err != nil {
			t.Fatal(err)
		}
		return out
	}

	read := func(endpoint, path string) counterResponse {
		t.Helper()
		request := httptest.NewRequest(http.MethodGet, endpoint+"?path="+path, nil)
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("GET %s status = %d, body = %s", endpoint, response.Code, response.Body.String())
		}
		var out counterResponse
		if err := json.NewDecoder(response.Body).Decode(&out); err != nil {
			t.Fatal(err)
		}
		return out
	}

	// 未记录过的文章读数为 0。
	if got := read("/views", "/posts/example/"); got.Count != 0 {
		t.Fatalf("initial views = %d, want 0", got.Count)
	}
	// 连续自增。
	if got := bump("/views", "/posts/example/"); got.Count != 1 {
		t.Fatalf("first view = %d, want 1", got.Count)
	}
	if got := bump("/views", "/posts/example/"); got.Count != 2 {
		t.Fatalf("second view = %d, want 2", got.Count)
	}
	if got := read("/views", "/posts/example/"); got.Count != 2 {
		t.Fatalf("read-back views = %d, want 2", got.Count)
	}
	// 反应与阅读数互不影响，且按 path 独立计数。
	if got := bump("/reactions", "/posts/example/"); got.Count != 1 {
		t.Fatalf("reaction = %d, want 1", got.Count)
	}
	if got := read("/views", "/posts/example/"); got.Count != 2 {
		t.Fatalf("views after reaction = %d, want 2 (tables must be separate)", got.Count)
	}
	if got := read("/views", "/posts/other/"); got.Count != 0 {
		t.Fatalf("other-path views = %d, want 0", got.Count)
	}
}

func TestSummary(t *testing.T) {
	handler := testApp(t)

	post := func(endpoint, path string) {
		t.Helper()
		body, _ := json.Marshal(map[string]string{"path": path})
		request := httptest.NewRequest(http.MethodPost, endpoint, bytes.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("POST %s status = %d", endpoint, response.Code)
		}
	}

	summary := func() (views, reactions int64) {
		t.Helper()
		request := httptest.NewRequest(http.MethodGet, "/summary", nil)
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("GET /summary status = %d", response.Code)
		}
		var out struct {
			Views     int64 `json:"views"`
			Reactions int64 `json:"reactions"`
		}
		if err := json.NewDecoder(response.Body).Decode(&out); err != nil {
			t.Fatal(err)
		}
		return out.Views, out.Reactions
	}

	// 空库汇总为 0。
	if v, r := summary(); v != 0 || r != 0 {
		t.Fatalf("empty summary = (%d, %d), want (0, 0)", v, r)
	}
	// 跨多篇文章累加。
	post("/views", "/posts/a/")
	post("/views", "/posts/a/")
	post("/views", "/posts/b/")
	post("/reactions", "/posts/a/")
	if v, r := summary(); v != 3 || r != 1 {
		t.Fatalf("summary = (%d, %d), want (3, 1)", v, r)
	}
}

func TestCounterRejectsBadPath(t *testing.T) {
	handler := testApp(t)
	cases := []struct {
		name, method, target, body string
	}{
		{"get missing path", http.MethodGet, "/views", ""},
		{"get external", http.MethodGet, "/views?path=https://example.com/posts/a/", ""},
		{"get non-post", http.MethodGet, "/reactions?path=/about/", ""},
		{"post non-post", http.MethodPost, "/views", `{"path":"/about/"}`},
		{"post traversal", http.MethodPost, "/reactions", `{"path":"/posts/../about/"}`},
		{"post unknown field", http.MethodPost, "/views", `{"path":"/posts/a/","x":1}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var bodyReader *strings.Reader
			if tc.body != "" {
				bodyReader = strings.NewReader(tc.body)
			} else {
				bodyReader = strings.NewReader("")
			}
			request := httptest.NewRequest(tc.method, tc.target, bodyReader)
			if tc.method == http.MethodPost {
				request.Header.Set("Content-Type", "application/json")
			}
			response := httptest.NewRecorder()
			handler.ServeHTTP(response, request)
			if response.Code != http.StatusBadRequest {
				t.Fatalf("status = %d, want 400, body = %s", response.Code, response.Body.String())
			}
		})
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
