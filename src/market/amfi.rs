use anyhow::{anyhow, Context, Result};
use reqwest::Client;

/// AMFI moved this file to the portal host; the old www path redirects.
const NAV_ALL_URL: &str = "https://portal.amfiindia.com/spages/NAVAll.txt";

#[derive(Debug, Clone, PartialEq)]
pub struct NavRow {
    pub scheme_code: String,
    pub name: String,
    pub nav: f64,
    pub date: String,
}

/// The live file has changed shape twice. Column positions are therefore read from the
/// header when one is present, falling back to the historical six-column layout.
///
/// Current header:
/// Scheme Code;ISIN Div Payout;ISIN Div Reinvestment;Scheme Name;Plan;Option;NAV;Date
pub fn parse_nav(text: &str, scheme_code: &str) -> Result<NavRow> {
    let mut columns: Option<Vec<String>> = None;

    for raw in text.lines() {
        // the file is CRLF-terminated
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(';').map(str::trim).collect();

        if f[0].eq_ignore_ascii_case("scheme code") {
            columns = Some(f.iter().map(|c| c.to_ascii_lowercase()).collect());
            continue;
        }
        if f[0] != scheme_code {
            continue;
        }

        let (nav_i, date_i, name_i, plan_i, option_i) = match &columns {
            Some(c) => {
                let idx = |want: &str| {
                    c.iter().position(|h| h == want).ok_or_else(|| {
                        anyhow!("amfi: header has no {want:?} column — file shape changed")
                    })
                };
                (
                    idx("net asset value")?,
                    idx("date")?,
                    idx("scheme name")?,
                    c.iter().position(|h| h == "plan"),
                    c.iter().position(|h| h == "option"),
                )
            }
            None => (4, 5, 1, None, None),
        };

        let raw_nav = f
            .get(nav_i)
            .ok_or_else(|| anyhow!("amfi: row too short for nav column {nav_i}"))?;
        let nav: f64 = raw_nav
            .parse()
            .with_context(|| format!("amfi: bad nav {raw_nav:?} for {scheme_code}"))?;
        if !nav.is_finite() || nav <= 0.0 {
            return Err(anyhow!("amfi: non-positive nav {nav} for {scheme_code}"));
        }
        let date = f
            .get(date_i)
            .ok_or_else(|| anyhow!("amfi: row too short for date column"))?;

        let mut name = f.get(name_i).copied().unwrap_or("").to_string();
        for i in [plan_i, option_i].into_iter().flatten() {
            if let Some(part) = f.get(i).filter(|p| !p.is_empty() && **p != "-") {
                name.push_str(" — ");
                name.push_str(part);
            }
        }

        return Ok(NavRow {
            scheme_code: f[0].to_string(),
            name,
            nav,
            date: iso_date(date)?,
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
    client
        .get(NAV_ALL_URL)
        .send()
        .await
        .context("amfi: request")?
        .error_for_status()
        .context("amfi: http status")?
        .text()
        .await
        .context("amfi: read body")
}
