package models

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
