//! The three paths that run on every poll tick, measured so a regression is visible.
//!
//! cargo bench --bench hot_paths

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use stock_predictor::models::{Bar, Headline, Instrument};
use stock_predictor::rules::indicators::{self, Indicators};
use stock_predictor::{llm::prompts, store};

fn bars(n: usize) -> Vec<Bar> {
    (0..n)
        .map(|i| {
            let close = 100.0 + (i % 10) as f64 * 0.4 + i as f64 * 0.01;
            Bar {
                ts: format!("2026-{:02}-{:02}T03:45:00Z", (i % 12) + 1, (i % 28) + 1),
                open: close,
                high: close + 0.5,
                low: close - 0.5,
                close,
                volume: 10_000,
            }
        })
        .collect()
}

fn bench_indicators(c: &mut Criterion) {
    let mut group = c.benchmark_group("indicators");
    for n in [250usize, 500, 1000] {
        let series = bars(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &series, |b, series| {
            b.iter(|| indicators::compute(black_box(series)).unwrap())
        });
    }
    group.finish();
}

fn bench_prompt(c: &mut Criterion) {
    let instrument = Instrument {
        id: 1,
        symbol: "INFY.NS".into(),
        class: stock_predictor::models::AssetClass::Stock,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    };
    let ind = Indicators {
        sma50: 1084.66,
        sma200: 1222.06,
        rsi14: 64.7,
        atr14: 42.41,
        high52: 1728.0,
        low52: 980.4,
    };
    let headlines: Vec<Headline> = (0..20)
        .map(|i| Headline {
            id: i,
            title: format!("Headline number {i} about the company"),
            publisher: "Reuters".into(),
            published_at: "2026-10-25T00:00:00Z".into(),
            url: format!("https://example.com/{i}"),
        })
        .collect();
    let tape = (0..24)
        .map(|i| format!("27 09:{:02} 1163.00 1164.10 1162.50 1163.40 12500", i))
        .collect::<Vec<_>>()
        .join("\n");

    let ctx = prompts::PromptContext {
        instrument: &instrument,
        ind: &ind,
        last_close: 1163.0,
        cash_available: 96_388.86,
        target_profit_pct: Some(5.0),
        limits: "trade size 500 to 50000 per fill; take-profit between 5% and 50%",
        headlines: &headlines,
        window_hours: 6,
        price_window: &tape,
        daily_days: 10,
        daily_summary: "2026-10-24 1150.20, 2026-10-27 1163.00",
        fundamentals: "unavailable",
        positions: "INFY.NS qty 17.000 at cost 1163.00",
        recent_trades: "2026-10-20 BUY 17.000 @ 1163.00 (open)",
        last_decision: "none",
        now: chrono::Utc::now(),
        extracts: &[],
        price_history: "",
    };

    c.bench_function("prompt_render_20_headlines", |b| {
        b.iter(|| prompts::render(black_box("decide_v1"), black_box(&ctx)).unwrap())
    });
}

fn bench_bar_upsert(c: &mut Criterion) {
    // One connection, one table, one upsert of a full year of bars: the write the poll loop
    // does for every instrument on every tick.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.db");
    let (pool, id, series) = rt.block_on(async {
        let pool = stock_predictor::db::connect_and_migrate(path.to_str().unwrap())
            .await
            .unwrap();
        let id: (i64,) = sqlx::query_as(
            "INSERT INTO instruments (symbol, class, name, currency)
             VALUES ('INFY.NS','stock','Infosys','INR') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        (pool, id.0, bars(250))
    });

    // Cold: the whole year, as on first add.
    c.bench_function("store_bars_cold_250", |b| {
        b.iter(|| {
            rt.block_on(async {
                sqlx::query("DELETE FROM bars WHERE instrument_id = ?")
                    .bind(id)
                    .execute(&pool)
                    .await
                    .unwrap();
                store::bars(&pool, id, black_box(&series)).await.unwrap();
            })
        })
    });

    // Warm: one new bar, which is what a poll tick actually writes.
    c.bench_function("store_bars_warm_1", |b| {
        let mut tail = series.clone();
        let mut n = 0u32;
        b.iter(|| {
            n += 1;
            let last = tail.last_mut().unwrap();
            last.close += 0.01;
            last.ts = format!("2026-12-31T03:45:{:02}Z", n % 60);
            rt.block_on(async {
                store::bars(&pool, id, black_box(&tail)).await.unwrap();
            })
        })
    });
}

criterion_group!(benches, bench_indicators, bench_prompt, bench_bar_upsert);
criterion_main!(benches);
