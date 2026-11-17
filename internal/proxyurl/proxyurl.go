// Package proxyurl holds the proxy-URL helpers shared by the settings layer and the HTTP
// client builder.
package proxyurl

import (
	"fmt"
	"net/url"
	"strings"
)

// A proxy must be an absolute http(s) URL with a host. A lenient value here would fail
// much later, at request time.
func ValidateProxy(raw string) error {
	if !strings.HasPrefix(raw, "http://") && !strings.HasPrefix(raw, "https://") {
		return fmt.Errorf("proxy must be a http:// or https:// URL")
	}
	u, err := url.Parse(raw)
	if err != nil {
		return fmt.Errorf("proxy URL %q: %w", raw, err)
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return fmt.Errorf("proxy must be a http:// or https:// URL")
	}
	if u.Hostname() == "" {
		return fmt.Errorf("proxy URL has no host")
	}
	return nil
}

// The address the API is willing to show: host and port, never the userinfo.
func ProxyHost(raw string) string {
	u, err := url.Parse(raw)
	if err != nil {
		return ""
	}
	host := u.Hostname()
	if host == "" {
		return ""
	}
	if u.Port() != "" {
		return host + ":" + u.Port()
	}
	return host
}
