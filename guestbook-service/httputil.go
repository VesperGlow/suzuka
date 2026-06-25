package main

import (
	"encoding/json"
	"errors"
	"io"
	"log"
	"net"
	"net/http"
	"net/url"
	"path"
	"strings"
)

// 请求体上限，留言与计数写接口共用。
const maxRequestBytes = 16 << 10

// clientIP 取请求来源 IP，优先信任反向代理写入的 X-Forwarded-For。
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

// ensureJSONEnd 确认请求体里只有一个 JSON 对象，没有尾随内容。
func ensureJSONEnd(decoder *json.Decoder) error {
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return errors.New("request body must contain one JSON object")
	}
	return nil
}

// validPostURL 校验传入路径是否为站内 /posts/ 的相对路径，
// 既用于留言的来源链接，也用于阅读数 / 喜欢的目标文章。
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
