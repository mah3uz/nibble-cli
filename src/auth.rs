use crate::http::{self, Problem};
use crate::store::{Tokens, now};
use crate::style::{CHANGE, DIM, HEADING, LINK, paint};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;
use url::Url;

pub const CLIENT_ID: &str = "nibble-cli";
const WAIT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct Server {
    pub issuer: String,
    pub api: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub revocation_endpoint: Option<String>,
    pub device_endpoint: Option<String>,
}

fn get_json(url: &str) -> Result<Value> {
    let response = http::client().get(url).send().with_context(|| format!("couldn't reach {url}"))?;
    let status = response.status();
    if status.as_u16() == 404 {
        bail!("{url} isn't there: the site may not run Nibble, or its administrator hasn't turned on Agent access");
    }
    if !status.is_success() {
        bail!("{url} answered {status}");
    }
    response.json().with_context(|| format!("{url} didn't answer with JSON"))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn discover(origin: &str) -> Result<Server> {
    let resource = get_json(&format!("{origin}/.well-known/oauth-protected-resource/api/v1"))?;
    let api = text(&resource, "resource").context("the site didn't name its API")?;
    let issuer = resource["authorization_servers"][0].as_str().context("the site didn't name who signs you in")?.to_string();
    if !http::same_origin(&api, origin) || !http::same_origin(&issuer, origin) {
        bail!("{origin} calls itself {}; sign in with that address", http::origin(&issuer).unwrap_or(issuer));
    }

    let metadata = get_json(&format!("{issuer}/.well-known/oauth-authorization-server"))?;
    if text(&metadata, "issuer").as_deref() != Some(issuer.as_str()) {
        bail!("the site's sign-in server doesn't name itself as {issuer}");
    }
    let supports_s256 = metadata["code_challenge_methods_supported"].as_array().is_some_and(|methods| methods.iter().any(|m| m == "S256"));
    if !supports_s256 {
        bail!("the site doesn't offer PKCE with S256, so signing in isn't safe");
    }
    let endpoint = |key: &str| -> Result<Option<String>> {
        match text(&metadata, key) {
            Some(url) if http::same_origin(&url, &issuer) => Ok(Some(url)),
            Some(url) => bail!("the site's {key} ({url}) is on another site"),
            None => Ok(None),
        }
    };
    Ok(Server {
        authorization_endpoint: endpoint("authorization_endpoint")?.context("no authorization endpoint")?,
        token_endpoint: endpoint("token_endpoint")?.context("no token endpoint")?,
        revocation_endpoint: endpoint("revocation_endpoint")?,
        device_endpoint: endpoint("device_authorization_endpoint")?,
        issuer,
        api,
    })
}

fn random(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::fill(&mut buffer[..]);
    URL_SAFE_NO_PAD.encode(buffer)
}

pub fn browser_login(server: &Server, open_browser: bool) -> Result<Tokens> {
    let listener = TcpListener::bind("127.0.0.1:0").context("couldn't listen on this computer for the sign-in to come back")?;
    let redirect_uri = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());
    let state = random(32);
    let verifier = random(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let mut url = Url::parse(&server.authorization_endpoint)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("resource", &server.api);

    anstream::eprintln!("{}\n  {}", paint(HEADING, "Sign in and choose what the CLI may do:"), paint(LINK, &url));
    if open_browser && webbrowser::open(url.as_str()).is_err() {
        anstream::eprintln!("{}", paint(DIM, "Couldn't open a browser; open the address above yourself."));
    }

    let params = wait_for_callback(listener)?;
    if params.get("state") != Some(&state) {
        bail!("the sign-in came back with the wrong state; nothing was saved");
    }
    if let Some(iss) = params.get("iss")
        && iss != &server.issuer
    {
        bail!("the sign-in came back from {iss}, not {}; nothing was saved", server.issuer);
    }
    if let Some(error) = params.get("error") {
        bail!("{}", params.get("error_description").cloned().unwrap_or_else(|| error.clone()));
    }
    let code = params.get("code").context("the sign-in came back without a code")?;

    token_request(
        server,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("client_id", CLIENT_ID),
            ("code_verifier", &verifier),
            ("resource", &server.api),
        ],
    )
}

fn wait_for_callback(listener: TcpListener) -> Result<std::collections::HashMap<String, String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            let url = Url::parse(&format!("http://127.0.0.1{target}"));
            let Ok(url) = url else { continue };
            if url.path() != "/callback" {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                continue;
            }
            let body = "<!doctype html><meta charset=utf-8><title>Nibble</title><p style=\"font:16px system-ui;margin:4rem auto;max-width:28rem\">You can close this tab and go back to your terminal.";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sender.send(url.query_pairs().into_owned().collect());
            break;
        }
    });
    receiver.recv_timeout(WAIT).map_err(|_| anyhow!("gave up waiting for the sign-in after {} minutes", WAIT.as_secs() / 60))
}

pub fn device_login(server: &Server) -> Result<Tokens> {
    let endpoint = server
        .device_endpoint
        .as_deref()
        .context("this site doesn't allow signing in with a code; an administrator can turn it on under Agent access")?;
    let response = http::client().post(endpoint).form(&[("client_id", CLIENT_ID), ("resource", server.api.as_str())]).send()?;
    let body: Value = response.json()?;
    let device_code = text(&body, "device_code").ok_or_else(|| Problem::from_body(400, &body))?;
    let user_code = text(&body, "user_code").unwrap_or_default();
    let verification = text(&body, "verification_uri").unwrap_or_default();
    if !http::same_origin(&verification, &server.issuer) {
        bail!("the site sent a sign-in address on another site ({verification}); stopping");
    }
    anstream::eprintln!(
        "{} {}\n{} {}\n{}",
        paint(HEADING, "On any device, open"),
        paint(LINK, &verification),
        paint(HEADING, "and enter the code"),
        paint(CHANGE, &user_code),
        paint(DIM, "Only do this if you started it yourself.")
    );

    let mut interval = body["interval"].as_u64().unwrap_or(5).max(1);
    let deadline = now() + body["expires_in"].as_i64().unwrap_or(600);
    while now() < deadline {
        std::thread::sleep(Duration::from_secs(interval));
        match token_request(
            server,
            &[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", &device_code), ("client_id", CLIENT_ID)],
        ) {
            Ok(tokens) => return Ok(tokens),
            Err(error) => match error.downcast_ref::<Problem>().map(|problem| problem.code.as_str()) {
                Some("authorization_pending") => {}
                Some("slow_down") => interval += 5,
                _ => return Err(error),
            },
        }
    }
    bail!("the code expired before anyone approved it")
}

pub fn refresh(site: &crate::config::Site, refresh_token: &str) -> Result<Tokens> {
    let server = Server {
        issuer: site.issuer.clone(),
        api: site.api.clone(),
        authorization_endpoint: String::new(),
        token_endpoint: site.token_endpoint.clone(),
        revocation_endpoint: None,
        device_endpoint: None,
    };
    token_request(
        &server,
        &[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", CLIENT_ID), ("resource", &site.api)],
    )
}

fn token_request(server: &Server, form: &[(&str, &str)]) -> Result<Tokens> {
    let response = http::client().post(&server.token_endpoint).form(form).send().context("couldn't reach the site to sign in")?;
    let status = response.status().as_u16();
    let body: Value = response.json().unwrap_or(Value::Null);
    if status != 200 {
        return Err(Problem::from_body(status, &body).into());
    }
    Ok(Tokens {
        access_token: text(&body, "access_token").context("no access token came back")?,
        refresh_token: text(&body, "refresh_token"),
        expires_at: now() + body["expires_in"].as_i64().unwrap_or(3600),
    })
}

pub fn revoke(site: &crate::config::Site, token: &str) {
    if let Some(endpoint) = &site.revocation_endpoint {
        let _ = http::client().post(endpoint).form(&[("token", token), ("client_id", CLIENT_ID)]).send();
    }
}
