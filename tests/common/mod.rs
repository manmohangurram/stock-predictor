//! Shared test doubles. Four copies of a fake market drifted apart; this is the one copy.
#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use stock_predictor::market::MarketSource;
use stock_predictor::models::{Bar, Headline};

pub struct FakeMarket {
    pub bars: Vec<Bar>,
    pub fail_bars: bool,
    /// Symbols whose bar fetch should fail, for per-instrument isolation tests.
    pub fail_symbols: Vec<String>,
    pub news: Vec<Headline>,
    /// Optional separate intraday tape; defaults to `bars`.
    pub tape: Option<Vec<Bar>>,
}

impl FakeMarket {
    pub fn ok(bars: Vec<Bar>) -> Self {
        Self {
            bars,
            fail_bars: false,
            fail_symbols: Vec::new(),
            news: vec![headline("headline", "https://example.com/h")],
            tape: None,
        }
    }

    pub fn with_news(mut self, news: Vec<Headline>) -> Self {
        self.news = news;
        self
    }

    pub fn without_news(mut self) -> Self {
        self.news = Vec::new();
        self
    }

    pub fn failing_for(mut self, symbol: &str) -> Self {
        self.fail_symbols.push(symbol.to_string());
        self
    }

    pub fn with_tape(mut self, tape: Vec<Bar>) -> Self {
        self.tape = Some(tape);
        self
    }

    pub fn empty() -> Self {
        Self {
            bars: Vec::new(),
            fail_bars: false,
            fail_symbols: Vec::new(),
            news: Vec::new(),
            tape: None,
        }
    }
}

impl MarketSource for FakeMarket {
    fn bars<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        let out = if self.fail_bars || self.fail_symbols.iter().any(|s| s == symbol) {
            Err(anyhow::anyhow!("simulated outage for {symbol}"))
        } else {
            Ok(self.bars.clone())
        };
        Box::pin(async move { out })
    }

    fn intraday<'a>(
        &'a self,
        _symbol: &'a str,
        _interval: &'a str,
        _range: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        let out = Ok(self.tape.clone().unwrap_or_else(|| self.bars.clone()));
        Box::pin(async move { out })
    }

    fn constituents<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<stock_predictor::market::Constituents>> + Send + 'a>> {
        // Two names with a rising tape, so the plan has a candidate without touching NSE.
        let out = Ok(vec![
            ("INFY.NS".to_string(), "Infosys".to_string()),
            ("TCS.NS".to_string(), "Tata Consultancy".to_string()),
        ]);
        Box::pin(async move { out })
    }

    fn news<'a>(
        &'a self,
        _symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>> {
        let news = self.news.clone();
        Box::pin(async move { Ok(news) })
    }
}

pub fn bar(ts: &str, close: f64) -> Bar {
    Bar {
        ts: ts.into(),
        open: close,
        high: close * 1.005,
        low: close * 0.995,
        close,
        volume: 10_000,
    }
}

pub fn headline(title: &str, url: &str) -> Headline {
    Headline {
        id: 0,
        title: title.into(),
        publisher: "Reuters".into(),
        published_at: "2026-10-25T00:00:00Z".into(),
        url: url.into(),
    }
}

pub fn headlines(n: usize) -> Vec<Headline> {
    (0..n)
        .map(|i| {
            headline(
                &format!("headline {i}"),
                &format!("https://example.com/{i}"),
            )
        })
        .collect()
}

/// A rising sawtooth: last close above both averages, RSI moderate.
pub fn rising_bars(n: usize) -> Vec<Bar> {
    (0..n)
        .map(|i| {
            let close = 95.0 + (i % 10) as f64 * 0.5 + i as f64 * 0.02;
            Bar {
                ts: format!("2026-01-01T00:00:{}Z", i % 60),
                open: close,
                high: close + 0.8,
                low: close - 0.8,
                close,
                volume: 10_000,
            }
        })
        .collect()
}
