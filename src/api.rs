use crate::auth;
use crate::config::{Profile, Site};
use crate::http::{self, Problem};
use crate::store::{self, Tokens};
use anyhow::{Context, Result, bail};
use serde_json::Value;

pub const API_VERSION: u32 = 1;
const RELEASES: &str = "https://github.com/mah3uz/nibble-cli/releases";

pub struct Session {
    pub site: Site,
    pub origin: String,
    pub label: String,
    profile: Option<Profile>,
    tokens: Tokens,
}

impl Session {
    pub fn for_profile(profile: Profile) -> Result<Self> {
        let tokens = store::load(&profile)?;
        Ok(Session { site: profile.site.clone(), origin: profile.origin.clone(), label: profile.label(), profile: Some(profile), tokens })
    }

    pub fn for_token(origin: &str, token: String) -> Result<Self> {
        let origin = http::origin(origin)?;
        let server = auth::discover(&origin)?;
        let site = Site {
            name: crate::config::host(&origin),
            issuer: server.issuer,
            api: server.api,
            token_endpoint: server.token_endpoint,
            revocation_endpoint: server.revocation_endpoint,
            accounts: Default::default(),
        };
        let label = format!("{} with NIBBLE_TOKEN", crate::config::host(&origin));
        Ok(Session {
            site,
            origin,
            label,
            profile: None,
            tokens: Tokens { access_token: token, refresh_token: None, expires_at: i64::MAX },
        })
    }

    pub fn with_tokens(site: Site, origin: String, tokens: Tokens) -> Self {
        let label = crate::config::host(&origin);
        Session { site, origin, label, profile: None, tokens }
    }

    fn refresh(&mut self) -> Result<()> {
        let (Some(profile), Some(refresh_token)) = (&self.profile, self.tokens.refresh_token.clone()) else {
            bail!("the token was refused; it may have expired or been revoked");
        };
        let tokens = auth::refresh(&self.site, &refresh_token).map_err(|error| match error.downcast_ref::<Problem>() {
            Some(problem) if problem.code == "invalid_grant" => {
                anyhow::anyhow!(
                    "you've been signed out of {} ({}); run `nibble auth login {}`",
                    profile.label(),
                    problem.detail,
                    profile.origin
                )
            }
            _ => error,
        })?;
        store::save(&profile.origin, &profile.account.email, profile.account.storage, &tokens)?;
        self.tokens = tokens;
        Ok(())
    }

    pub fn call(&mut self, operation: &str, input: &Value) -> Result<Value> {
        let url = format!("{}/operations/{}", self.site.api, operation);
        self.send(|client, token| client.post(&url).bearer_auth(token).json(input))
    }

    pub fn catalogue(&mut self) -> Result<Value> {
        let url = format!("{}/operations", self.site.api);
        self.send(|client, token| client.get(&url).bearer_auth(token))
    }

    fn send(&mut self, build: impl Fn(&reqwest::blocking::Client, &str) -> reqwest::blocking::RequestBuilder) -> Result<Value> {
        if !self.tokens.fresh() {
            self.refresh()?;
        }
        let client = http::client();
        let mut response = build(&client, &self.tokens.access_token).send().with_context(|| format!("couldn't reach {}", self.origin))?;
        if response.status().as_u16() == 401 && self.tokens.refresh_token.is_some() {
            self.refresh()?;
            response = build(&client, &self.tokens.access_token).send()?;
        }
        let status = response.status().as_u16();
        let version = response.headers().get("nibble-api-version").and_then(|value| value.to_str().ok());
        if version.is_some() || (200..300).contains(&status) {
            check_version(&self.origin, version)?;
        }
        if status == 404 && response.headers().get("content-type").is_none_or(|kind| !kind.to_str().unwrap_or("").contains("json")) {
            bail!("{} answered 404: its administrator may have turned Agent access off", self.origin);
        }
        let body: Value = response.json().unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            return Err(Problem::from_body(status, &body).into());
        }
        Ok(body)
    }
}

// Only the side that is behind can fix a mismatch, so the message names it.
fn check_version(origin: &str, header: Option<&str>) -> Result<()> {
    match header.and_then(|value| value.trim().parse::<u32>().ok()) {
        Some(API_VERSION) => Ok(()),
        Some(site) if site > API_VERSION => bail!(
            "{origin} speaks version {site} of Nibble's management API, and this nibble only version {API_VERSION}; \
             install the latest nibble from {RELEASES}"
        ),
        _ => bail!(
            "{origin} runs a Nibble older than this nibble supports (it needs management API version {API_VERSION}); \
             ask the site's administrator to run `bin/rails nibble:upgrade`, or use an older nibble"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_on_the_same_contract_is_used() {
        assert!(check_version("https://example.com", Some(&API_VERSION.to_string())).is_ok());
    }

    #[test]
    fn a_newer_site_asks_for_a_newer_cli() {
        let error = check_version("https://example.com", Some(&(API_VERSION + 1).to_string())).unwrap_err().to_string();
        assert!(error.contains("install the latest nibble"), "{error}");
    }

    #[test]
    fn an_older_or_unversioned_site_asks_for_a_site_upgrade() {
        for header in [Some("0"), Some("2026-09-27"), None] {
            let error = check_version("https://example.com", header).unwrap_err().to_string();
            assert!(error.contains("nibble:upgrade"), "{header:?}: {error}");
        }
    }
}
