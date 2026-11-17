package httpx

import (
	"testing"

	"stock-predictor/internal/proxyurl"
)

func TestACredentialledProxyURLShowsOnlyTheHost(t *testing.T) {
	if got := proxyurl.ProxyHost("http://user:secret@10.0.0.2:3128"); got != "10.0.0.2:3128" {
		t.Fatalf("got %q", got)
	}
	if got := proxyurl.ProxyHost("https://proxy.example.in"); got != "proxy.example.in" {
		t.Fatalf("got %q", got)
	}
	if got := proxyurl.ProxyHost("not a url"); got != "" {
		t.Fatalf("got %q", got)
	}
}

func TestABareHostAndPortIsNotAProxy(t *testing.T) {
	if err := proxyurl.ValidateProxy("10.0.0.2:3128"); err == nil {
		t.Fatal("bare host accepted")
	}
	if err := proxyurl.ValidateProxy("socks5://10.0.0.2:1080"); err == nil {
		t.Fatal("socks5 accepted")
	}
	if err := proxyurl.ValidateProxy("http://10.0.0.2:3128"); err != nil {
		t.Fatalf("http rejected: %v", err)
	}
	if err := proxyurl.ValidateProxy("https://user:pass@proxy.example.in:8443"); err != nil {
		t.Fatalf("credentialled https rejected: %v", err)
	}
}
