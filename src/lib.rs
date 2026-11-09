/// Lowercase hex of a digest. sha2 0.11's `finalize()` dropped `LowerHex`.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub mod api;
pub mod backtest;
pub mod config;
pub mod db;
pub mod engine;
pub mod fundscan;
pub mod http;
pub mod llm;
pub mod market;
pub mod models;
pub mod notify;
pub mod rules;
pub mod runs;
pub mod schedule;
pub mod secrets;
pub mod seed;
pub mod store;
pub mod trades;
pub mod trend;
