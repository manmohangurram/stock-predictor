package secrets

import (
	"path/filepath"
	"testing"
)

func TestASecretRoundTripsAndNeverAppearsInTheStoredForm(t *testing.T) {
	key := make([]byte, 32)
	for i := range key {
		key[i] = 7
	}
	sealed, err := EncryptWith(key, "sk-live-hunter2")
	if err != nil {
		t.Fatal(err)
	}
	if len(sealed) < len(Prefix) || sealed[:len(Prefix)] != Prefix {
		t.Fatalf("missing prefix: %q", sealed)
	}
	if got, ok := DecryptWith(key, sealed); !ok || got != "sk-live-hunter2" {
		t.Fatalf("round trip failed: %q %v", got, ok)
	}
}

func TestTheWrongKeyYieldsNothingRatherThanGarbage(t *testing.T) {
	k1 := make([]byte, 32)
	k2 := make([]byte, 32)
	k1[0], k2[0] = 1, 2
	sealed, err := EncryptWith(k1, "secret")
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := DecryptWith(k2, sealed); ok {
		t.Fatal("wrong key decrypted")
	}
}

func TestTheSamePlaintextEncryptsDifferentlyEachTime(t *testing.T) {
	key := make([]byte, 32)
	key[0] = 3
	a, _ := EncryptWith(key, "x")
	b, _ := EncryptWith(key, "x")
	if a == b {
		t.Fatal("nonce reuse")
	}
}

func TestTheKeyFileSitsBesideTheDatabase(t *testing.T) {
	if got, want := KeyPath("data/stock-predictor.db"), filepath.Join("data", "secret.key"); got != want {
		t.Fatalf("got %q want %q", got, want)
	}
}
