//! The one place an outbound HTTP client is built, so the proxy decision is made once.
//!
//! `.no_proxy()` comes first and unconditionally: reqwest otherwise honours `HTTP_PROXY`,
//! `HTTPS_PROXY` and `ALL_PROXY` from the environment, and the environment would then outrank the
//! value on the settings page. The UI is the only place a proxy is configured.

use anyhow::{Context, Result};
use reqwest::{Client, Proxy};

use crate::rules::runtime::Runtime;

/// A proxy must be an absolute http(s) URL with a host. reqwest is lenient about bare
/// `host:port`, and a lenient value here would fail much later, at request time.
pub fn validate_proxy(url: &str) -> Result<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!("proxy must be a http:// or https:// URL");
    }
    let parsed = reqwest::Url::parse(url).map_err(|e| anyhow::anyhow!("proxy URL {url:?}: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!("proxy must be a http:// or https:// URL");
    }
    if parsed.host_str().unwrap_or_default().is_empty() {
        anyhow::bail!("proxy URL has no host");
    }
    Ok(())
}

pub fn client(rt: &Runtime, timeout_secs: u64) -> Result<Client> {
    client_with(rt, timeout_secs, None)
}

/// Same, with a browser-ish user agent for the endpoints that expect one.
pub fn client_with(rt: &Runtime, timeout_secs: u64, user_agent: Option<&str>) -> Result<Client> {
    let mut b = Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(timeout_secs));
    if let Some(ua) = user_agent {
        b = b.user_agent(ua);
    }

    if let Some(url) = rt.proxy.as_deref() {
        validate_proxy(url)?;
        let mut proxy = Proxy::all(url).with_context(|| format!("proxy URL {url:?} is not usable"))?;
        if let Some(user) = rt.proxy_user.as_deref() {
            // Basic auth from the two fields, so the password is a secret like any other.
            proxy = proxy.basic_auth(user, rt.proxy_pass.as_deref().unwrap_or_default());
        }
        for host in rt
            .proxy_bypass
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|h| !h.is_empty())
        {
            proxy = proxy.no_proxy(reqwest::NoProxy::from_string(host));
        }
        b = b.proxy(proxy);
    }

    b.build().context("build http client")
}

/// The address the API is willing to show: host and port, never the userinfo.
pub fn proxy_host(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => match u.port() {
            Some(p) => format!("{}:{p}", u.host_str().unwrap_or_default()),
            None => u.host_str().unwrap_or_default().to_string(),
        },
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credentialled_proxy_url_shows_only_the_host() {
        assert_eq!(proxy_host("http://user:secret@10.0.0.2:3128"), "10.0.0.2:3128");
        assert_eq!(proxy_host("https://proxy.example.in"), "proxy.example.in");
        assert_eq!(proxy_host("not a url"), "");
    }

    #[test]
    fn a_bare_host_and_port_is_not_a_proxy() {
        assert!(validate_proxy("10.0.0.2:3128").is_err());
        assert!(validate_proxy("socks5://10.0.0.2:1080").is_err());
        assert!(validate_proxy("http://10.0.0.2:3128").is_ok());
        assert!(validate_proxy("https://user:pass@proxy.example.in:8443").is_ok());
    }
}
