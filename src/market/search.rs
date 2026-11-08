use anyhow::Result;
use serde::Deserialize;

use super::amfi;

/// One search hit, from either source. `code` is what the user adds.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub code: String,
    pub name: String,
    pub class: crate::models::AssetClass,
    pub venue: String,
    pub price: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Quotes {
    #[serde(default)]
    quotes: Vec<Quote>,
}

#[derive(Debug, Deserialize)]
struct Quote {
    symbol: String,
    #[serde(rename = "shortname", default)]
    short_name: Option<String>,
    #[serde(rename = "longname", default)]
    long_name: Option<String>,
    #[serde(rename = "exchDisp", default, alias = "exchdisp")]
    venue: Option<String>,
    #[serde(rename = "quoteType", default)]
    quote_type: Option<String>,
}

/// Yahoo covers exchange-listed instruments, and Indian mutual funds too — under opaque codes
/// like `0P00012ALS.BO`, which are priceable and addable. AMFI is the other source, by scheme code.
pub fn parse_yahoo(body: &str) -> Result<Vec<Hit>> {
    let q: Quotes = serde_json::from_str(body)?;
    Ok(q.quotes
        .into_iter()
        .filter_map(|x| {
            let class = match x.quote_type.as_deref() {
                Some("EQUITY") | Some("ETF") | Some("INDEX") => crate::models::AssetClass::Stock,
                Some("MUTUALFUND") => crate::models::AssetClass::MutualFund,
                _ => return None,
            };
            // For a fund the short name is the code, so the readable one is longname.
            let name = match class {
                crate::models::AssetClass::MutualFund => x.long_name.clone().or(x.short_name),
                _ => x.short_name.clone().or(x.long_name),
            }
            .unwrap_or_else(|| x.symbol.clone());
            Some(Hit {
                code: x.symbol,
                name,
                class,
                venue: x.venue.unwrap_or_default(),
                price: None,
            })
        })
        .collect())
}

/// AMFI's file is the only reliable source for Indian mutual funds.
pub fn search_amfi(navall: &str, query: &str, limit: usize) -> Vec<Hit> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for raw in navall.lines() {
        let line = raw.trim_end_matches('\r');
        let f: Vec<&str> = line.split(';').map(str::trim).collect();
        if f.len() < 8 || f[0].eq_ignore_ascii_case("scheme code") {
            continue;
        }
        let name = f[3];
        let haystack = line.to_ascii_lowercase();
        if !haystack.contains(&needle) {
            continue;
        }
        if f[5].eq_ignore_ascii_case("idcw-re-investment") {
            continue;
        }
        out.push(Hit {
            code: f[0].to_string(),
            name: format!("{} — {} {}", name, f[4], f[5]).trim().to_string(),
            class: crate::models::AssetClass::MutualFund,
            venue: "AMFI".into(),
            price: f[6].parse().ok(),
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Scheme code lookup used when adding a fund by code.
pub fn amfi_by_code(navall: &str, code: &str) -> Option<Hit> {
    amfi::parse_nav(navall, code).ok().map(|row| Hit {
        code: row.scheme_code,
        name: row.name,
        class: crate::models::AssetClass::MutualFund,
        venue: "AMFI".into(),
        price: Some(row.nav),
    })
}
