pub mod amfi;
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
