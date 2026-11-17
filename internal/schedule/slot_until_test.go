package schedule

import (
	"testing"
	"time"
)

func TestTheNextSlotIsTheBoundaryNotTheInterval(t *testing.T) {
	// 09:00 IST -> 09:15 sharp, fifteen minutes, not "whenever 900s lands".
	if got := UntilNextSlot(ist(2026, 11, 17, 9, 0)); got != 15*time.Minute {
		t.Fatalf("%v", got)
	}
	// Inside the first slot -> 11:15.
	if got := UntilNextSlot(ist(2026, 11, 17, 10, 0)); got != 75*time.Minute {
		t.Fatalf("%v", got)
	}
	if got := UntilNextSlot(ist(2026, 11, 17, 13, 16)); got != 19*time.Hour+59*time.Minute {
		t.Fatalf("%v", got)
	}
}

func TestAFridayEveningReachesMondayOpen(t *testing.T) {
	got := UntilNextSlot(ist(2026, 11, 13, 16, 0))
	want := 65*time.Hour + 15*time.Minute
	if got != want {
		t.Fatalf("%v want %v", got, want)
	}
}

func TestASlotDueNowIsZero(t *testing.T) {
	if got := UntilNextSlot(ist(2026, 11, 17, 9, 15)); got != 0 {
		t.Fatalf("%v", got)
	}
}
