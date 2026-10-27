pub mod amfi;
pub mod search;
pub mod yahoo;

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::models::{Bar, Headline};

/// Everything the engine needs from the outside market. Tests implement this with fakes, so
/// no test reaches the network.
pub trait MarketSource: Send + Sync {
    fn bars<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>>;

    /// Intraday tape. `interval`/`range` follow Yahoo's vocabulary (15m/5d, 60m/7d).
    fn intraday<'a>(
        &'a self,
        symbol: &'a str,
        interval: &'a str,
        range: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>>;

    fn news<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>>;
}

/// Search across both sources. Yahoo for exchange-listed instruments, AMFI for Indian funds.
pub async fn search(
    client: &reqwest::Client,
    query: &str,
    amfi_text: Option<&str>,
) -> Result<Vec<search::Hit>> {
    let url = "https://query1.finance.yahoo.com/v1/finance/search";
    let body = client
        .get(url)
        .query(&[("q", query), ("quotesCount", "10"), ("newsCount", "0")])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut hits = search::parse_yahoo(&body).unwrap_or_default();
    if let Some(text) = amfi_text {
        hits.extend(search::search_amfi(text, query, 5));
    }
    Ok(hits)
}

pub struct YahooSource {
    client: reqwest::Client,
}

impl YahooSource {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: yahoo::client()?,
        })
    }
}

impl MarketSource for YahooSource {
    fn bars<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        Box::pin(async move { yahoo::fetch_bars(&self.client, symbol).await })
    }

    fn intraday<'a>(
        &'a self,
        symbol: &'a str,
        interval: &'a str,
        range: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        Box::pin(async move { yahoo::fetch_intraday(&self.client, symbol, interval, range).await })
    }

    fn news<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>> {
        Box::pin(async move { yahoo::fetch_news(&self.client, symbol).await })
    }
}
