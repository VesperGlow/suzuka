package main

import (
	"encoding/json"
	"errors"
	"io"
	"log"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"path"
	"strings"
)

// 请求体上限，留言与计数写接口共用。
const maxRequestBytes = 16 << 10

// clientIP 取请求来源 IP。仅当直连方来自回环或私有网络（即常见的本机/容器反代）时，
// 才信任 X-Forwarded-For；公网直连客户端不能靠伪造请求头绕过限流。
// 上游反代仍须覆盖而不是透传客户端自带的 X-Forwarded-For。
func clientIP(r *http.Request) string {
	remote := parseRemoteIP(r.RemoteAddr)
	if remote.IsValid() && (remote.IsLoopback() || remote.IsPrivate()) {
		for _, value := range strings.Split(r.Header.Get("X-Forwarded-For"), ",") {
			if forwarded, err := netip.ParseAddr(strings.TrimSpace(value)); err == nil {
				return forwarded.Unmap().String()
			}
		}
	}
	if remote.IsValid() {
		return remote.Unmap().String()
	}

	if host, _, err := net.SplitHostPort(r.RemoteAddr); err == nil {
		return host
	}
	return r.RemoteAddr
}

func parseRemoteIP(value string) netip.Addr {
	if address, err := netip.ParseAddrPort(value); err == nil {
		return address.Addr()
	}
	address, _ := netip.ParseAddr(strings.Trim(value, "[]"))
	return address
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

// writeInternalError 对外隐藏实现细节，同时把真实错误留在服务日志中便于排查。
func (a *app) writeInternalError(w http.ResponseWriter, r *http.Request, operation string, err error) {
	if a.logger != nil {
		a.logger.Printf("%s %s: %s: %v", r.Method, r.URL.Path, operation, err)
	}
	writeError(w, http.StatusInternalServerError, operation)
}
