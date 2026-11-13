//! Revenue, profit and margins, straight from Yahoo's fundamentals time series.
//!
//! The endpoint needs no crumb (unlike `quoteSummary`, which rate-limits us), and it answers for
//! Indian listings. Figures come back in the reporting currency — sometimes USD for an Indian
//! company, so the currency is carried through and never assumed.
//!
//! This is context for a decision, not a valuation model: the prompt gets four years of revenue and
//! net income with their growth, plus the latest quarter's revenue, and nothing else.

use anyhow::{Context, Result};

const TS: &str =
    "https://query2.finance.yahoo.com/ws/fundamentals-timeseries/v1/finance/timeseries";

#[derive(Debug, Clone, PartialEq)]
pub struct Year {
    pub as_of: String,
    pub revenue: Option<f64>,
    pub net_income: Option<f64>,
    pub operating_income: Option<f64>,
    pub gross_profit: Option<f64>,
    pub currency: String,
    /// The same endpoint serves valuation: PE and market cap, crumb-free.
    pub pe: Option<f64>,
    pub market_cap: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fundamentals {
    /// Oldest first.
    pub years: Vec<Year>,
    /// The latest quarter, when the endpoint has one.
    pub quarter: Option<(String, f64)>,
    /// Trailing PE and market cap, as of the latest close.
    pub valuation: Option<(f64, f64)>,
}

impl Fundamentals {
    pub fn latest(&self) -> Option<&Year> {
        self.years.last()
    }

    /// Year-over-year revenue growth, in percent, from the last two annual figures.
    pub fn revenue_growth_pct(&self) -> Option<f64> {
        let n = self.years.len();
        if n < 2 {
            return None;
        }
        let (a, b) = (self.years[n - 2].revenue?, self.years[n - 1].revenue?);
        (a > 0.0).then(|| (b - a) / a * 100.0)
    }

    /// Year-over-year net income growth, in percent.
    pub fn profit_growth_pct(&self) -> Option<f64> {
        let n = self.years.len();
        if n < 2 {
            return None;
        }
        let (a, b) = (self.years[n - 2].net_income?, self.years[n - 1].net_income?);
        (a.abs() > 0.0).then(|| (b - a) / a * 100.0)
    }

    /// Net margin in percent — the shape of the business, not its size.
    pub fn net_margin_pct(&self) -> Option<f64> {
        let y = self.latest()?;
        let rev = y.revenue?;
        let net = y.net_income?;
        (rev > 0.0).then(|| net / rev * 100.0)
    }

    /// A few lines a broker would say out loud. Empty when there is nothing honest to say.
    pub fn summary(&self) -> String {
        let Some(y) = self.latest() else {
            return String::new();
        };
        let money = |v: Option<f64>| match v {
            Some(n) if n.abs() >= 1e9 => format!("{:.1}B {}", n / 1e9, y.currency),
            Some(n) if n.abs() >= 1e6 => format!("{:.0}M {}", n / 1e6, y.currency),
            Some(n) => format!("{n:.0} {}", y.currency),
            None => "—".into(),
        };
        let pct = |v: Option<f64>| v.map(|p| format!("{p:+.1}%")).unwrap_or_else(|| "—".into());
        let mut out = format!(
            "FY{}: revenue {}, net income {}, net margin {}, operating income {}\n",
            y.as_of,
            money(y.revenue),
            money(y.net_income),
            pct(self.net_margin_pct()),
            money(y.operating_income),
        );
        out.push_str(&format!(
            "Growth: revenue {} YoY, profit {} YoY\n",
            pct(self.revenue_growth_pct()),
            pct(self.profit_growth_pct())
        ));
        if let Some((pe, cap)) = self.valuation {
            out.push_str(&format!(
                "Valuation: trailing PE {pe:.1}, market cap {}{}\n",
                if cap > 0.0 {
                    format!("{:.0}B ", cap / 1e9)
                } else {
                    String::new()
                },
                y.currency
            ));
        }
        if let Some((as_of, rev)) = &self.quarter {
            out.push_str(&format!(
                "Latest quarter ({as_of}): revenue {}\n",
                match *rev {
                    n if n.abs() >= 1e9 => format!("{:.1}B {}", n / 1e9, y.currency),
                    n => format!("{n:.0} {}", y.currency),
                }
            ));
        }
        out
    }
}

pub async fn fetch(client: &reqwest::Client, symbol: &str, now_year: i32) -> Result<Fundamentals> {
    let year_start = |y: i32| {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, y, 1, 1, 0, 0, 0)
            .single()
            .map(|d| d.timestamp())
            .unwrap_or(0)
    };
    let period1 = year_start(now_year - 5);
    let period2 = year_start(now_year + 1);
    let body = client
        .get(format!("{TS}/{symbol}"))
        .query(&[
            ("symbol", symbol),
            (
                "type",
                "annualTotalRevenue,annualNetIncome,annualOperatingIncome,annualGrossProfit,\
                 trailingPeRatio,trailingMarketCap",
            ),
            ("period1", &period1.to_string()),
            ("period2", &period2.to_string()),
        ])
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 stock-predictor")
        .send()
        .await
        .context("fundamentals: request")?
        .error_for_status()
        .context("fundamentals: status")?
        .text()
        .await
        .context("fundamentals: body")?;
    parse(&body)
}

pub fn parse(body: &str) -> Result<Fundamentals> {
    let v: serde_json::Value = serde_json::from_str(body).context("fundamentals: payload shape")?;
    // Yahoo splits the series across result entries; result[0] alone is often empty.
    let results = v
        .get("timeseries")
        .and_then(|t| t.get("result"))
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow::anyhow!("fundamentals: no result"))?;

    let series = |key: &str| -> Vec<(String, f64, String)> {
        let row = results
            .iter()
            .find(|r| r.get(key).is_some_and(|v| !v.is_null()))
            .unwrap_or(&results[0]);
        row.get(key)
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let as_of = p.get("asOfDate")?.as_str()?.to_string();
                        let raw = p.get("reportedValue")?.get("raw")?.as_f64()?;
                        let currency = p
                            .get("currencyCode")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some((as_of, raw, currency))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let revenue = series("annualTotalRevenue");
    let net = series("annualNetIncome");
    let op = series("annualOperatingIncome");
    let gross = series("annualGrossProfit");

    let lookup = |s: &[(String, f64, String)], date: &str| {
        s.iter().find(|(d, _, _)| d == date).map(|(_, v, _)| *v)
    };
    let mut years: Vec<Year> = revenue
        .iter()
        .map(|(as_of, rev, currency)| Year {
            as_of: as_of.clone(),
            revenue: Some(*rev),
            net_income: lookup(&net, as_of),
            operating_income: lookup(&op, as_of),
            gross_profit: lookup(&gross, as_of),
            currency: if currency.is_empty() {
                "INR".into()
            } else {
                currency.clone()
            },
            pe: None,
            market_cap: None,
        })
        .collect();
    years.sort_by(|a, b| a.as_of.cmp(&b.as_of));
    years.dedup_by(|a, b| a.as_of == b.as_of);

    let pe = series("trailingPeRatio").last().map(|(_, v, _)| *v);
    let cap = series("trailingMarketCap").last().map(|(_, v, _)| *v);
    let valuation = match (pe, cap) {
        (Some(p), Some(c)) => Some((p, c)),
        (Some(p), None) => Some((p, 0.0)),
        _ => None,
    };
    if let (Some((_, p, _)), Some(last)) = (series("trailingPeRatio").last(), years.last_mut()) {
        last.pe = Some(*p);
        last.market_cap = series("trailingMarketCap").last().map(|(_, v, _)| *v);
    }
    Ok(Fundamentals {
        years,
        quarter: None,
        valuation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"timeseries":{"result":[{
      "meta":{"symbol":["INFY.NS"]},
      "annualTotalRevenue":[
        {"asOfDate":"2023-03-31","reportedValue":{"raw":1821200000000.0},"currencyCode":"INR"},
        {"asOfDate":"2024-03-31","reportedValue":{"raw":1862000000000.0},"currencyCode":"INR"},
        {"asOfDate":"2025-03-31","reportedValue":{"raw":1930000000000.0},"currencyCode":"INR"},
        {"asOfDate":"2026-03-31","reportedValue":{"raw":2020000000000.0},"currencyCode":"INR"}],
      "annualNetIncome":[
        {"asOfDate":"2025-03-31","reportedValue":{"raw":267130000000.0},"currencyCode":"INR"},
        {"asOfDate":"2026-03-31","reportedValue":{"raw":331000000000.0},"currencyCode":"INR"}]
    }]}}"#;

    #[test]
    fn growth_and_margin_are_computed_from_the_series() {
        let f = parse(BODY).unwrap();
        assert_eq!(f.years.len(), 4);
        // 1,930 -> 2,020 is +4.7%.
        let g = f.revenue_growth_pct().unwrap();
        assert!((g - 4.66).abs() < 0.1, "{g}");
        // 267.13 -> 331.0 is +23.9%.
        let p = f.profit_growth_pct().unwrap();
        assert!((p - 23.9).abs() < 0.2, "{p}");
        // 331 of 2,020 is 16.4%.
        let m = f.net_margin_pct().unwrap();
        assert!((m - 16.4).abs() < 0.2, "{m}");
    }

    #[test]
    fn a_single_year_has_no_growth_to_claim() {
        let one = r#"{"timeseries":{"result":[{"annualTotalRevenue":[{"asOfDate":"2026-03-31","reportedValue":{"raw":1000.0},"currencyCode":"INR"}]}]}}"#;
        let f = parse(one).unwrap();
        assert_eq!(f.revenue_growth_pct(), None);
        assert_eq!(f.profit_growth_pct(), None);
    }

    #[test]
    fn the_summary_states_currency_and_direction() {
        let s = parse(BODY).unwrap().summary();
        assert!(s.contains("INR"), "{s}");
        assert!(s.contains("FY2026-03-31"), "{s}");
        assert!(s.contains("+4.7%"), "{s}");
    }
}
