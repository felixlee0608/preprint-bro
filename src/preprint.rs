//! Which preprint server a paper comes from, and the ids they share.
//!
//! arXiv papers keep arXiv's own id (`1706.03762`, `hep-th/9711200`).
//! bioRxiv and medRxiv papers are identified by their DOI. Both servers
//! mint `10.1101/…` DOIs, so a bare DOI cannot tell them apart by itself:
//! it is tried on bioRxiv first and then medRxiv, while a link that names
//! the host decides immediately. The resolved source travels with the
//! metadata (`Meta.source`), so the rest of the code never has to guess.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::arxiv::PaperId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Arxiv,
    Biorxiv,
    Medrxiv,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Arxiv => "arxiv",
            Source::Biorxiv => "biorxiv",
            Source::Medrxiv => "medrxiv",
        }
    }

    /// The name a reader sees, in the server's own casing.
    pub fn label(self) -> &'static str {
        match self {
            Source::Arxiv => "arXiv",
            Source::Biorxiv => "bioRxiv",
            Source::Medrxiv => "medRxiv",
        }
    }

    /// The host of the paper's own pages and figures.
    pub fn host(self) -> &'static str {
        match self {
            Source::Arxiv => "arxiv.org",
            Source::Biorxiv => "www.biorxiv.org",
            Source::Medrxiv => "www.medrxiv.org",
        }
    }

    /// The alias for a paper's course: `arxiv-1706-03762`,
    /// `biorxiv-10-1101-2024-06-28-601293`.
    pub fn alias(self, id: &str) -> String {
        format!("{}-{}", self.as_str(), id.to_ascii_lowercase().replace(['/', '.'], "-"))
    }

    /// Where a reader sees the paper itself.
    pub fn abs_url(self, id: &str) -> String {
        match self {
            Source::Arxiv => format!("https://arxiv.org/abs/{id}"),
            Source::Biorxiv | Source::Medrxiv => format!("https://{}/content/{id}", self.host()),
        }
    }

    /// The `server` name api.biorxiv.org expects; None for arXiv.
    pub fn api_server(self) -> Option<&'static str> {
        match self {
            Source::Arxiv => None,
            Source::Biorxiv => Some("biorxiv"),
            Source::Medrxiv => Some("medrxiv"),
        }
    }

    /// From the persisted `source` column, tolerant of anything.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "arxiv" => Some(Source::Arxiv),
            "biorxiv" => Some(Source::Biorxiv),
            "medrxiv" => Some(Source::Medrxiv),
            _ => None,
        }
    }
}

/// A canonical preprint: no version suffix, no URL. For bioRxiv/medRxiv the
/// id is the DOI without its version (`10.1101/2024.06.28.601293`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperRef {
    pub source: Source,
    pub id: String,
    pub version: Option<u32>,
}

static DOI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"10\.1101/[A-Za-z0-9][A-Za-z0-9._-]*").unwrap());

/// The known suffixes a biorxiv.org/medrxiv.org content URL can end in.
const CONTENT_SUFFIXES: &[&str] = &[".full", ".pdf", ".source"];

/// Accepts a bare DOI, a doi.org link, or a biorxiv.org / medrxiv.org
/// content link. A link that names the host pins the source; a bare DOI is
/// taken as bioRxiv and the metadata fetch falls back to medRxiv (see bio.rs).
pub fn parse_ref(input: &str) -> Option<PaperRef> {
    let mut s = input.trim();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest;
        }
    }
    let lower = s.to_ascii_lowercase();

    // arXiv host or an arXiv-style id: the original path.
    if ["arxiv.org/", "www.arxiv.org/", "export.arxiv.org/", "aero.oops.wtf/paper-bro/"]
        .iter()
        .any(|h| lower.starts_with(h))
    {
        if let Some(p) = PaperId::parse(s) {
            return Some(PaperRef { source: Source::Arxiv, id: p.id, version: p.version });
        }
        return None;
    }
    if let Some(p) = PaperId::parse(s) {
        return Some(PaperRef { source: Source::Arxiv, id: p.id, version: p.version });
    }

    // bioRxiv / medRxiv content links, e.g.
    // https://www.biorxiv.org/content/10.1101/2024.06.28.601293v1.full
    let (source, rest) = if lower.starts_with("www.medrxiv.org/") || lower.starts_with("medrxiv.org/") {
        (Source::Medrxiv, &s[s.find('/')?..])
    } else if lower.starts_with("www.biorxiv.org/") || lower.starts_with("biorxiv.org/") {
        (Source::Biorxiv, &s[s.find('/')?..])
    } else if lower.starts_with("doi.org/") {
        (Source::Biorxiv, s)
    } else {
        // A bare DOI, or nothing useful.
        (Source::Biorxiv, s)
    };
    let tail = rest.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    // A bare DOI has to be the whole input: `example.com/content/10.1101/1`
    // is rejected rather than mined for its DOI. A link that names a known
    // host may carry the DOI anywhere in its path.
    let bare = !(lower.starts_with("doi.org/")
        || lower.starts_with("www.biorxiv.org/")
        || lower.starts_with("biorxiv.org/")
        || lower.starts_with("www.medrxiv.org/")
        || lower.starts_with("medrxiv.org/"));
    let m = if bare {
        let end = tail.len();
        DOI.find(tail).filter(|m| m.start() == 0 && m.end() == end)?
    } else {
        DOI.find(tail)?
    };
    let mut id = m.as_str().to_string();
    for suffix in CONTENT_SUFFIXES {
        if let Some(stripped) = id.strip_suffix(suffix) {
            id = stripped.to_string();
            break;
        }
    }
    // "…601293v1" → id "…601293", version 1. Only when the part after the
    // last `v` is a number, so an id that contains `v` otherwise survives.
    let version = match id.rsplit_once('v') {
        Some((base, v)) if !base.is_empty() && v.parse::<u32>().is_ok() => {
            let n = v.parse::<u32>().ok();
            id = base.to_string();
            n
        }
        _ => None,
    };
    Some(PaperRef { source, id, version })
}

impl PaperRef {
    /// The metadata servers to try, in order. A bare DOI (source Biorxiv)
    /// could live on either server, so bioRxiv is tried first and medRxiv
    /// second; a link that named the host tries just that one.
    pub fn servers(&self) -> Vec<Source> {
        match self.source {
            Source::Biorxiv => vec![Source::Biorxiv, Source::Medrxiv],
            s => vec![s],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Option<(Source, String, Option<u32>)> {
        parse_ref(s).map(|p| (p.source, p.id, p.version))
    }

    #[test]
    fn arxiv_ids_still_resolve() {
        assert_eq!(r("1706.03762"), Some((Source::Arxiv, "1706.03762".into(), None)));
        assert_eq!(r("https://arxiv.org/abs/1706.03762v7"), Some((Source::Arxiv, "1706.03762".into(), Some(7))));
        assert_eq!(r("hep-th/9711200"), Some((Source::Arxiv, "hep-th/9711200".into(), None)));
    }

    #[test]
    fn biorxiv_links_in_every_form() {
        let want = Some((Source::Biorxiv, "10.1101/2024.06.28.601293".into(), Some(1)));
        for s in [
            "10.1101/2024.06.28.601293v1",
            "https://www.biorxiv.org/content/10.1101/2024.06.28.601293v1",
            "https://www.biorxiv.org/content/10.1101/2024.06.28.601293v1.full",
            "http://biorxiv.org/content/10.1101/2024.06.28.601293v1?context=all",
        ] {
            assert_eq!(r(s), want, "{s}");
        }
        assert_eq!(r("https://doi.org/10.1101/2024.06.28.601293"), Some((Source::Biorxiv, "10.1101/2024.06.28.601293".into(), None)));
        assert_eq!(r("10.1101/2024.06.28.601293"), Some((Source::Biorxiv, "10.1101/2024.06.28.601293".into(), None)));
    }

    #[test]
    fn medrxiv_links_pin_the_medrxiv_source() {
        assert_eq!(
            r("https://www.medrxiv.org/content/10.1101/2024.06.28.601293v1.full"),
            Some((Source::Medrxiv, "10.1101/2024.06.28.601293".into(), Some(1)))
        );
        // A bare DOI defaults to bioRxiv; medRxiv is probed at fetch time.
        assert_eq!(r("10.1101/2024.06.28.601293").map(|x| x.0), Some(Source::Biorxiv));
    }

    #[test]
    fn rejects_things_that_are_not_preprints() {
        for s in ["", "hello", "10.1101/../../etc/passwd", "https://example.com/content/10.1101/1", "10.1101/"] {
            assert_eq!(parse_ref(s), None, "{s}");
        }
    }

    #[test]
    fn aliases_fit_urls_and_are_unique() {
        assert_eq!(Source::Arxiv.alias("1706.03762"), "arxiv-1706-03762");
        assert_eq!(Source::Biorxiv.alias("10.1101/2024.06.28.601293"), "biorxiv-10-1101-2024-06-28-601293");
        assert_eq!(Source::Medrxiv.alias("10.1101/2024.06.28.601293"), "medrxiv-10-1101-2024-06-28-601293");
        assert!(Source::Biorxiv.alias("10.1101/2024.06.28.601293").len() < 63);
    }

    #[test]
    fn abs_urls_go_to_the_right_host() {
        assert_eq!(Source::Arxiv.abs_url("1706.03762"), "https://arxiv.org/abs/1706.03762");
        assert_eq!(
            Source::Biorxiv.abs_url("10.1101/2024.06.28.601293"),
            "https://www.biorxiv.org/content/10.1101/2024.06.28.601293"
        );
        assert_eq!(
            Source::Medrxiv.abs_url("10.1101/2024.06.28.601293"),
            "https://www.medrxiv.org/content/10.1101/2024.06.28.601293"
        );
    }
}
