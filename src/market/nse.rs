//! NSE's own list of what moved today.
//!
//! One endpoint, no cookies: `live-analysis-variations?index=gainers` returns the day's variation
//! lists grouped by index. `allSec` is every security, already ranked by percentage change — which
//! is exactly the "what is hot in India right now" question, answered by the exchange.

use anyhow::{Context, Result};
use serde::Deserialize;

const GAINERS_URL: &str = "https://www.nseindia.com/api/live-analysis-variations?index=gainers";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) stock-predictor/0.4";

/// One row of the variation list, trimmed to what the app uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Gainer {
    pub symbol: String,
    pub ltp: f64,
    pub previous_close: f64,
    pub per_change: f64,
    pub turnover: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default, rename = "allSec")]
    all_sec: Option<Group>,
}

#[derive(Debug, Deserialize)]
struct Group {
    #[serde(default)]
    data: Vec<Row>,
}

#[derive(Debug, Deserialize)]
struct Row {
    symbol: String,
    #[serde(default, deserialize_with = "number_or_text")]
    ltp: Option<f64>,
    #[serde(default, deserialize_with = "number_or_text")]
    prev_price: Option<f64>,
    #[serde(default, rename = "perChange", deserialize_with = "number_or_text")]
    per_change: Option<f64>,
    #[serde(default, deserialize_with = "number_or_text")]
    turnover: Option<f64>,
}

/// NSE is not consistent about types: the same field is `22.86` on one row and `"22.86 Cr"` on the
/// next, so one formatted value used to reject the entire payload with "invalid type: string".
/// Read a number, a number in text, or a text with an Indian magnitude suffix.
fn number_or_text<'de, D>(d: D) -> std::result::Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let raw = Value::deserialize(d)?;
    Ok(match raw {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => from_text(&s),
        _ => None,
    })
}

/// `"22.86 Cr"` → 22,86,00,000; `"1.5 L"` → 1,50,000; `"14.18%"` → 14.18; `"-"` → None.
fn from_text(raw: &str) -> Option<f64> {
    let text = raw.trim().replace(',', "");
    if text.is_empty() || text == "-" || text.eq_ignore_ascii_case("null") {
        return None;
    }
    let text = text.trim_end_matches('%').trim();
    let lower = text.to_ascii_lowercase();
    let (body, scale) = if let Some(head) = lower.strip_suffix("cr") {
        (head.trim(), 1e7)
    } else if let Some(head) = lower
        .strip_suffix("lakh")
        .or_else(|| lower.strip_suffix("lac"))
        .or_else(|| lower.strip_suffix('l'))
    {
        (head.trim(), 1e5)
    } else if let Some(head) = lower.strip_suffix('k') {
        (head.trim(), 1e3)
    } else {
        (lower.as_str(), 1.0)
    };
    body.trim().parse::<f64>().ok().map(|n| n * scale)
}

pub async fn fetch_gainers(client: &reqwest::Client, top: usize) -> Result<Vec<Gainer>> {
    let body = client
        .get(GAINERS_URL)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("nse: request")?
        .error_for_status()
        .context("nse: status")?
        .text()
        .await
        .context("nse: body")?;
    parse_gainers(&body, top)
}

/// Pure, so the shape can be tested against a fixture.
pub fn parse_gainers(body: &str, top: usize) -> Result<Vec<Gainer>> {
    let env: Envelope = serde_json::from_str(body).context("nse: payload shape")?;
    let rows = env
        .all_sec
        .ok_or_else(|| anyhow::anyhow!("nse: no allSec group in the payload"))?
        .data;
    let mut out: Vec<Gainer> = rows
        .into_iter()
        .filter_map(|r| {
            Some(Gainer {
                symbol: r.symbol.trim().to_string(),
                ltp: r.ltp.filter(|v| v.is_finite() && *v > 0.0)?,
                previous_close: r
                    .prev_price
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .unwrap_or(0.0),
                per_change: r.per_change.filter(|v| v.is_finite())?,
                turnover: r.turnover,
            })
        })
        .filter(|g| !g.symbol.is_empty())
        .collect();
    out.sort_by(|a, b| b.per_change.total_cmp(&a.per_change));
    out.truncate(top);
    Ok(out)
}

impl Gainer {
    /// The symbol as Yahoo knows it. NSE quotes the bare ticker; Yahoo wants the exchange suffix.
    pub fn yahoo_symbol(&self) -> String {
        format!("{}.NS", self.symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = r#"{
      "legends": [["NIFTY","NIFTY 50"]],
      "allSec": {"timestamp": "09-Nov-2026 15:30:00", "data": [
        {"symbol":"ALEMBICLTD","series":"EQ","ltp":113.21,"prev_price":99.15,"perChange":14.18,"turnover":1234.5},
        {"symbol":"TRANSWORLD","series":"EQ","ltp":228.98,"prev_price":197.82,"perChange":15.75,"turnover":88.0},
        {"symbol":"IOLCP","series":"EQ","ltp":210.12,"prev_price":184.28,"perChange":14.02,"turnover":55.2}
      ]}
    }"#;

    #[test]
    fn the_list_is_ranked_by_change_and_trimmed() {
        let g = parse_gainers(PAYLOAD, 2).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].symbol, "TRANSWORLD");
        assert_eq!(g[1].symbol, "ALEMBICLTD");
        assert!((g[0].per_change - 15.75).abs() < 1e-9);
    }

    #[test]
    fn a_symbol_is_translated_for_yahoo() {
        let g = parse_gainers(PAYLOAD, 1).unwrap();
        assert_eq!(g[0].yahoo_symbol(), "TRANSWORLD.NS");
    }

    /// The live payload mixes types: `"22.86 Cr"` for turnover on some rows, plain numbers on
    /// others. One formatted value must not reject the whole list.
    #[test]
    fn formatted_numbers_are_read_rather_than_rejected() {
        let payload = r#"{"allSec":{"data":[
            {"symbol":"ALEMBICLTD","ltp":"113.21","prev_price":99.15,"perChange":"14.18","turnover":"22.86 Cr"},
            {"symbol":"IOLCP","ltp":210.12,"prev_price":"184.28","perChange":"14.02","turnover":"1.5 L"},
            {"symbol":"ODDROW","ltp":"-","prev_price":"-","perChange":"-","turnover":"-"}
        ]}}"#;
        let g = parse_gainers(payload, 10).unwrap();
        assert_eq!(g.len(), 2, "the row with no price is skipped, not fatal");
        assert!((g[0].turnover.unwrap() - 2.286e8).abs() < 1.0);
        assert!((g[1].turnover.unwrap() - 1.5e5).abs() < 1.0);
    }

    #[test]
    fn a_payload_without_allsec_is_an_error_not_an_empty_day() {
        assert!(parse_gainers(r#"{"data":"Missing index or key."}"#, 10).is_err());
    }
}
