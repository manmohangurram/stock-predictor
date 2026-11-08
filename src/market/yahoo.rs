use std::collections::HashSet;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;

use crate::models::{Bar, Headline};

const CHART_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const SEARCH_URL: &str = "https://query1.finance.yahoo.com/v1/finance/search";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) stock-predicter/0.1";

/// All Yahoo access goes through this module. Endpoint drift touches one file; the client itself
/// comes from `crate::http` so the proxy is decided in one place.
pub fn client(rt: &crate::rules::runtime::Runtime) -> Result<Client> {
    crate::http::client_with(rt, 20, Some(UA))
}

fn rfc3339(secs: i64) -> String {
    DateTime::from_timestamp(secs, 0)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn num(v: Option<&serde_json::Value>, idx: usize) -> Option<f64> {
    v.and_then(|a| a.get(idx)).and_then(|x| x.as_f64())
}

pub fn parse_chart(body: &str) -> Result<Vec<Bar>> {
    let v: serde_json::Value = serde_json::from_str(body).context("chart: invalid json")?;
    let result = v
        .pointer("/chart/result")
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow!("chart: missing chart.result"))?;
    if result.is_empty() {
        return Ok(Vec::new());
    }

    let first = &result[0];
    let timestamps = first
        .get("timestamp")
        .and_then(|t| t.as_array())
        .ok_or_else(|| anyhow!("chart: missing timestamp"))?;
    let quote = first
        .pointer("/indicators/quote/0")
        .ok_or_else(|| anyhow!("chart: missing indicators.quote"))?;

    let (open, high, low, close, vol) = (
        quote.get("open"),
        quote.get("high"),
        quote.get("low"),
        quote.get("close"),
        quote.get("volume"),
    );

    let mut bars = Vec::with_capacity(timestamps.len());
    for (i, t) in timestamps.iter().enumerate() {
        let Some(close) = num(close, i) else {
            continue; // halted / missing candle: skip, never fabricate
        };
        bars.push(Bar {
            ts: rfc3339(t.as_i64().unwrap_or(0)),
            open: num(open, i).unwrap_or(close),
            high: num(high, i).unwrap_or(close),
            low: num(low, i).unwrap_or(close),
            close,
            volume: num(vol, i).unwrap_or(0.0) as i64,
        });
    }
    Ok(bars)
}

/// Headlines only. The chosen endpoint carries title/publisher/date/link and no body at
/// all, which is exactly what the token budget wants.
pub fn parse_news(body: &str) -> Result<Vec<Headline>> {
    let v: serde_json::Value = serde_json::from_str(body).context("news: invalid json")?;
    let arr = v
        .get("news")
        .and_then(|n| n.as_array())
        .ok_or_else(|| anyhow!("news: missing news[] — endpoint shape changed"))?;

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in arr {
        let Some(url) = item.get("link").and_then(|l| l.as_str()) else {
            continue;
        };
        let Some(title) = item.get("title").and_then(|t| t.as_str()) else {
            continue;
        };
        if !seen.insert(url.to_string()) {
            continue;
        }
        out.push(Headline {
            id: 0,
            title: title.to_string(),
            publisher: item
                .get("publisher")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string(),
            published_at: rfc3339(
                item.get("providerPublishTime")
                    .and_then(|t| t.as_i64())
                    .unwrap_or(0),
            ),
            url: url.to_string(),
        });
    }
    Ok(out)
}

pub fn chart_url(symbol: &str) -> String {
    format!("{CHART_URL}/{symbol}")
}

/// A missing chart is normal for mutual funds, which Yahoo does not carry for India.
/// The caller decides whether that is an error (stocks) or expected (funds).
pub async fn fetch_bars(client: &Client, symbol: &str) -> Result<Vec<Bar>> {
    fetch_chart(client, symbol, "1d", "1y").await
}

/// Intraday tape. Yahoo's intervals that actually exist: 1m, 5m, 15m, 30m, 60m, 90m, 1h.
/// There is no 3-hour interval; anything coarser has to be aggregated by us.
pub async fn fetch_intraday(
    client: &Client,
    symbol: &str,
    interval: &str,
    range: &str,
) -> Result<Vec<Bar>> {
    fetch_chart(client, symbol, interval, range).await
}

async fn fetch_chart(
    client: &Client,
    symbol: &str,
    interval: &str,
    range: &str,
) -> Result<Vec<Bar>> {
    let url = chart_url(symbol);
    let body = client
        .get(url)
        .query(&[("range", range), ("interval", interval)])
        .send()
        .await
        .context("chart: request")?
        .error_for_status()
        .context("chart: http status")?
        .text()
        .await
        .context("chart: read body")?;
    parse_chart(&body).with_context(|| format!("chart: parse {symbol}"))
}

/// Yahoo's search endpoint returns nothing for an exchange-qualified ticker
/// (`INFY.NS` → 0 results), but finds the company by bare symbol (`INFY` → 5+).
pub fn news_query(symbol: &str) -> &str {
    match symbol.rsplit_once('.') {
        Some((bare, "NS")) | Some((bare, "BO")) => bare,
        _ => symbol,
    }
}

pub async fn fetch_news(client: &Client, symbol: &str) -> Result<Vec<Headline>> {
    let body = client
        .get(SEARCH_URL)
        .query(&[
            ("q", news_query(symbol)),
            ("newsCount", "20"),
            ("quotesCount", "0"),
        ])
        .send()
        .await
        .context("news: request")?
        .error_for_status()
        .context("news: http status")?
        .text()
        .await
        .context("news: read body")?;
    parse_news(&body).with_context(|| format!("news: parse {symbol}"))
}
