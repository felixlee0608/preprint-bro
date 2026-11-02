//! bioRxiv and medRxiv: metadata from api.biorxiv.org, full text as JATS XML.
//!
//! The metadata API is open and lightweight, and serves both servers (the
//! `server` path element picks which). Full text and figures live on
//! www.biorxiv.org / www.medrxiv.org behind Cloudflare, which rate-limits by
//! IP; a self-hosted builder fetches each paper once per build, comfortably
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
        let http = reqwest::Client::builder()
            .user_agent("preprint-bro/0.2 (self-hosted preprint course builder)")
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
    pub async fn full_xml(&self, url: &str) -> Result<Option<String>> {
        let resp = self.http.get(url).send().await.with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
            bail!(
                "{host} answered {status} — its CDN is rate-limiting this IP \
                 (preprint servers throttle aggressively; a few minutes between builds usually clears it). \
                 Retry the build a little later."
            );
        }
        let text = resp.error_for_status().with_context(|| format!("GET {url}"))?.text().await?;
        Ok(Some(text))
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
