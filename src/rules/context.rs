//! What a short-term decision needs that a long-term one does not.
//!
//! A trade that must finish inside a week lives or dies on the next few sessions: where today sits
//! against yesterday's range, whether the move has volume behind it, how much of the day is left,
//! and whether the market itself is risk-on. This module computes those from the bars we already
//! store, so the prompt carries about twenty numbers instead of a thousand bars.

use chrono::{DateTime, Datelike, Timelike, Utc};

use crate::models::Bar;

const IST_OFFSET_MINUTES: i64 = 330;
/// NSE equity session, in IST minutes from midnight.
const OPEN_MIN: u32 = 9 * 60 + 15;
const CLOSE_MIN: u32 = 15 * 60 + 30;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShortContext {
    /// Minutes since the open (0 before it) and until the close (0 after it).
    pub minutes_from_open: u32,
    pub minutes_to_close: u32,
    /// Today's move so far, and the gap from yesterday's close, in percent.
    pub day_change_pct: Option<f64>,
    pub gap_pct: Option<f64>,
    /// Yesterday's high/low and where the last price sits between them, in percent.
    pub prev_high_pct: Option<f64>,
    pub prev_low_pct: Option<f64>,
    /// Relative volume: today's pace against the 20-day average, 1.0 = average.
    pub rvol: Option<f64>,
    pub atr14: Option<f64>,
    pub atr_pct: Option<f64>,
    /// Close-to-close returns over 3, 5 and 10 sessions.
    pub ret_3d: Option<f64>,
    pub ret_5d: Option<f64>,
    pub ret_10d: Option<f64>,
    /// Distance from the 52-week extremes, in percent.
    pub from_52w_high_pct: Option<f64>,
    pub from_52w_low_pct: Option<f64>,
}

impl ShortContext {
    /// A compact block a model can read in a few seconds. Every line is a number it would otherwise
    /// have to compute, and an uncomputable line is omitted rather than filled with a guess.
    pub fn render(&self) -> String {
        let pct = |v: Option<f64>| v.map(|x| format!("{x:+.2}%")).unwrap_or_else(|| "—".into());
        let num = |v: Option<f64>| v.map(|x| format!("{x:.2}")).unwrap_or_else(|| "—".into());
        let mut out = String::new();
        out.push_str(&format!(
            "Session: {} min since the open, {} min to the close\n",
            self.minutes_from_open, self.minutes_to_close
        ));
        out.push_str(&format!(
            "Today: change {}, gap {}, RVOL {} (20-day average = 1.0)\n",
            pct(self.day_change_pct),
            pct(self.gap_pct),
            num(self.rvol)
        ));
        out.push_str(&format!(
            "Yesterday's range: last price is {} from yesterday's high, {} from its low\n",
            pct(self.prev_high_pct),
            pct(self.prev_low_pct)
        ));
        out.push_str(&format!(
            "Volatility: ATR14 {}, that is {} of price\n",
            num(self.atr14),
            pct(self.atr_pct)
        ));
        out.push_str(&format!(
            "Momentum: 3-day {}, 5-day {}, 10-day {}\n",
            pct(self.ret_3d),
            pct(self.ret_5d),
            pct(self.ret_10d)
        ));
        out.push_str(&format!(
            "Position in the year: {} below the 52-week high, {} above the low\n",
            pct(self.from_52w_high_pct.map(|v| -v)),
            pct(self.from_52w_low_pct)
        ));
        out
    }
}

/// Today's session position, in IST minutes.
fn session(now: DateTime<Utc>) -> (u32, u32) {
    let ist = now + chrono::Duration::minutes(IST_OFFSET_MINUTES);
    let mins = ist.hour() * 60 + ist.minute();
    let from_open = mins.saturating_sub(OPEN_MIN);
    let to_close = CLOSE_MIN.saturating_sub(mins);
    (from_open.min(CLOSE_MIN - OPEN_MIN), to_close)
}

/// `bars` are daily closes, oldest first; `intraday` is today's tape (any interval), oldest first.
pub fn compute(bars: &[Bar], intraday: &[Bar], now: DateTime<Utc>) -> ShortContext {
    let mut c = ShortContext::default();
    let (from_open, to_close) = session(now);
    c.minutes_from_open = from_open;
    c.minutes_to_close = to_close;

    let last = match bars.last() {
        Some(b) => b,
        None => return c,
    };
    let prev = bars.len().checked_sub(2).map(|i| &bars[i]);

    if let Some(p) = prev {
        if p.close > 0.0 {
            c.day_change_pct = Some((last.close - p.close) / p.close * 100.0);
            c.prev_high_pct = Some((last.close - p.high) / p.close * 100.0);
            c.prev_low_pct = Some((last.close - p.low) / p.close * 100.0);
        }
        if let Some(today) = intraday.first() {
            if p.close > 0.0 && today.open > 0.0 {
                c.gap_pct = Some((today.open - p.close) / p.close * 100.0);
            }
        }
    }

    // ATR14 from the daily series: simple true range average, which is what the rules use.
    if bars.len() >= 15 {
        let tail = &bars[bars.len() - 14..];
        let mut sum = 0.0;
        for i in 0..tail.len() {
            let prev_close = if i == 0 {
                tail[0].open
            } else {
                tail[i - 1].close
            };
            let tr = (tail[i].high - tail[i].low)
                .max((tail[i].high - prev_close).abs())
                .max((tail[i].low - prev_close).abs());
            sum += tr;
        }
        let atr = sum / tail.len() as f64;
        c.atr14 = Some(atr);
        if last.close > 0.0 {
            c.atr_pct = Some(atr / last.close * 100.0);
        }
    }

    // Relative volume: today's traded quantity against the 20-day average.
    if bars.len() >= 21 {
        let avg: f64 = bars[bars.len() - 21..bars.len() - 1]
            .iter()
            .map(|b| b.volume as f64)
            .sum::<f64>()
            / 20.0;
        if avg > 0.0 {
            let today = intraday.iter().map(|b| b.volume as f64).sum::<f64>();
            if today > 0.0 {
                c.rvol = Some(today / avg);
            }
        }
    }

    let ret = |n: usize| -> Option<f64> {
        if bars.len() <= n {
            return None;
        }
        let a = bars[bars.len() - 1 - n].close;
        (a > 0.0).then(|| (last.close - a) / a * 100.0)
    };
    c.ret_3d = ret(3);
    c.ret_5d = ret(5);
    c.ret_10d = ret(10);

    let hi = bars.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let lo = bars.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    if last.close > 0.0 && hi > 0.0 && lo > 0.0 {
        c.from_52w_high_pct = Some((hi - last.close) / hi * 100.0);
        c.from_52w_low_pct = Some((last.close - lo) / lo * 100.0);
    }
    c
}

/// `YYYY-MM-DD` of the session the tape belongs to, for the prompt's "as of".
pub fn session_date(now: DateTime<Utc>) -> String {
    let ist = now + chrono::Duration::minutes(IST_OFFSET_MINUTES);
    format!("{:04}-{:02}-{:02}", ist.year(), ist.month(), ist.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(close: f64, high: f64, low: f64, volume: i64) -> Bar {
        Bar {
            ts: "2026-11-06T00:00:00Z".into(),
            open: close,
            high,
            low,
            close,
            volume,
        }
    }

    fn series(n: usize, step: f64) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let c = 100.0 + i as f64 * step;
                bar(c, c + 1.0, c - 1.0, 1000)
            })
            .collect()
    }

    #[test]
    fn momentum_and_position_in_the_year_are_computed() {
        let bars = series(30, 1.0);
        let today = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 9, 6, 0, 0).unwrap();
        let c = compute(&bars, &[], today);
        assert!(c.ret_5d.unwrap() > 4.0, "{:?}", c.ret_5d);
        assert!(c.from_52w_high_pct.unwrap() < 1.0);
        assert!(c.atr_pct.unwrap() > 0.0);
    }

    #[test]
    fn a_flat_series_has_no_relvolume_without_tape() {
        let bars = series(30, 0.0);
        let today = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 9, 6, 0, 0).unwrap();
        let c = compute(&bars, &[], today);
        assert_eq!(c.rvol, None, "no intraday volume means no RVOL, not a guess");
    }

    #[test]
    fn the_session_clock_is_ist() {
        // 10:00 UTC is 15:30 IST — the close.
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 9, 10, 0, 0).unwrap();
        let (from_open, to_close) = session(now);
        assert_eq!(from_open, 6 * 60 + 15);
        assert_eq!(to_close, 0);
    }

    #[test]
    fn the_card_omits_what_it_cannot_compute() {
        let c = ShortContext::default();
        let text = c.render();
        assert!(text.contains("RVOL —"), "{text}");
        assert!(!text.contains("NaN"));
    }
}
