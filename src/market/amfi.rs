use anyhow::{anyhow, Context, Result};
use reqwest::Client;

const NAV_ALL_URL: &str = "https://www.amfiindia.com/spages/NAVAll.txt";

#[derive(Debug, Clone, PartialEq)]
pub struct NavRow {
    pub scheme_code: String,
    pub name: String,
    pub nav: f64,
    pub date: String,
}

/// AMFI publishes one `;`-delimited file a day. A missing scheme is an error: a zero NAV
/// would silently poison the cash ledger.
pub fn parse_nav(text: &str, scheme_code: &str) -> Result<NavRow> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Scheme Code") {
            continue;
        }
        let f: Vec<&str> = line.split(';').map(str::trim).collect();
        if f.len() < 6 || f[0] != scheme_code {
            continue;
        }
        let nav: f64 = f[4]
            .parse()
            .with_context(|| format!("amfi: bad nav {:?} for {scheme_code}", f[4]))?;
        if !nav.is_finite() || nav <= 0.0 {
            return Err(anyhow!("amfi: non-positive nav {nav} for {scheme_code}"));
        }
        return Ok(NavRow {
            scheme_code: f[0].to_string(),
            name: f[1].to_string(),
            nav,
            date: iso_date(f[5])?,
        });
    }
    Err(anyhow!("amfi: scheme {scheme_code} not found"))
}

fn iso_date(d: &str) -> Result<String> {
    chrono::NaiveDate::parse_from_str(d, "%d-%b-%Y")
        .map(|d| d.format("%Y-%m-%d").to_string())
        .map_err(|_| anyhow!("amfi: bad date {d:?}"))
}

pub async fn fetch_nav_all(client: &Client) -> Result<String> {
    Ok(client
        .get(NAV_ALL_URL)
        .send()
        .await
        .context("amfi: request")?
        .error_for_status()
        .context("amfi: http status")?
        .text()
        .await
        .context("amfi: read body")?)
}
