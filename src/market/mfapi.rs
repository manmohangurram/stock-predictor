//! AMFI's NAV history through the mfapi.in mirror: one scheme code, its whole NAV series.
//!
//! Yahoo's Indian fund coverage is patchy and opaque, but AMFI scheme codes are stable and
//! complete. This is the source for "what has this fund actually returned", which is what a
//! six-month-plus commitment has to be judged on.

use anyhow::{Context, Result};
use serde::Deserialize;

const MFAPI: &str = "https://api.mfapi.in/mf";

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    data: Vec<NavPoint>,
    #[serde(default)]
    meta: Meta,
}

#[derive(Debug, Deserialize, Default)]
struct Meta {
    #[serde(default, rename = "scheme_name")]
    scheme_name: String,
    /// Free from AMFI: "PPFAS Mutual Fund".
    #[serde(default)]
    fund_house: String,
    /// Free from AMFI: "Equity Scheme - Flexi Cap Fund". Enough to spot two funds that are the
    /// same bet.
    #[serde(default)]
    scheme_category: String,
}

#[derive(Debug, Deserialize)]
struct NavPoint {
    date: String,
    #[serde(deserialize_with = "as_f64")]
    nav: f64,
}

fn as_f64<'de, D>(d: D) -> std::result::Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("not a number")),
        serde_json::Value::String(s) => s.trim().parse::<f64>().map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom("nav is not a number")),
    }
}

/// One fund's history, oldest first, with its name and the trailing returns.
#[derive(Debug, Clone, PartialEq)]
pub struct FundHistory {
    pub code: String,
    pub name: String,
    pub fund_house: String,
    pub category: String,
    pub navs: Vec<(String, f64)>,
}

impl FundHistory {
    /// Percentage return over the trailing `years`, from the oldest point within that window.
    /// `None` when the series is too short to say.
    pub fn trailing_return(&self, years: f64) -> Option<f64> {
        let (last_date, last_nav) = self.navs.last()?;
        let cutoff = months_ago(last_date, (years * 12.0) as i64);
        let start = self
            .navs
            .iter()
            .rev()
            .find(|(d, _)| d.as_str() <= cutoff.as_str())?;
        if start.1 <= 0.0 {
            return None;
        }
        if start.0.as_str() > cutoff.as_str() {
            return None;
        }
        Some((last_nav - start.1) / start.1 * 100.0)
    }
}

/// `DD-MM-YYYY` → `YYYY-MM-DD`, so comparisons and arithmetic work on a sortable string.
fn sortable_date(date: &str) -> String {
    if let [d, m, y] = date.split('-').collect::<Vec<_>>().as_slice() {
        if y.len() == 4 {
            return format!("{y}-{m}-{d}");
        }
    }
    date.to_string()
}

/// `YYYY-MM-DD` shifted back `months` months. Approximation is fine: the exact anniversary day is
/// not what decides a ranking.
fn months_ago(date: &str, months: i64) -> String {
    let (y, m, d) = match date.split('-').collect::<Vec<_>>().as_slice() {
        [y, m, d] => (
            y.parse::<i64>().unwrap_or(2026),
            m.parse::<i64>().unwrap_or(1),
            d.parse::<i64>().unwrap_or(1),
        ),
        _ => (2026, 1, 1),
    };
    let total = y * 12 + (m - 1) - months;
    let ny = total.div_euclid(12);
    let nm = total.rem_euclid(12) + 1;
    format!("{ny:04}-{nm:02}-{d:02}")
}

pub async fn fetch(client: &reqwest::Client, code: &str) -> Result<FundHistory> {
    let body = client
        .get(format!("{MFAPI}/{code}"))
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 stock-predictor")
        .send()
        .await
        .context("mfapi: request")?
        .error_for_status()
        .context("mfapi: status")?
        .text()
        .await
        .context("mfapi: body")?;
    parse(code, &body)
}

pub fn parse(code: &str, body: &str) -> Result<FundHistory> {
    let env: Envelope = serde_json::from_str(body).context("mfapi: payload shape")?;
    let mut navs: Vec<(String, f64)> = env
        .data
        .into_iter()
        .filter_map(|p| {
            let nav = p.nav;
            (nav.is_finite() && nav > 0.0).then(|| (sortable_date(&p.date), nav))
        })
        .collect();
    navs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(FundHistory {
        code: code.to_string(),
        name: env.meta.scheme_name,
        fund_house: env.meta.fund_house,
        category: env.meta.scheme_category,
        navs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{
      "status":"SUCCESS",
      "meta":{"scheme_name":"Axis ELSS - Direct - Growth",
               "fund_house":"Axis Mutual Fund",
               "scheme_category":"Equity Scheme - ELSS"},
      "data":[
        {"date":"01-01-2023","nav":"100.00"},
        {"date":"01-01-2024","nav":"112.00"},
        {"date":"01-01-2025","nav":"118.00"},
        {"date":"01-01-2026","nav":"130.00"}
      ]
    }"#;

    #[test]
    fn trailing_return_reads_across_the_year_boundary() {
        let h = parse("120503", BODY).unwrap();
        assert_eq!(h.name, "Axis ELSS - Direct - Growth");
        // 100 -> 130 over three years is +30%.
        let r = h.trailing_return(3.0).unwrap();
        assert!((r - 30.0).abs() < 1.0, "{r}");
    }

    #[test]
    fn a_series_too_short_is_none_not_a_guess() {
        let h = parse(
            "120503",
            r#"{"meta":{},"data":[{"date":"01-01-2026","nav":"100.0"}]}"#,
        )
        .unwrap();
        assert_eq!(h.trailing_return(1.0), None);
    }

    #[test]
    fn nav_is_sorted_oldest_first() {
        let h = parse("120503", BODY).unwrap();
        assert_eq!(h.navs.first().map(|(d, _)| d.as_str()), Some("2023-01-01"));
        assert_eq!(h.navs.last().map(|(d, _)| d.as_str()), Some("2026-01-01"));
    }
}
