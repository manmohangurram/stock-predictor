// Package tstamp renders timestamps the way the Rust build wrote them: RFC 3339 with a
// sub-second component, UTC. Rows are compared as strings, so the format is part of the
// data.
package tstamp

import "time"

const layout = "2006-01-02T15:04:05.000000000+00:00"

func Now() string {
	return time.Now().UTC().Format(layout)
}

func From(t time.Time) string {
	return t.UTC().Format(layout)
}
