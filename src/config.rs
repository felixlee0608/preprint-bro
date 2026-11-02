//! Configuration, read once from the environment at boot.
//!
//! Every knob has a default except the credentials and the model API's URL and
//! name. `.env.example` lists them all with their defaults.
//!
//! Model endpoints: the old single-provider variables (`LLM_BASE_URL`,
//! `LLM_MODEL`, …) still work and name the provider `default`. For several
//! providers, set `LLM_<NAME>_BASE_URL` (with optional
//! `LLM_<NAME>_MODEL/_API_KEY/_FALLBACK_MODEL/_REASONING/_JSON_MODE/
//! _PRICE_INPUT/_PRICE_CACHED/_PRICE_OUTPUT`) and pick one with
//! `LLM_DEFAULT=<name>`. Each build may also name a provider explicitly.

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;

/// The path prefix preprint-bro is served under. The reverse proxy routes
/// `/preprint-bro*` here without stripping it, so every route and asset URL
/// carries it.
pub const BASE: &str = "/preprint-bro";

/// How the model is asked for strict JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonMode {
    /// `response_format: {type: "json_schema", json_schema: {strict: true}}`.
    Schema,
    /// `response_format: {type: "json_object"}`.
    Object,
    /// No response_format; the prompt asks for JSON and the caller strips
    /// any markdown fences from the answer.
    Text,
}

impl JsonMode {
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "schema" => Ok(Self::Schema),
            "object" => Ok(Self::Object),
            "text" | "none" | "" => Ok(Self::Text),
            other => bail!("unknown JSON mode {other:?} (expected schema, object or text)"),
        }
    }
}

/// The OpenAI-compatible chat-completions endpoint a build calls.
#[derive(Clone)]
pub struct Endpoint {
    /// The endpoint's name: `default`, or the `LLM_<NAME>_*` name.
    pub name: String,
    /// The full chat-completions URL.
    pub url: String,
    /// Sent as a bearer token. Empty for a server that wants none.
    pub key: String,
    pub model: String,
    /// Used if `model` stops existing (HTTP 404).
    pub fallback_model: String,
    /// Whether the server understands `reasoning_effort`. Ollama and most
    /// local servers reject unknown fields, so this is off unless the
    /// provider says it is on.
    pub reasoning: bool,
    /// How strict JSON output is requested.
    pub json_mode: JsonMode,
    /// USD per million tokens, for the daily budget and the cost note.
    pub price_input: f64,
    pub price_cached: f64,
    pub price_output: f64,
}

// Never print the key (or a token in the URL's query), whatever logs this.
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("name", &self.name)
            .field("url", &self.url.split('?').next().unwrap_or_default())
            .field("key", &if self.key.is_empty() { "" } else { "<redacted>" })
            .field("model", &self.model)
            .field("fallback_model", &self.fallback_model)
            .field("reasoning", &self.reasoning)
            .field("json_mode", &self.json_mode)
            .field("price_input", &self.price_input)
            .field("price_cached", &self.price_cached)
            .field("price_output", &self.price_output)
            .finish()
    }
}

impl Endpoint {
    /// What a command that makes no model calls carries in place of one.
    fn unused() -> Self {
        Self {
            name: "(not used)".into(),
            url: String::new(),
            key: String::new(),
            model: String::new(),
            fallback_model: String::new(),
            reasoning: false,
            json_mode: JsonMode::Text,
            price_input: 1.25,
            price_cached: 0.15,
            price_output: 4.25,
        }
    }

    /// One OpenAI-compatible server: `LLM_BASE_URL` (up to and including
    fn from_vars_for(name: &str, get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        // Env keys are conventionally uppercase: LLM_DEEPSEEK_BASE_URL etc.
        let prefix = if name == "default" { "LLM_".to_string() } else { format!("LLM_{}_", name.to_ascii_uppercase()) };
        let prefixed = |key: &str| get(&format!("{prefix}{key}"));
        let need = |key: &str| -> anyhow::Result<String> {
            prefixed(key).with_context(|| format!("{prefix}{key} is not set"))
        };
        let base = need("BASE_URL")?;
        let parsed = reqwest::Url::parse(&base)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            .with_context(|| format!("{prefix}BASE_URL={base} isn't a URL (expected https://host/v1)"))?;
        let model = need("MODEL")?;
        let mut url = parsed.clone();
        url.set_fragment(None);
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("{prefix}BASE_URL={base} can't carry a path"))?
            .pop_if_empty()
            .extend(["chat", "completions"]);
        let fallback_model = prefixed("FALLBACK_MODEL").unwrap_or_else(|| model.clone());
        let reasoning = prefixed("REASONING")
            .map(|v| v.parse::<u8>())
            .transpose()
            .map_err(|e| anyhow::anyhow!("{prefix}REASONING: {e}"))?
            .unwrap_or(1)
            == 1;
        let json_mode = JsonMode::from_str(&prefixed("JSON_MODE").unwrap_or_else(|| "schema".into()))?;
        let price = |key: &str, default: f64| -> Result<f64> {
            match prefixed(key) {
                None => Ok(default),
                Some(v) => v
                    .parse()
                    .map_err(|e| anyhow::anyhow!("{prefix}{key}={v}: {e}")),
            }
        };
        Ok(Self {
            name: name.to_string(),
            url: url.to_string(),
            key: prefixed("API_KEY").unwrap_or_default(),
            fallback_model,
            model,
            reasoning,
            json_mode,
            price_input: price("PRICE_INPUT", 1.25)?,
            price_cached: price("PRICE_CACHED", 0.15)?,
            price_output: price("PRICE_OUTPUT", 4.25)?,
        })
    }

    /// True when the key would cross a network unencrypted: one is set and the
    /// address is plain http to a host other than this machine.
    pub fn key_in_clear(&self) -> bool {
        let Ok(u) = reqwest::Url::parse(&self.url) else { return false };
        let local = u.host_str().is_some_and(|h| {
            h == "localhost" || h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        });
        u.scheme() == "http" && !self.key.is_empty() && !local
    }
}

/// Every provider configured in the environment, at least one. The legacy
/// `LLM_BASE_URL` variables make the provider named `default`.
pub fn providers_from_env() -> Result<Vec<Endpoint>> {
    let mut names: Vec<String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("LLM_") && k.ends_with("_BASE_URL"))
        .filter_map(|(k, _)| {
            // "LLM_BASE_URL" strips to just "BASE_URL", which doesn't end in
            // "_BASE_URL" again; unwrap_or_default turns that into "" → default.
            let rest = k.strip_prefix("LLM_")?;
            let name = rest.strip_suffix("_BASE_URL").unwrap_or_default();
            Some(if name.is_empty() { "default".to_string() } else { name.to_ascii_lowercase() })
        })
        .collect();
    names.sort();
    names.dedup();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        let ep = Endpoint::from_vars_for(&name, var)?;
        if ep.key_in_clear() {
            log::warn!(
                "{name}: LLM_BASE_URL is plain http to {}: LLM_API_KEY is sent unencrypted. Use https unless that network is one you trust.",
                ep.name
            );
        }
        out.push(ep);
    }
    if out.is_empty() {
        bail!(
            "no LLM provider is configured. Set LLM_BASE_URL/LLM_MODEL (one provider) or \
             LLM_<NAME>_BASE_URL with LLM_DEFAULT=<name> (several). See .env.example."
        );
    }
    Ok(out)
}

// No Debug: it holds the database URL and the AERO token.
#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    /// Every configured model endpoint.
    pub providers: Vec<Endpoint>,
    /// The provider a build uses unless the build names one.
    /// Which provider a build uses when a request doesn't name one.
    #[allow(dead_code)]
    pub default_provider: String,
    /// The default provider, kept for the code paths that need one endpoint.
    pub endpoint: Endpoint,
    pub aero_token: String,
    pub aero_mcp_url: String,
    /// Origin the published courses and preprint-bro itself are served from.
    pub public_origin: String,
    /// When false, a finished course is kept in the database and served from
    /// `/preprint-bro/preview/{alias}` instead of being published to AERO.
    pub publish: bool,
    /// A Google Analytics measurement id (e.g. `G-XXXX`), injected into the
    /// app's pages. Empty in self-hosted installs, so nothing is sent anywhere.
    pub analytics_id: String,
    /// Where the app's static files are fetched from first, each falling back
    /// to this server. Empty means serve everything from this server.
    pub asset_cdn: String,
    pub chrome: PathBuf,
    pub listen: SocketAddr,
    pub admin_listen: SocketAddr,
    pub workers: usize,
    pub builds_per_ip_per_day: u32,
    pub builds_per_day: u32,
    pub max_queued: i64,
    pub daily_budget_usd: f64,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn required(name: &str) -> Result<String> {
    var(name).with_context(|| format!("{name} is not set"))
}

fn parsed<T: std::str::FromStr>(name: &str, default: T) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    match var(name) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|e| anyhow::anyhow!("{name}={v}: {e}")),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_env_for(true)
    }

    /// `needs_model` is false for a command that never calls the model, which
    /// then runs without the `LLM_*` settings.
    pub fn from_env_for(needs_model: bool) -> Result<Self> {
        let chrome = PathBuf::from(var("CHROME_PATH").unwrap_or_else(|| {
            ".chrome/chrome-headless-shell-mac-arm64/chrome-headless-shell".into()
        }));
        if !chrome.exists() {
            bail!("CHROME_PATH {} does not exist", chrome.display());
        }
        let providers = if needs_model {
            providers_from_env()?
        } else {
            // A command that makes no model calls still needs a provider map
            // to exist, so `Llm` can be constructed; an empty one is fine.
            Vec::new()
        };
        let default_provider = var("LLM_DEFAULT").unwrap_or_else(|| "default".into());
        let publish = parsed("PUBLISH", 0u8)? == 1;
        let endpoint = if providers.is_empty() {
            Endpoint::unused()
        } else {
            providers
                .iter()
                .find(|p| p.name == default_provider)
                .cloned()
                .with_context(|| format!("LLM_DEFAULT names {default_provider:?}, but no LLM_{}_BASE_URL is set", default_provider.to_uppercase()))?
        };
        Ok(Self {
            database_url: required("NEON_DB")?,
            providers,
            default_provider,
            endpoint,
            // Publishing needs an AERO token; a self-hosted install that only
            // previews locally (PUBLISH=0) doesn't have one and doesn't need it.
            aero_token: if publish { required("AERO_TOKEN")? } else { var("AERO_TOKEN").unwrap_or_default() },
            aero_mcp_url: var("AERO_MCP_URL").unwrap_or_else(|| "https://aero.oops.wtf/mcp".into()),
            public_origin: var("PUBLIC_ORIGIN").unwrap_or_else(|| "https://aero.oops.wtf".into()),
            publish,
            analytics_id: var("ANALYTICS_ID").unwrap_or_default(),
            asset_cdn: var("ASSET_CDN").unwrap_or_default(),
            chrome,
            listen: parsed("LISTEN", "127.0.0.1:8083".parse()?)?,
            admin_listen: parsed("ADMIN_LISTEN", "127.0.0.1:8085".parse()?)?,
            workers: parsed("WORKERS", 2)?,
            builds_per_ip_per_day: parsed("BUILDS_PER_IP_PER_DAY", 3)?,
            builds_per_day: parsed("BUILDS_PER_DAY", 40)?,
            max_queued: parsed("MAX_QUEUED", 20)?,
            daily_budget_usd: parsed("DAILY_BUDGET_USD", 50.0)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| map.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone())
    }

    fn legacy(pairs: &[(&str, &str)]) -> Endpoint {
        Endpoint::from_vars_for("default", vars(pairs)).unwrap()
    }

    #[test]
    fn any_openai_compatible_server_by_url_model_and_key() {
        let e = legacy(&[("LLM_BASE_URL", "https://ep.example/v1/"), ("LLM_MODEL", "org/model"), ("LLM_API_KEY", "id.secret")]);
        assert_eq!((e.name.as_str(), e.url.as_str(), e.key.as_str()), ("default", "https://ep.example/v1/chat/completions", "id.secret"));
        assert_eq!((e.model.as_str(), e.fallback_model.as_str()), ("org/model", "org/model"));
        // The legacy variables name the provider `default`.
        assert_eq!(e.json_mode, JsonMode::Schema);
        assert!(e.reasoning);
        assert_eq!((e.price_input, e.price_cached, e.price_output), (1.25, 0.15, 4.25));
        // A local server wants no key, and may name a fallback model.
        let e = legacy(&[("LLM_BASE_URL", "http://localhost:11434/v1"), ("LLM_MODEL", "a"), ("LLM_FALLBACK_MODEL", "b")]);
        assert_eq!((e.name.as_str(), e.url.as_str(), e.key.as_str(), e.fallback_model.as_str()), ("default", "http://localhost:11434/v1/chat/completions", "", "b"));
    }

    #[test]
    fn named_providers_are_read_and_default_to_schema() {
        let get = vars(&[
            ("LLM_DEEPSEEK_BASE_URL", "https://api.deepseek.com/v1"),
            ("LLM_DEEPSEEK_MODEL", "deepseek-chat"),
            ("LLM_DEEPSEEK_API_KEY", "sk-ds"),
            ("LLM_OLLAMA_BASE_URL", "http://localhost:11434/v1"),
            ("LLM_OLLAMA_MODEL", "qwen2.5:7b"),
            ("LLM_OLLAMA_JSON_MODE", "object"),
            ("LLM_OLLAMA_REASONING", "0"),
            ("LLM_OLLAMA_PRICE_INPUT", "0"),
        ]);
        let e = Endpoint::from_vars_for("deepseek", &get).unwrap();
        assert_eq!((e.name.as_str(), e.url.as_str(), e.model.as_str()), ("deepseek", "https://api.deepseek.com/v1/chat/completions", "deepseek-chat"));
        assert_eq!(e.json_mode, JsonMode::Schema);
        let e = Endpoint::from_vars_for("ollama", &get).unwrap();
        assert!(!e.reasoning);
        assert_eq!(e.json_mode, JsonMode::Object);
        assert_eq!(e.price_input, 0.0);
        assert_eq!(e.key, "");
    }

    #[test]
    fn the_path_is_added_before_any_query() {
        let url = |base: &str| legacy(&[("LLM_BASE_URL", base), ("LLM_MODEL", "m")]).url;
        assert_eq!(url("https://ep.example/v1?api-version=1"), "https://ep.example/v1/chat/completions?api-version=1");
        assert_eq!(url("https://ep.example/v1/?api-version=1"), "https://ep.example/v1/chat/completions?api-version=1");
        assert_eq!(url("https://ep.example"), "https://ep.example/chat/completions");
        assert_eq!(url("https://ep.example/v1#notes"), "https://ep.example/v1/chat/completions");
    }

    #[test]
    fn the_url_and_the_model_are_required() {
        let err = Endpoint::from_vars_for("default", vars(&[("LLM_MODEL", "m")])).unwrap_err().to_string();
        assert!(err.contains("LLM_BASE_URL"), "{err}");
        let err = Endpoint::from_vars_for("default", vars(&[("LLM_BASE_URL", "https://ep.example/v1")])).unwrap_err().to_string();
        assert!(err.contains("LLM_MODEL"), "{err}");
    }

    #[test]
    fn bare_llm_vars_name_the_default_provider() {
        // providers_from_env scans the real environment: the legacy LLM_* vars
        // must come out as one provider named `default` (not an empty name).
        unsafe {
            std::env::set_var("LLM_BASE_URL", "https://api.deepseek.com/v1");
            std::env::set_var("LLM_MODEL", "deepseek-flash");
            std::env::set_var("LLM_API_KEY", "sk-test");
            std::env::set_var("LLM_OLLAMA_BASE_URL", "http://localhost:11434/v1");
            std::env::set_var("LLM_OLLAMA_MODEL", "qwen2.5:7b");
        }
        let providers = providers_from_env().unwrap();
        unsafe {
            std::env::remove_var("LLM_BASE_URL");
            std::env::remove_var("LLM_MODEL");
            std::env::remove_var("LLM_API_KEY");
            std::env::remove_var("LLM_OLLAMA_BASE_URL");
            std::env::remove_var("LLM_OLLAMA_MODEL");
        }
        let names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["default", "ollama"], "bare LLM_* vars must be the `default` provider");
        assert_eq!(providers[0].url, "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(providers[0].model, "deepseek-flash");
    }

    #[test]
    fn a_key_meant_for_another_service_is_never_picked_up() {
        // Only LLM_API_KEY is read, so one left in the environment for something
        // else is not sent to this server.
        let e = legacy(&[("LLM_BASE_URL", "https://ep.example/v1"), ("LLM_MODEL", "m"), ("OPENAI_API_KEY", "other")]);
        assert_eq!(e.key, "");
    }

    #[test]
    fn a_key_over_plain_http_to_another_machine_is_noticed() {
        let e = |url: &str, key: &str| Endpoint { name: "h".into(), url: url.into(), key: key.into(), model: "m".into(), fallback_model: "m".into(), reasoning: false, json_mode: JsonMode::Schema, price_input: 1.25, price_cached: 0.15, price_output: 4.25 };
        assert!(e("http://llm.internal:8000/v1/chat/completions", "k").key_in_clear());
        assert!(!e("https://llm.example/v1/chat/completions", "k").key_in_clear());
        assert!(!e("http://llm.internal:8000/v1/chat/completions", "").key_in_clear());
        for local in ["http://localhost:11434/v1/chat/completions", "http://127.0.0.1:8000/v1/chat/completions", "http://[::1]:8000/v1/chat/completions"] {
            assert!(!e(local, "k").key_in_clear(), "{local}");
        }
    }

    #[test]
    fn debug_output_never_shows_the_key_or_a_token_in_the_url() {
        let e = Endpoint { name: "h".into(), url: "https://h/v1/chat/completions?token=hunter2".into(), key: "sk-secret".into(), model: "m".into(), fallback_model: "m".into(), reasoning: false, json_mode: JsonMode::Schema, price_input: 1.25, price_cached: 0.15, price_output: 4.25 };
        let shown = format!("{e:?}");
        assert!(!shown.contains("sk-secret") && !shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("<redacted>") && shown.contains("https://h/v1/chat/completions"), "{shown}");
    }

    #[test]
    fn a_base_url_that_is_not_a_url_stops_the_boot() {
        for bad in ["ep.example/v1", "ftp://ep.example/v1", "https://"] {
            let err = Endpoint::from_vars_for("default", vars(&[("LLM_BASE_URL", bad), ("LLM_MODEL", "m")])).unwrap_err().to_string();
            assert!(err.contains("LLM_BASE_URL"), "{bad}: {err}");
        }
    }

    #[test]
    fn an_unknown_json_mode_and_bad_price_stop_the_boot() {
        let err = Endpoint::from_vars_for("x", vars(&[("LLM_X_BASE_URL", "https://e/v1"), ("LLM_X_MODEL", "m"), ("LLM_X_JSON_MODE", "banana")])).unwrap_err().to_string();
        assert!(err.contains("banana"), "{err}");
        let err = Endpoint::from_vars_for("x", vars(&[("LLM_X_BASE_URL", "https://e/v1"), ("LLM_X_MODEL", "m"), ("LLM_X_PRICE_INPUT", "cheap")])).unwrap_err().to_string();
        assert!(err.contains("PRICE_INPUT"), "{err}");
    }
}
