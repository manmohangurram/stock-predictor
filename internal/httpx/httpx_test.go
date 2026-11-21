package httpx

import (
	"io"
	"net/http"
	"strings"
	"testing"

	"stock-predictor/internal/proxyurl"
)

type recordingTransport struct{ agents []string }

func (r *recordingTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	r.agents = append(r.agents, req.Header.Get("User-Agent"))
	return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(""))}, nil
}

func TestTheClientSendsTheAgentItWasGiven(t *testing.T) {
	rec := &recordingTransport{}
	client := &http.Client{Transport: userAgentTransport{base: rec, ua: "Mozilla/5.0 stock-predictor"}}
	if _, err := client.Get("http://example.in/chart"); err != nil {
		t.Fatal(err)
	}
	if rec.agents[0] != "Mozilla/5.0 stock-predictor" {
		t.Fatalf("%q", rec.agents[0])
	}
}

func TestARequestKeepsTheAgentItSetItself(t *testing.T) {
	rec := &recordingTransport{}
	client := &http.Client{Transport: userAgentTransport{base: rec, ua: "Mozilla/5.0 stock-predictor"}}
	req, err := http.NewRequest(http.MethodGet, "http://example.in/chart", nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 custom")
	if _, err := client.Do(req); err != nil {
		t.Fatal(err)
	}
	if rec.agents[0] != "Mozilla/5.0 custom" {
		t.Fatalf("%q", rec.agents[0])
	}
}

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
