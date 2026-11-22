package trend

import (
	"strings"
	"testing"
)

func TestALeftOutNameSaysWhichFloorsItMissed(t *testing.T) {
	ltp, turnover, change := 4.2, 21_000_000.0, 2.0
	c := candidate{symbol: "OAL.NS", ltp: &ltp, turnover: &turnover, changePct: &change}

	got := whyNotOffered(c, 5, 10_000_000_000, 19.5)
	if !strings.Contains(got, "price 4.20 is under the 5.00 floor") {
		t.Fatalf("the price floor is missing: %q", got)
	}
	if !strings.Contains(got, "turnover 2.1 crore is under the 1000.0 crore floor") {
		t.Fatalf("the turnover floor is missing: %q", got)
	}
	if strings.Contains(got, "band") {
		t.Fatalf("a 2%% move is not at the band: %q", got)
	}
}

func TestANameThatClearsEveryFloorHasNoReason(t *testing.T) {
	ltp, turnover, change := 1619.0, 12_897_300_000.0, 14.95
	c := candidate{symbol: "X.NS", ltp: &ltp, turnover: &turnover, changePct: &change}

	if got := whyNotOffered(c, 10, 10_000_000, 19.5); got != "" {
		t.Fatalf("a name that clears every floor was refused: %q", got)
	}
	change = 19.5
	if got := whyNotOffered(c, 10, 10_000_000, 19.5); !strings.Contains(got, "band") {
		t.Fatalf("a move at the band must say so: %q", got)
	}
}
