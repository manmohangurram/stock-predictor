//! A real call to DeepSeek, to prove the prompt gets a response the system can actually use.
//!
//! Ignored by default: it costs money and needs a key. Run it deliberately:
//!   DEEPSEEK_API_KEY=sk-... cargo test --test live_deepseek -- --ignored --nocapture

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use stock_predicter::rules::runtime::Runtime;
use stock_predicter::llm::deepseek::{self, ChatRequest, HttpReply, Transport};
use stock_predicter::llm::parse::{self, LlmVerdict};
use stock_predicter::llm::prompts::{self, PromptContext};
use stock_predicter::models::{AssetClass, Headline, Instrument};
use stock_predicter::rules::indicators::Indicators;

struct Real {
    inner: deepseek::ReqwestTransport,
}

impl Transport for Real {
    fn post<'a>(
        &'a self,
        url: &'a str,
        body: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<HttpReply>> + Send + 'a>> {
        self.inner.post(url, body)
    }
}

fn key() -> String {
    std::env::var("DEEPSEEK_API_KEY").unwrap_or_default()
}

// The provider key comes from the environment for these opt-in checks only.


fn ctx<'a>(i: &'a Instrument, x: &'a Indicators, h: &'a [Headline]) -> PromptContext<'a> {
    PromptContext {
        instrument: i,
        ind: x,
        last_close: 1163.0,
        cash_available: 96_388.86,
        target_profit_pct: Some(5.0),
        limits: "trade size 500 to 50000 per fill; take-profit between 5% and 50%",
        window_hours: 6,
        headlines: h,
        price_window: "27 09:15 1161.20 1164.40 1160.80 1163.40 22100\n27 09:30 1163.40 1164.10 1162.50 1163.00 12500",
        daily_days: 10,
        daily_summary: "2026-10-24 1150.20, 2026-10-27 1163.00",
        positions: "INFY.NS qty 17.000 at cost 1163.00, stop 1104.85, target 1279.30",
        recent_trades: "2026-10-20 BUY 17.000 @ 1163.00 (open)",
        last_decision: "none",
        now: chrono::Utc::now(),
        extracts: &[],
        price_history: "",
    }
}

#[tokio::test]
#[ignore = "makes a real, billed API call; run with --ignored and a key"]
async fn deepseek_returns_a_response_the_system_can_use() {
    assert!(!key().is_empty(), "DEEPSEEK_API_KEY is required");

    let dir = tempfile::tempdir().unwrap();
    let pool = stock_predicter::db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();

    let i = Instrument {
        id: 1,
        symbol: "INFY.NS".into(),
        class: AssetClass::Stock,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    };
    let x = Indicators {
        sma50: 1084.66,
        sma200: 1222.06,
        rsi14: 64.7,
        atr14: 42.41,
        high52: 1728.0,
        low52: 980.4,
    };
    let h = vec![Headline {
        id: 1,
        title: "Infosys guidance raised".into(),
        publisher: "Reuters".into(),
        published_at: "2026-10-25T00:00:00Z".into(),
        url: "https://example.com/1".into(),
    }];

    let rendered = prompts::render("decide_v1", &ctx(&i, &x, &h)).unwrap();
    let real = Real {
        inner: deepseek::ReqwestTransport::new(key(), &Runtime::default()).unwrap(),
    };
    let model = stock_predicter::llm::provider::ModelConfig {
        provider: stock_predicter::llm::provider::Provider::Deepseek,
        model: "deepseek-chat".into(),
        base_url: "https://api.deepseek.com".into(),
        api_key: key(),
    };
    let reply = deepseek::chat(
        &pool,
        &model,
        &real,
        ChatRequest {
            conversation_id: "live-check".into(),
            round: 1,
            prompt: rendered.text.clone(),
            prompt_template: rendered.template.clone(),
            prompt_hash: rendered.hash.clone(),
            purpose: "live contract check".into(),
        },
    )
    .await
    .expect("the API answered");

    println!(
        "\n--- raw response ---\n{}\n--------------------",
        reply.content
    );

    let verdict = parse::verdict(&reply.content).expect("response matches the agreed shape");
    match verdict {
        LlmVerdict::Decide(p) => {
            println!(
                "decided: {:?} entry {} stop {:?} target {:?}",
                p.action, p.entry, p.stop, p.target
            );
            if p.action == stock_predicter::models::Action::Buy {
                assert!(p.stop.is_some(), "a BUY must carry a stop");
                assert!(p.target.is_some(), "a BUY must carry a take-profit");
            }
            assert!(!p.cites.is_empty(), "a decision must cite a headline");
            assert!(
                p.cites.iter().all(|id| h.iter().any(|x| x.id == *id)),
                "citations must be ids that were sent"
            );
        }
        LlmVerdict::NeedMore(more) => {
            println!("asked for more: {more:?}");
            assert!(!more.is_empty());
        }
        LlmVerdict::Pass { why } => println!("passed: {why}"),
    }
}

/// The case that matters most: a clean uptrend with material news and no position. If the model
/// says BUY, it must also say where to get out on both sides — that is the whole contract.
#[tokio::test]
#[ignore = "makes a real, billed API call; run with --ignored and a key"]
async fn deepseek_buy_carries_both_exits() {
    assert!(!key().is_empty(), "DEEPSEEK_API_KEY is required");

    let dir = tempfile::tempdir().unwrap();
    let pool = stock_predicter::db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();

    let i = Instrument {
        id: 1,
        symbol: "INFY.NS".into(),
        class: AssetClass::Stock,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    };
    // A coherent uptrend with room to the 52-week high: close above SMA50 above SMA200,
    // RSI moderate, ATR wide enough for sane levels, price 14% below the high.
    let x = Indicators {
        sma50: 1150.0,
        sma200: 1050.0,
        rsi14: 58.0,
        atr14: 25.0,
        high52: 1400.0,
        low52: 900.0,
    };
    let h = vec![Headline {
        id: 1,
        title: "Infosys raises full-year guidance by 200bps".into(),
        publisher: "Reuters".into(),
        published_at: "2026-10-27T00:00:00Z".into(),
        url: "https://example.com/1".into(),
    }];

    let mut c = ctx(&i, &x, &h);
    c.last_close = 1200.0;
    // Every window must agree with last_close, or the model rightly refuses an incoherent picture.
    c.daily_summary = "2026-10-20 1150.00, 2026-10-21 1164.00, 2026-10-22 1171.00, 2026-10-23 1186.00, 2026-10-24 1191.00, 2026-10-27 1200.00";
    c.price_window = "27 09:15 1194.00 1197.50 1193.20 1196.80 18400\n27 09:30 1196.80 1199.20 1196.10 1198.60 14200\n27 09:45 1198.60 1200.80 1197.90 1200.00 12100";
    c.positions = "INFY.NS: not held";
    c.recent_trades = "none in the last 7 days";
    c.daily_days = 6;
    c.window_hours = 6;

    let rendered = prompts::render("decide_v1", &c).unwrap();
    let real = Real {
        inner: deepseek::ReqwestTransport::new(key(), &Runtime::default()).unwrap(),
    };
    let model = stock_predicter::llm::provider::ModelConfig {
        provider: stock_predicter::llm::provider::Provider::Deepseek,
        model: "deepseek-chat".into(),
        base_url: "https://api.deepseek.com".into(),
        api_key: key(),
    };
    let reply = deepseek::chat(
        &pool,
        &model,
        &real,
        ChatRequest {
            conversation_id: "live-buy".into(),
            round: 1,
            prompt: rendered.text.clone(),
            prompt_template: rendered.template.clone(),
            prompt_hash: rendered.hash.clone(),
            purpose: "live buy contract".into(),
        },
    )
    .await
    .expect("the API answered");

    println!(
        "\n--- raw response ---\n{}\n--------------------",
        reply.content
    );
    let verdict = parse::verdict(&reply.content).expect("shape");

    match verdict {
        LlmVerdict::Decide(p) => {
            println!(
                "decided: {:?} entry {} stop {:?} target {:?} cites {:?}",
                p.action, p.entry, p.stop, p.target, p.cites
            );
            if p.action == stock_predicter::models::Action::Buy {
                let stop = p.stop.expect("BUY must carry a stop");
                let target = p.target.expect("BUY must carry a take-profit");
                assert!(
                    stop < p.entry,
                    "stop {stop} must be below entry {}",
                    p.entry
                );
                assert!(
                    target > p.entry,
                    "target {target} must be above entry {}",
                    p.entry
                );
                assert!(
                    (p.entry - 1200.0).abs() <= x.atr14,
                    "entry {} must sit within one ATR of the close",
                    p.entry
                );
            }
            assert_eq!(p.cites, vec![1], "must cite the headline that drove it");
            assert!(
                p.rationale.split_whitespace().count() < 120,
                "rationale should be short"
            );
        }
        // Passing on a clean setup is allowed by the contract; the exit requirement is enforced
        // by the validator either way. Recorded loudly so the run is not mistaken for a BUY.
        other => println!("note: model passed on a clean setup: {other:?}"),
    }
}
