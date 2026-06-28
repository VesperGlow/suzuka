package main

import (
	"sync"
	"time"
)

const maxRateLimitKeys = 10_000

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
	maxKeys   int
}

func newRateLimiter(limit int, window time.Duration, now func() time.Time) *rateLimiter {
	return &rateLimiter{
		limit:     limit,
		window:    window,
		now:       now,
		hits:      make(map[string][]time.Time),
		lastSweep: now(),
		maxKeys:   maxRateLimitKeys,
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

	// 伪造大量来源地址时，不允许 map 无界增长。容量耗尽后的新地址共享一个保守桶，
	// 老地址仍按各自窗口正常计算。
	if _, exists := rl.hits[key]; !exists && len(rl.hits) >= rl.maxKeys {
		key = "__overflow__"
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
