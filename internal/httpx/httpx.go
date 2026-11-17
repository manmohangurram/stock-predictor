// Package httpx is the one place an outbound HTTP client is built, so the proxy decision
// is made once.
//
// A transport built here never consults HTTP_PROXY/HTTPS_PROXY/ALL_PROXY: the proxy comes
// only from the database, so the environment can never outrank the settings page.
package httpx

import (
	"fmt"
	"log/slog"
	"net/http"
	"net/url"
	"strings"
	"time"

	"stock-predictor/internal/proxyurl"
	"stock-predictor/internal/rules"
)

// logTransport records every outbound request at debug, and every failure or 4xx/5xx at warn.
// Host only: query strings and paths can carry a key (Gemini) or a topic (ntfy).
type logTransport struct {
	base http.RoundTripper
}

func (t logTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	started := time.Now()
	resp, err := t.base.RoundTrip(req)
	ms := time.Since(started).Milliseconds()
	if err != nil {
		slog.Warn("outbound request failed", "method", req.Method, "host", req.URL.Host,
			"ms", ms, "error", err)
		return nil, err
	}
	attrs := []any{"method", req.Method, "host", req.URL.Host, "status", resp.StatusCode, "ms", ms}
	if resp.StatusCode >= 400 {
		slog.Warn("outbound request", attrs...)
	} else {
		slog.Debug("outbound request", attrs...)
	}
	return resp, nil
}

func ValidateProxy(raw string) error {
	return proxyurl.ValidateProxy(raw)
}

// Client builds a client with the runtime's proxy. The environment's proxies are never read.
func Client(rt *rules.Runtime, timeoutSecs int) (*http.Client, error) {
	return ClientWith(rt, timeoutSecs, "")
}

// Same, with a browser-ish user agent for the endpoints that expect one.
func ClientWith(rt *rules.Runtime, timeoutSecs int, userAgent string) (*http.Client, error) {
	transport := &http.Transport{Proxy: nil} // no env proxies, ever
	if rt.Proxy != nil && *rt.Proxy != "" {
		raw := *rt.Proxy
		if err := proxyurl.ValidateProxy(raw); err != nil {
			return nil, err
		}
		u, err := url.Parse(raw)
		if err != nil {
			return nil, fmt.Errorf("proxy URL %q is not usable: %w", raw, err)
		}
		if rt.ProxyUser != nil && *rt.ProxyUser != "" {
			// Basic auth from the two fields, so the password is a secret like any other.
			pass := ""
			if rt.ProxyPass != nil {
				pass = *rt.ProxyPass
			}
			u.User = url.UserPassword(*rt.ProxyUser, pass)
		}
		var bypass []string
		if rt.ProxyBypass != nil {
			for _, host := range strings.Split(*rt.ProxyBypass, ",") {
				host = strings.TrimSpace(host)
				if host != "" {
					bypass = append(bypass, host)
				}
			}
		}
		transport.Proxy = proxyFunc(u, bypass)
	}
	return &http.Client{
		Transport: logTransport{base: transport},
		Timeout:   time.Duration(timeoutSecs) * time.Second,
	}, nil
}

func proxyFunc(proxyURL *url.URL, bypass []string) func(*http.Request) (*url.URL, error) {
	return func(req *http.Request) (*url.URL, error) {
		for _, host := range bypass {
			if host == req.URL.Hostname() || host == req.URL.Host {
				return nil, nil
			}
		}
		return proxyURL, nil
	}
}
