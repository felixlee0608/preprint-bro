//! bioRxiv and medRxiv: metadata from api.biorxiv.org, full text as JATS XML.
//!
//! The metadata API is open and lightweight, and serves both servers (the
//! `server` path element picks which); it is fetched with reqwest. Full text
//! lives on www.biorxiv.org / www.medrxiv.org behind Cloudflare, which
//! fingerprints the HTTP client (reqwest's rustls handshake gets
//! challenged/429'd) and rate-limits by IP, so the full text is fetched with
//! the system `curl` using browser headers, once per build — comfortably
//! inside the limit as long as builds aren't hammered.
//!
//! A bare `10.1101/…` DOI could be from either server, so `meta` takes the
//! candidate servers in order and returns the first match, with its resolved
//! source. A link that named the host passes just that one.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::time::Duration;

use crate::arxiv::Meta;
use crate::preprint::Source;

#[derive(Deserialize)]
struct ApiResponse {
    #[allow(dead_code)]
    messages: Vec<ApiMessage>,
    #[serde(default)]
    collection: Vec<ApiPaper>,
}

#[derive(Deserialize)]
struct ApiMessage {
    #[allow(dead_code)]
    status: String,
}

#[derive(Deserialize)]
struct ApiPaper {
    #[serde(default)]
    title: String,
    #[serde(default)]
    authors: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    category: String,
    #[serde(default, rename = "abstract")]
    r#abstract: String,
    /// The API's own JATS XML full-text URL (a `.source.xml` path).
    #[serde(default)]
    jatsxml: String,
}

pub struct Client {
    http: reqwest::Client,
}

impl Client {
    pub fn new() -> Result<Self> {
        // A browser-like UA and headers: bioRxiv/medRxiv full text sits behind
        // Cloudflare, which answers 429 / challenge pages to requests that do
        // not look like a browser (a bare custom UA was enough to get blocked).
        // The papers are openly licensed; this just fetches each one once.
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::ACCEPT, "application/xml,text/xml,application/xhtml+xml,*/*;q=0.8".parse()?);
        headers.insert(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9".parse()?);
        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .default_headers(headers)
            .timeout(Duration::from_secs(90))
            .build()?;
        Ok(Self { http })
    }

    /// Metadata for one DOI, trying `servers` in order. Returns the first
    /// match, with the JATS XML URL the API points at for the full text
    /// (empty when the API gave none). `None` when no server has the paper.
    pub async fn meta(&self, servers: &[Source], doi: &str) -> Result<Option<(Meta, String)>> {
        for source in servers {
            let Some(server) = source.api_server() else { continue };
            let url = format!("https://api.biorxiv.org/details/{server}/{doi}/na/json");
            let body = self
                .http
                .get(&url)
                .send()
                .await
                .with_context(|| format!("GET {url}"))?
                .error_for_status()
                .with_context(|| format!("GET {url}"))?
                .text()
                .await?;
            let parsed: ApiResponse =
                serde_json::from_str(&body).context("api.biorxiv.org returned malformed JSON")?;
            let Some(p) = parsed.collection.into_iter().next() else { continue };
            let jatsxml = p.jatsxml.clone();
            return Ok(Some((meta_from_api(*source, doi, p)?, jatsxml)));
        }
        Ok(None)
    }

    /// The JATS XML full text at the API's own URL. `None` when the server
    /// answers 404 (a version that stopped existing). A 403/429 means the
    /// site's Cloudflare is rate-limiting this IP: that is surfaced as an
    /// error with a hint, not silently treated as "no full text".
    ///
    /// Full text is fetched with the system `curl`, not reqwest: Cloudflare
    /// fingerprints the HTTP client's TLS stack, and reqwest's rustls hand
    /// shake is challenged/429'd even with browser headers, while the
    /// system curl (Secure Transport on macOS, OpenSSL elsewhere) passes.
    /// Both platforms ship curl, so self-hosting still needs nothing extra.
    pub async fn full_xml(&self, url: &str) -> Result<Option<String>> {
        // Offline / rate-limited path: when PREPRINT_XML_FILE points at a
        // local JATS XML file, read it instead of hitting the network.
        // Handy for testing against a paper while its server is
        // rate-limiting your IP (bioRxiv's Cloudflare blocks are aggressive).
        if let Ok(path) = std::env::var("PREPRINT_XML_FILE") {
            if !path.trim().is_empty() {
                let body = tokio::fs::read_to_string(&path)
                    .await
                    .with_context(|| format!("reading PREPRINT_XML_FILE {path}"))?;
                return Ok(Some(body));
            }
        }
        const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        let url = url.to_string();
        let out = tokio::process::Command::new("curl")
            .args(["-sSL", "-m", "30"])
            .arg("-A")
            .arg(UA)
            .arg("-H")
            .arg("Accept: application/xml,text/xml,application/xhtml+xml,*/*;q=0.8")
            .arg("-H")
            .arg("Accept-Language: en-US,en;q=0.9")
            .arg("-w")
            .arg("\n%{http_code}")
            .arg(&url)
            .output()
            .await
            .with_context(|| format!("running curl for {url}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let code: u16 = stdout.rsplit('\n').next().unwrap_or("").trim().parse().unwrap_or(0);
        let body = stdout.trim_end_matches('\n').rsplit_once('\n').map(|(b, _)| b.to_string()).unwrap_or_default();
        if code == 0 {
            bail!("curl could not reach {url} (exit {})", out.status.code().unwrap_or(-1));
        }
        match code {
            404 => return Ok(None),
            403 | 429 => bail!(
                "{url} answered {code} — its CDN is rate-limiting this IP \
                 (preprint servers throttle aggressively; a few minutes between builds usually clears it). \
                 Retry the build a little later."
            ),
            _ if code < 200 || code >= 300 => bail!("{url} answered HTTP {code}"),
            _ => {}
        }
        if body.trim().is_empty() {
            bail!("{url} answered 200 with an empty body");
        }
        Ok(Some(body))
    }
}

fn meta_from_api(source: Source, doi: &str, p: ApiPaper) -> Result<Meta> {
    let authors: Vec<String> = p.authors.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
    let published = chrono::NaiveDate::parse_from_str(&p.date, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|t| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(t, chrono::Utc));
    let categories = if p.category.trim().is_empty() { Vec::new() } else { vec![p.category.trim().to_string()] };
    Ok(Meta {
        source,
        id: doi.to_string(),
        version: p.version.parse().unwrap_or(1),
        title: p.title,
        authors,
        abstract_: p.r#abstract,
        categories,
        license: license_url(&p.license),
        published,
    })
}

/// The API's license abbreviation to a URL the course page can link to.
fn license_url(abbr: &str) -> Option<String> {
    match abbr.trim().to_ascii_lowercase().as_str() {
        "cc0" => Some("http://creativecommons.org/publicdomain/zero/1.0/".into()),
        "cc_by" => Some("http://creativecommons.org/licenses/by/4.0/".into()),
        "cc_by_nc" => Some("http://creativecommons.org/licenses/by-nc/4.0/".into()),
        "cc_by_nd" => Some("http://creativecommons.org/licenses/by-nd/4.0/".into()),
        "cc_by_nc_sa" => Some("http://creativecommons.org/licenses/by-nc-sa/4.0/".into()),
        "cc_by_nc_nd" => Some("http://creativecommons.org/licenses/by-nc-nd/4.0/".into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_abbreviations_resolve() {
        assert_eq!(license_url("cc_by").unwrap(), "http://creativecommons.org/licenses/by/4.0/");
        assert_eq!(license_url("cc_by_nc_nd").unwrap(), "http://creativecommons.org/licenses/by-nc-nd/4.0/");
        assert_eq!(license_url("cc0").unwrap(), "http://creativecommons.org/publicdomain/zero/1.0/");
        assert_eq!(license_url(""), None);
    }

    #[test]
    fn authors_split_on_semicolons() {
        let meta = meta_from_api(
            Source::Biorxiv,
            "10.1101/2024.06.28.601293",
            ApiPaper {
                title: "T".into(),
                authors: "Chen, Y.; Abram, S. J.; Ivry, R. B.".into(),
                date: "2024-07-01".into(),
                version: "1".into(),
                license: "cc_by_nc_nd".into(),
                category: "neuroscience".into(),
                r#abstract: "A".into(),
                jatsxml: String::new(),
            },
        )
        .unwrap();
        assert_eq!(meta.version, 1);
        assert_eq!(meta.authors, vec!["Chen, Y.", "Abram, S. J.", "Ivry, R. B."]);
        assert_eq!(meta.categories, vec!["neuroscience"]);
        assert_eq!(meta.source, Source::Biorxiv);
        assert!(meta.published.is_some());
    }
}
