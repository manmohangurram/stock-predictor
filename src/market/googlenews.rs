//! Company news from Google News' own RSS feed. Public, no key, English and the India edition,
//! with a real publisher and a date on every item.

use anyhow::{Context, Result};
use reqwest::Client;

use crate::models::Headline;

const URL: &str = "https://news.google.com/rss/search";

/// The last `days` of coverage for `company`. `when:` is Google News' own operator, so the window
/// is applied by the index, not by us after the fact.
pub async fn fetch_news(client: &Client, company: &str, days: u32) -> Result<Vec<Headline>> {
    // Aimed at what re-rates a stock, in the market it trades in. A bare name answers with prize
    // ceremonies and interviews.
    let query = format!(
        "\"{company}\" (stock OR shares OR results OR earnings OR guidance OR order OR profit OR downgrade OR upgrade) India when:{days}d"
    );
    let body = client
        .get(URL)
        .query(&[
            ("q", query.as_str()),
            ("hl", "en-IN"),
            ("gl", "IN"),
            ("ceid", "IN:en"),
        ])
        .send()
        .await
        .context("google news: request")?
        .error_for_status()
        .context("google news: http status")?
        .text()
        .await
        .context("google news: read body")?;
    Ok(parse(&body))
}

/// Publishers that report markets rather than reprint a press release. Preferred, not required:
/// an empty list is worse than a weaker source.
const TRUSTED: &[&str] = &[
    "reuters",
    "business standard",
    "the economic times",
    "economic times",
    "mint",
    "livemint",
    "moneycontrol",
    "cnbc",
    "the hindu businessline",
    "businessline",
    "financial express",
    "bloomberg",
    "ndtv profit",
    "the times of india",
    "barron",
    "marketwatch",
    "forbes india",
];

fn is_trusted(publisher: &str) -> bool {
    let p = publisher.to_lowercase();
    TRUSTED.iter().any(|t| p.contains(t))
}

/// Keep the market publishers when the feed has any, and everything when it has none.
pub fn prefer_market_sources(items: Vec<Headline>) -> Vec<Headline> {
    if items.iter().any(|h| is_trusted(&h.publisher)) {
        items.into_iter().filter(|h| is_trusted(&h.publisher)).collect()
    } else {
        items
    }
}

/// The items of a Google News RSS body. Hand-parsed: the shape is fixed and a dependency for one
/// feed is not worth it.
pub fn parse(body: &str) -> Vec<Headline> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("<item>") {
        let after = &rest[start + "<item>".len()..];
        let Some(end) = after.find("</item>") else {
            break;
        };
        let item = &after[..end];
        rest = &after[end + "</item>".len()..];

        let Some(link) = tag(item, "link") else { continue };
        let Some(raw_title) = tag(item, "title") else {
            continue;
        };
        let publisher = tag(item, "source").unwrap_or_default();
        // Google formats the title as "Headline - Publisher"; the publisher has its own tag.
        let title = match raw_title.rsplit_once(" - ") {
            Some((head, tail)) if !publisher.is_empty() && tail == publisher => head.to_string(),
            _ => raw_title,
        };
        let published_at = tag(item, "pubDate")
            .and_then(|d| chrono::DateTime::parse_from_rfc2822(&d).ok())
            .map(|d| d.with_timezone(&chrono::Utc).to_rfc3339())
            .unwrap_or_default();
        if published_at.is_empty() {
            continue;
        }
        out.push(Headline {
            id: 0,
            title,
            publisher,
            published_at,
            url: link,
        });
    }
    out
}

/// One tag's text, with the CDATA and entity forms RSS feeds actually use.
fn tag(item: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let start = item.find(&open)?;
    let after = &item[start..];
    let inline = after.find('>')?;
    if after[..inline].ends_with('/') {
        return None;
    }
    let body = &after[inline + 1..];
    let close = body.find(&format!("</{name}>"))?;
    let raw = body[..close].trim();
    let text = raw
        .strip_prefix("<![CDATA[")
        .and_then(|t| t.strip_suffix("]]>"))
        .unwrap_or(raw);
    Some(unescape(text))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::{parse, prefer_market_sources};

    const FEED: &str = r#"<?xml version="1.0"?><rss><channel>
      <item>
        <title>Infosys raises FY27 guidance after strong Q2 - Reuters</title>
        <link>https://news.google.com/rss/articles/AAA</link>
        <pubDate>Fri, 13 Nov 2026 10:30:00 GMT</pubDate>
        <source url="https://www.reuters.com">Reuters</source>
      </item>
      <item>
        <title><![CDATA[Infosys wins a large deal &amp; lifts outlook - The Economic Times]]></title>
        <link>https://news.google.com/rss/articles/BBB</link>
        <pubDate>Fri, 13 Nov 2026 08:00:00 GMT</pubDate>
        <source url="https://economictimes.indiatimes.com">The Economic Times</source>
      </item>
      <item>
        <title>no date, dropped</title>
        <link>https://news.google.com/rss/articles/CCC</link>
      </item>
    </channel></rss>"#;

    #[test]
    fn a_feed_becomes_dated_headlines_from_real_publishers() {
        let hits = parse(FEED);
        assert_eq!(hits.len(), 2, "an item without a date is not an article");
        assert_eq!(hits[0].title, "Infosys raises FY27 guidance after strong Q2");
        assert_eq!(hits[0].publisher, "Reuters");
        assert!(hits[0].published_at.starts_with("2026-11-13"));
        // CDATA and entities survive, and the publisher suffix is stripped.
        assert_eq!(hits[1].title, "Infosys wins a large deal & lifts outlook");
        assert_eq!(hits[1].publisher, "The Economic Times");
    }

    #[test]
    fn a_market_publisher_wins_when_the_feed_has_one() {
        let feed = r#"<rss><channel>
          <item><title>Infosys raises guidance - Prizes Weekly</title>
            <link>https://n/1</link><pubDate>Fri, 13 Nov 2026 10:30:00 GMT</pubDate>
            <source url="https://prizes.test">Prizes Weekly</source></item>
          <item><title>Infosys Q2 results beat estimates - Business Standard</title>
            <link>https://n/2</link><pubDate>Fri, 13 Nov 2026 09:00:00 GMT</pubDate>
            <source url="https://business-standard.com">Business Standard</source></item>
        </channel></rss>"#;
        let kept = prefer_market_sources(parse(feed));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].publisher, "Business Standard");
    }

    #[test]
    fn a_body_that_is_not_a_feed_is_empty_not_an_error() {
        assert!(parse("<html>not a feed</html>").is_empty());
        assert!(parse("").is_empty());
    }
}
