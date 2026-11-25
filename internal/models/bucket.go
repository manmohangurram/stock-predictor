package models

// The series kept in full, and the fold that carries everything older.
//
// 200-70 = 10 x 13 and 252-70 = 14 x 13: with these numbers both long windows end exactly on a
// bucket boundary, so a folded series gives the same 200-day mean and the same 52-week range as the
// dense one, instead of a mean that includes part of a bucket it cannot see inside.
const (
	DailyFloor     = 70
	BucketSessions = 13
	BucketKeep     = 20
)

// A run of consecutive sessions folded into one row: the mean close, the extremes over the run, and
// how many sessions went into it. The count is what makes a weighted mean exact rather than an
// average of averages.
type Bucket struct {
	EndsOn   string
	Sessions int
	Mean     float64
	High     float64
	Low      float64
}
