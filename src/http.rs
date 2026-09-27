use anyhow::{Result, bail};
use reqwest::blocking::Client;
use serde_json::Value;
use std::fmt;
use std::time::Duration;
use url::Url;

const LOCAL: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

pub fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("nibble-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .expect("an HTTP client")
}

pub fn origin(input: &str) -> Result<String> {
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let url = Url::parse(&with_scheme)?;
    let host = url.host_str().unwrap_or_default();
    match url.scheme() {
        "https" => {}
        "http" if LOCAL.contains(&host) => {}
        _ => bail!("{input} must be https (plain http only for this computer)"),
    }
    Ok(url.origin().ascii_serialization())
}

pub fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

#[derive(Debug)]
pub struct Problem {
    pub status: u16,
    pub code: String,
    pub detail: String,
    pub hint: Option<String>,
    pub details: Option<Value>,
}

impl Problem {
    pub fn from_body(status: u16, body: &Value) -> Self {
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_string);
        Problem {
            status,
            code: text("code").or_else(|| text("error")).unwrap_or_else(|| "error".into()),
            detail: text("detail").or_else(|| text("error_description")).unwrap_or_else(|| format!("the site answered {status}")),
            hint: text("hint"),
            details: body.get("details").cloned(),
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({ "code": self.code, "message": self.detail, "hint": self.hint, "details": self.details, "status": self.status })
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Problem {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_only_travel_over_https_except_to_this_computer() {
        assert_eq!(origin("example.com").unwrap(), "https://example.com");
        assert_eq!(origin("https://example.com/cp/anything").unwrap(), "https://example.com");
        assert_eq!(origin("http://localhost:3000").unwrap(), "http://localhost:3000");
        assert!(origin("http://example.com").is_err(), "plain http would send a bearer token in the clear");
        assert!(origin("ftp://example.com").is_err());
    }

    #[test]
    fn same_origin_means_scheme_host_and_port() {
        assert!(same_origin("https://a.test/oauth/token", "https://a.test"));
        assert!(!same_origin("https://a.test.evil/oauth", "https://a.test"));
        assert!(!same_origin("http://a.test", "https://a.test"));
    }
}
