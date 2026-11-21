package market

import (
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

// firstRefused refuses the first edge it is asked and answers the second — whichever edge the
// random pick makes "first".
type firstRefused struct {
	calls int
	hosts []string
}

func (f *firstRefused) RoundTrip(r *http.Request) (*http.Response, error) {
	f.calls++
	f.hosts = append(f.hosts, r.URL.Scheme+"://"+r.URL.Host)
	if f.calls == 1 {
		return &http.Response{
			StatusCode: http.StatusTooManyRequests, Header: http.Header{},
			Body: io.NopCloser(strings.NewReader("")), Request: r,
		}, nil
	}
	return &http.Response{
		StatusCode: http.StatusOK, Header: http.Header{},
		Body: io.NopCloser(strings.NewReader(chartBody)), Request: r,
	}, nil
}

func TestARefusedEdgeHandsTheAskingToTheOther(t *testing.T) {
	resetYahooLimiter(t)
	fake := &firstRefused{}
	bars, err := FetchBars(&http.Client{Transport: fake}, "INFY.NS")
	if err != nil {
		t.Fatalf("the second edge was never asked: %v", err)
	}
	if len(bars) != 2 {
		t.Fatalf("%d bars", len(bars))
	}
	if fake.calls != 2 || fake.hosts[0] == fake.hosts[1] {
		t.Fatalf("hosts %v", fake.hosts)
	}
}

func TestARefusalCoolsOneEdgeNotThePair(t *testing.T) {
	resetYahooLimiter(t)
	// Long enough that the assertion does not race the cooldown.
	throttleFirst = 2 * time.Second
	fake := &firstRefused{}
	if _, err := FetchBars(&http.Client{Transport: fake}, "INFY.NS"); err != nil {
		t.Fatal(err)
	}
	if !hostBlocked(fake.hosts[0]) {
		t.Fatal("the refused edge is still being asked")
	}
	if hostBlocked(fake.hosts[1]) {
		t.Fatal("the edge that answered is cooling down too")
	}
	if free := throttleFreeIn(); free != 0 {
		t.Fatalf("no edge is free for %v", free)
	}
}

type alwaysRefused struct{ calls int }

func (f *alwaysRefused) RoundTrip(r *http.Request) (*http.Response, error) {
	f.calls++
	return &http.Response{
		StatusCode: http.StatusTooManyRequests, Header: http.Header{},
		Body: io.NopCloser(strings.NewReader("")), Request: r,
	}, nil
}

func TestBothEdgesRefusingCostsOneAttemptEach(t *testing.T) {
	resetYahooLimiter(t)
	fake := &alwaysRefused{}
	if _, err := FetchBars(&http.Client{Transport: fake}, "INFY.NS"); err == nil {
		t.Fatal("a chart no edge would answer was reported as a chart")
	}
	if fake.calls != 2 {
		t.Fatalf("%d attempts on a refusal", fake.calls)
	}
	throttleMu.Lock()
	strikes := throttleStrikes
	throttleMu.Unlock()
	if strikes != 2 {
		t.Fatalf("strikes = %d, want one per edge", strikes)
	}
}
