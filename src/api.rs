use crate::auth;
use crate::config::{Profile, Site};
use crate::http::{self, Problem};
use crate::store::{self, Tokens};
use anyhow::{Context, Result, bail};
use serde_json::Value;

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
        let url = format!("{}/{}", self.site.api, operation);
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
