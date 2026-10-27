use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::market::{amfi, yahoo};
use crate::models::AssetClass;
use crate::rules::budget::{self, CashKind, NewCashEntry};

/// A small, real watchlist. Everything downstream uses live Yahoo data for these.
pub const STOCKS: &[(&str, &str)] = &[
    ("INFY.NS", "Infosys"),
    ("TCS.NS", "Tata Consultancy Services"),
    ("HDFCBANK.NS", "HDFC Bank"),
    ("RELIANCE.NS", "Reliance Industries"),
];

pub const MF_SCHEME: &str = "120503";
const SPEND_PER_STOCK: f64 = 20_000.0;
const SPEND_PER_FUND: f64 = 25_000.0;

#[derive(Debug, Default)]
pub struct SeedReport {
    pub instruments_added: usize,
    pub positions_added: usize,
    pub skipped: Vec<String>,
}

/// Seeds only what it can price from a live source: a position at the real last close, or at
/// the scheme's real NAV. Nothing here invents a number.
pub async fn run(pool: &SqlitePool, with_positions: bool) -> Result<SeedReport> {
    let client = yahoo::client()?;
    let mut report = SeedReport::default();

    for (symbol, name) in STOCKS {
        let added = insert_instrument(pool, symbol, AssetClass::Stock, name).await?;
        report.instruments_added += added;

        if !with_positions {
            continue;
        }
        let id = instrument_id(pool, symbol).await?;
        if has_position(pool, id).await? {
            continue;
        }
        match yahoo::fetch_bars(&client, symbol).await {
            Ok(bars) if !bars.is_empty() => {
                let close = bars.last().unwrap().close;
                let qty = (SPEND_PER_STOCK / close).floor();
                if qty < 1.0 {
                    report
                        .skipped
                        .push(format!("{symbol}: price too high for the stake"));
                    continue;
                }
                if !budget::fits(pool, qty * close).await? {
                    report
                        .skipped
                        .push(format!("{symbol}: does not fit the budget"));
                    continue;
                }
                sqlx::query(
                    "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, stop, target, state)
                     VALUES (?, ?, ?, ?, ?, ?, 'open')",
                )
                .bind(id)
                .bind(qty)
                .bind(close)
                .bind(chrono::Utc::now().to_rfc3339())
                .bind(close * 0.95)
                .bind(close * 1.10)
                .execute(pool)
                .await?;
                budget::record(
                    pool,
                    NewCashEntry {
                        kind: CashKind::Buy,
                        amount: -(qty * close),
                        reference: format!("seed buy {symbol} @ {close:.2}"),
                    },
                )
                .await?;
                report.positions_added += 1;
            }
            Ok(_) => report.skipped.push(format!("{symbol}: no price data")),
            Err(e) => report.skipped.push(format!("{symbol}: {e}")),
        }
    }

    // One real mutual fund, priced from AMFI's daily file rather than invented.
    let added = insert_instrument(
        pool,
        "AXISELSS",
        AssetClass::MutualFund,
        "Axis ELSS Tax Saver",
    )
    .await?;
    report.instruments_added += added;
    if with_positions {
        let id = instrument_id(pool, "AXISELSS").await?;
        if !has_position(pool, id).await? {
            match amfi::fetch_nav_all(&client)
                .await
                .and_then(|text| amfi::parse_nav(&text, MF_SCHEME))
            {
                Ok(nav) => {
                    let qty = budget::units_floor(SPEND_PER_FUND / nav.nav);
                    if qty > 0.0 && budget::fits(pool, qty * nav.nav).await? {
                        sqlx::query(
                            "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state)
                             VALUES (?, ?, ?, ?, 'open')",
                        )
                        .bind(id)
                        .bind(qty)
                        .bind(nav.nav)
                        .bind(chrono::Utc::now().to_rfc3339())
                        .execute(pool)
                        .await?;
                        budget::record(
                            pool,
                            NewCashEntry {
                                kind: CashKind::Buy,
                                amount: -(qty * nav.nav),
                                reference: format!("seed buy {} @ {}", nav.name, nav.nav),
                            },
                        )
                        .await?;
                        report.positions_added += 1;
                    }
                }
                Err(e) => report
                    .skipped
                    .push(format!("AXISELSS: AMFI unavailable ({e})")),
            }
        }
    }

    Ok(report)
}

async fn insert_instrument(
    pool: &SqlitePool,
    symbol: &str,
    class: AssetClass,
    name: &str,
) -> Result<usize> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES (?, ?, ?, 'INR')",
    )
    .bind(symbol)
    .bind(class.as_str())
    .bind(name)
    .execute(pool)
    .await
    .context("seed instrument")?;
    Ok(res.rows_affected() as usize)
}

async fn instrument_id(pool: &SqlitePool, symbol: &str) -> Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = ?")
        .bind(symbol)
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

async fn has_position(pool: &SqlitePool, instrument_id: i64) -> Result<bool> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM positions WHERE instrument_id = ? AND state = 'open'")
            .bind(instrument_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0 > 0)
}
