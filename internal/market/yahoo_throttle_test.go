package market

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"
	"time"
)

// A spark endpoint that answers every chunk but one. The refused chunk is refused on every
// attempt, the way a throttle behaves.
type sparkFake struct {
	mu         sync.Mutex
	calls      int
	refused    map[string]bool
	retryAfter string
}

func (f *sparkFake) RoundTrip(r *http.Request) (*http.Response, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls++
	syms := strings.Split(r.URL.Query().Get("symbols"), ",")
	for _, s := range syms {
		if f.refused[s] {
			header := http.Header{}
			if f.retryAfter != "" {
				header.Set("Retry-After", f.retryAfter)
			}
			return &http.Response{
				StatusCode: http.StatusTooManyRequests, Header: header,
				Body: io.NopCloser(strings.NewReader("")), Request: r,
			}, nil
		}
	}
	payload := map[string]map[string][]float64{}
	for _, s := range syms {
		payload[s] = map[string][]float64{"close": {100, 101, 102, 103, 104, 105, 106}}
	}
	body, _ := json.Marshal(payload)
	return &http.Response{
		StatusCode: http.StatusOK, Header: http.Header{},
		Body: io.NopCloser(bytes.NewReader(body)), Request: r,
	}, nil
}

func resetYahooLimiter(t *testing.T) {
	t.Helper()
	savedGap, savedFirst, savedWait := yahooGap, throttleFirst, sparkWaitMax
	throttleMu.Lock()
	throttleUntil, throttleAt, throttleStrikes = map[string]time.Time{}, map[string]time.Time{}, 0
	throttleMu.Unlock()
	yahooNext.Store(0)
	yahooGap, throttleFirst, sparkWaitMax = 0, 5*time.Millisecond, 500*time.Millisecond
	t.Cleanup(func() {
		yahooGap, throttleFirst, sparkWaitMax = savedGap, savedFirst, savedWait
		throttleMu.Lock()
		throttleUntil, throttleAt, throttleStrikes = map[string]time.Time{}, map[string]time.Time{}, 0
		throttleMu.Unlock()
		yahooNext.Store(0)
	})
}

func symbolChunks() (symbols []string, refused []string) {
	for i := 0; i < 20; i++ {
		symbols = append(symbols, "OK"+strings.Repeat("A", i%3)+string(rune('A'+i)))
	}
	for i := 0; i < 20; i++ {
		symbols = append(symbols, "FAIL"+string(rune('A'+i)))
	}
	symbols = append(symbols, "TAIL1")
	return symbols, []string{"FAILA"}
}

func TestAThrottledChunkCostsTwentyNamesNotTheDay(t *testing.T) {
	resetYahooLimiter(t)
	symbols, refused := symbolChunks()
	fake := &sparkFake{refused: map[string]bool{}}
	for _, s := range refused {
		fake.refused[s] = true
	}

	closes, err := FetchClosesBatch(&http.Client{Transport: fake}, symbols)
	if err != nil {
		t.Fatalf("a refused chunk must not fail the sweep: %v", err)
	}
	if _, ok := closes["FAILA"]; ok {
		t.Fatal("the refused chunk's names came back anyway")
	}
	if _, ok := closes["TAIL1"]; !ok {
		t.Fatal("the chunk after the refused one was lost")
	}
	if len(closes) < 20 {
		t.Fatalf("only %d names survived", len(closes))
	}
}

func TestASweepThatIsRefusedEverywhereIsAnError(t *testing.T) {
	resetYahooLimiter(t)
	fake := &sparkFake{refused: map[string]bool{"FAILA": true, "FAILB": true}}
	if _, err := FetchClosesBatch(&http.Client{Transport: fake}, []string{"FAILA", "FAILB"}); err == nil {
		t.Fatal("a sweep of nothing must report why")
	}
}

func TestEveryThrottleWaitsLongerThanTheOneBefore(t *testing.T) {
	resetYahooLimiter(t)
	waits := []time.Duration{}
	for i := 0; i < 3; i++ {
		noteThrottle(yahooHosts[0], &http.Response{StatusCode: http.StatusTooManyRequests, Header: http.Header{}})
		throttleMu.Lock()
		waits = append(waits, time.Until(throttleUntil[yahooHosts[0]]))
		throttleMu.Unlock()
	}
	if waits[1] <= waits[0] || waits[2] <= waits[1] {
		t.Fatalf("the cooldown did not escalate: %v", waits)
	}
}

func TestRetryAfterRaisesTheCooldown(t *testing.T) {
	resetYahooLimiter(t)
	header := http.Header{}
	header.Set("Retry-After", "120")
	noteThrottle(yahooHosts[1], &http.Response{StatusCode: http.StatusTooManyRequests, Header: header})
	throttleMu.Lock()
	left := time.Until(throttleUntil[yahooHosts[1]])
	throttleMu.Unlock()
	if left < 100*time.Second {
		t.Fatalf("Retry-After was ignored: %v", left)
	}
}
