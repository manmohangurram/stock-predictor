package ids

import (
	"testing"
	"time"
)

func TestAnIDIsACanonicalLowercaseUUID(t *testing.T) {
	id := New()
	if !Valid(id) {
		t.Fatalf("%q is not a canonical uuid", id)
	}
	if id[14] != '7' {
		t.Fatalf("version nibble is %q, want 7", id[14])
	}
	switch id[19] {
	case '8', '9', 'a', 'b':
	default:
		t.Fatalf("variant nibble is %q", id[19])
	}
	if Valid("00000000-0000-0000-0000-00000000000") || Valid("ABCDEF00-0000-7000-8000-000000000000") {
		t.Fatal("a malformed or upper-case id passed")
	}
}

func TestTheTimestampLeadsTheIDSoLaterSortsLater(t *testing.T) {
	before := New()
	time.Sleep(3 * time.Millisecond)
	after := New()
	if before >= after {
		t.Fatalf("%s did not sort after %s", after, before)
	}
}

func TestNoDuplicatesInABurst(t *testing.T) {
	seen := map[string]bool{}
	for i := 0; i < 10000; i++ {
		id := New()
		if seen[id] {
			t.Fatalf("duplicate %s at %d", id, i)
		}
		seen[id] = true
	}
}
