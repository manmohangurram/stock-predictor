// Package ids mints the UUIDv7 every row's identity is made of.
package ids

import "github.com/google/uuid"

// New returns a lower-case canonical UUIDv7. Its leading 48 bits are the millisecond it was
// created, so rows read newest-first by id without a second timestamp column.
func New() string {
	id, err := uuid.NewV7()
	if err != nil {
		panic("ids: no entropy: " + err.Error())
	}
	return id.String()
}

// Valid reports whether s is a canonical lower-case UUIDv7.
func Valid(s string) bool {
	id, err := uuid.Parse(s)
	return err == nil && id.Version() == 7 && id.String() == s
}
