use crate::api::Session;
use crate::http::Problem;
use crate::output::Output;
use crate::style::{CHANGE, DIM, HEADING, NAME, STRONG, paint};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

const FIRST_POLL: Duration = Duration::from_secs(2);
const LONGEST_POLL: Duration = Duration::from_secs(10);

pub struct Invocation {
    pub operation: Option<String>,
    pub input: Vec<String>,
    pub help: bool,
}

#[derive(Default)]
pub struct Globals {
    pub json: bool,
    pub site: Option<String>,
    pub pick: Option<String>,
    pub no_wait: bool,
}

pub fn globals(words: Vec<String>) -> (Vec<String>, Globals) {
    let mut globals = Globals::default();
    let mut rest = Vec::new();
    let mut words = words.into_iter();
    while let Some(word) = words.next() {
        match word.split_once('=').map_or((word.as_str(), None), |(key, value)| (key, Some(value.to_string()))) {
            ("--json", None) => globals.json = true,
            ("--no-wait", None) => globals.no_wait = true,
            ("--site", value) => globals.site = value.or_else(|| words.next()),
            ("--pick", value) => globals.pick = value.or_else(|| words.next()),
            _ => rest.push(word),
        }
    }
    (rest, globals)
}

pub fn parse(words: Vec<String>) -> Invocation {
    let help = words.iter().any(|word| word == "--help" || word == "-h");
    let mut words = words.into_iter().filter(|word| word != "--help" && word != "-h");
    let operation = words.next().map(|name| name.replace('-', "_"));
    Invocation { operation, input: words.collect(), help }
}

fn operations(catalogue: &Value) -> &[Value] {
    catalogue["data"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

pub fn list(session: &mut Session, output: Output) -> Result<()> {
    let catalogue = session.catalogue()?;
    crate::complete::remember(session, &catalogue);
    if output.json {
        output.success(&catalogue["data"], Some(&session.origin), None);
        return Ok(());
    }
    anstream::println!("{} {}\n", paint(HEADING, "Operations on"), paint(STRONG, format!("{} ({})", session.site.name, session.label)));
    for operation in operations(&catalogue) {
        let name = operation["name"].as_str().unwrap_or_default();
        let marker = if operation["annotations"]["readOnlyHint"] == json!(true) { " ".into() } else { paint(CHANGE, "*") };
        anstream::println!("  {marker} {} {}", paint(NAME, format!("{name:<22}")), operation["title"].as_str().unwrap_or_default());
    }
    let unavailable = catalogue["unavailable"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if !unavailable.is_empty() {
        anstream::println!("\n{}", paint(DIM, "Not available on this connection:"));
        for operation in unavailable {
            let name = operation["name"].as_str().unwrap_or_default();
            let title = operation["title"].as_str().unwrap_or_default();
            let needs = operation["needs"].as_str().unwrap_or_default();
            anstream::println!("{}", paint(DIM, format!("    {name:<22} {title}, needs {needs}")));
        }
    }
    anstream::println!(
        "\n{} {}",
        paint(CHANGE, "*"),
        paint(DIM, "changes content. `nibble remote <operation> --help` shows its arguments.")
    );
    Ok(())
}

pub fn describe(operation: &Value) {
    anstream::println!(
        "{}\n\n{}\n",
        paint(HEADING, operation["title"].as_str().unwrap_or_default()),
        operation["description"].as_str().unwrap_or_default()
    );
    let required: Vec<&str> =
        operation["input"]["required"].as_array().map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if let Some(properties) = operation["input"]["properties"].as_object() {
        anstream::println!("{}", paint(HEADING, "Arguments"));
        let kinds: Vec<String> = properties
            .values()
            .map(|schema| match &schema["type"] {
                Value::Array(types) => types.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("|"),
                other => other.as_str().unwrap_or("any").to_string(),
            })
            .collect();
        let width = kinds.iter().map(String::len).max().unwrap_or(0);
        for ((name, schema), kind) in properties.iter().zip(&kinds) {
            let flag = if name == "dry_run" { "--dry-run".to_string() } else { format!("--{}", name.replace('_', "-")) };
            let needed = if required.contains(&name.as_str()) { paint(CHANGE, "required") } else { " ".repeat(8) };
            anstream::println!(
                "  {} {} {needed}  {}",
                paint(NAME, format!("{flag:<20}")),
                paint(DIM, format!("{kind:<width$}")),
                schema["description"].as_str().unwrap_or_default()
            );
        }
        if operation["name"] == json!("upload_asset") {
            anstream::println!(
                "  {} {} {}  Upload this file from your computer instead of --url or --content-base64",
                paint(NAME, format!("{:<20}", "--file")),
                paint(DIM, format!("{:<width$}", "path")),
                " ".repeat(8)
            );
        }
    }
    anstream::println!("\n{}", paint(DIM, "Objects and arrays take JSON, @file.json, or - for standard input."));
}

pub fn run(session: &mut Session, invocation: Invocation, output: Output, pick: Option<&str>, no_wait: bool) -> Result<()> {
    let Some(name) = invocation.operation else { return list(session, output) };
    let catalogue = session.catalogue()?;
    crate::complete::remember(session, &catalogue);
    let Some(operation) = operations(&catalogue).iter().find(|operation| operation["name"] == json!(name)).cloned() else {
        match unavailable_message(&catalogue, &name, &crate::config::host(&session.origin)) {
            Some(message) => bail!("{message}"),
            None => bail!("this connection has no operation called {name}; `nibble remote` lists them"),
        }
    };
    if invocation.help {
        describe(&operation);
        return Ok(());
    }

    let (words, file) = split_file(&invocation.input)?;
    let mut input = build_input(&operation, &words)?;
    if let Some(file) = file {
        let plan = upload_plan(&name, &file, &input)?;
        if !plan.path.is_file() {
            bail!("{} isn't a file", plan.path.display());
        }
        if !output.json {
            anstream::eprintln!(
                "{} {} to {}",
                paint(CHANGE, "→"),
                paint(NAME, format!("uploading {}", plan.filename)),
                paint(STRONG, &session.label)
            );
        }
        let uploaded = session.upload(&plan.path, &plan.filename)?;
        input.remove("filename");
        input.insert("upload".into(), uploaded["data"]["upload"].clone());
    }
    let changes = operation["annotations"]["readOnlyHint"] != json!(true);
    if changes && !output.json {
        anstream::eprintln!("{} {} on {}", paint(CHANGE, "→"), paint(NAME, &name), paint(STRONG, &session.label));
    }
    let response = match session.call(&name, &Value::Object(input)) {
        Ok(response) => response,
        Err(error) => match approval_of(&error) {
            Some(approval) if !output.json => {
                anstream::eprintln!("{} {}", paint(CHANGE, "!"), paint(STRONG, format!("Needs your approval: {}", approval.url)));
                if no_wait {
                    anstream::eprintln!(
                        "{}",
                        paint(
                            DIM,
                            format!(
                                "Check it later with `nibble remote get-approval --site {} --approval {}`.",
                                session.site_flag(),
                                approval.id
                            )
                        )
                    );
                    return Ok(());
                }
                json!({ "data": wait_for(session, &approval)? })
            }
            _ => return Err(error),
        },
    };
    if name == "describe_site" {
        crate::complete::remember_site(&session.key(), &response["data"]);
    }
    output.success(&response["data"], Some(&session.origin), pick);
    Ok(())
}

pub fn unavailable_message(catalogue: &Value, name: &str, site: &str) -> Option<String> {
    let operation = catalogue["unavailable"].as_array()?.iter().find(|operation| operation["name"] == json!(name))?;
    let title = operation["title"].as_str().unwrap_or(name).to_lowercase();
    let needs = operation["needs"].as_str().unwrap_or("more access");
    Some(format!(
        "This connection can't {title}: it needs {needs}. Change its access under Account → Connected apps on the site, \
         or run `nibble auth login {site}` again."
    ))
}

#[derive(Debug, PartialEq)]
pub struct UploadPlan {
    pub path: PathBuf,
    pub filename: String,
}

pub fn split_file(words: &[String]) -> Result<(Vec<String>, Option<String>)> {
    let mut rest = Vec::new();
    let mut file = None;
    let mut words = words.iter();
    while let Some(word) = words.next() {
        if word == "--file" {
            file = Some(words.next().cloned().context("--file needs a path")?);
        } else if let Some(path) = word.strip_prefix("--file=") {
            file = Some(path.to_string());
        } else {
            rest.push(word.clone());
        }
    }
    Ok((rest, file))
}

pub fn upload_plan(operation: &str, file: &str, input: &Map<String, Value>) -> Result<UploadPlan> {
    if operation != "upload_asset" {
        bail!("--file is for upload-asset");
    }
    if let Some(other) = ["upload", "url", "content_base64"].into_iter().find(|key| input.contains_key(*key)) {
        bail!("--file can't be combined with --{}; give the file one way", other.replace('_', "-"));
    }
    let path = PathBuf::from(file);
    let filename = match input.get("filename").and_then(Value::as_str) {
        Some(name) => name.to_string(),
        None => {
            path.file_name().and_then(|name| name.to_str()).with_context(|| format!("{file} has no file name; add --filename"))?.to_string()
        }
    };
    Ok(UploadPlan { path, filename })
}

pub struct Approval {
    pub id: String,
    pub url: String,
}

pub fn approval_of(error: &anyhow::Error) -> Option<Approval> {
    let problem = error.downcast_ref::<Problem>().filter(|problem| problem.code == "approval_required")?;
    let details = problem.details.as_ref()?;
    Some(Approval { id: details["approval"].as_str()?.to_string(), url: details["approval_url"].as_str()?.to_string() })
}

#[derive(Debug, PartialEq)]
pub enum Poll {
    Waiting,
    Done(Value),
    Stopped { code: String, message: String },
}

pub fn poll_decision(approval: &Value) -> Poll {
    let stopped = |code: &str, message: &str| Poll::Stopped { code: code.into(), message: message.into() };
    match approval["status"].as_str() {
        Some("pending" | "approved") => Poll::Waiting,
        Some("done") => Poll::Done(approval["result"].clone()),
        Some("failed") => Poll::Stopped {
            code: approval["error"]["code"].as_str().unwrap_or("error").into(),
            message: approval["error"]["message"].as_str().unwrap_or("the change failed after it was approved").into(),
        },
        Some("denied") => stopped("approval_denied", "The person declined it; nothing changed."),
        Some("expired") => stopped("approval_expired", "Nobody approved it in time; run the command again to ask again."),
        other => stopped("error", &format!("the site reported an approval status this nibble doesn't know ({})", other.unwrap_or("none"))),
    }
}

pub fn next_delay(previous: Duration) -> Duration {
    (previous * 3 / 2).min(LONGEST_POLL)
}

fn wait_for(session: &mut Session, approval: &Approval) -> Result<Value> {
    let (id, site) = (approval.id.clone(), session.site_flag());
    let _ = ctrlc::set_handler(move || {
        anstream::eprintln!(
            "\nStopped waiting; the request stays open. Check it with: nibble remote get-approval --site {site} --approval {id}"
        );
        std::process::exit(130);
    });
    anstream::eprintln!("{}", paint(DIM, "Waiting for approval… (Ctrl-C to stop waiting; the request stays open)"));
    let mut delay = FIRST_POLL;
    loop {
        std::thread::sleep(delay);
        let reply = match session.call("get_approval", &json!({ "approval": approval.id })) {
            Ok(reply) => reply,
            Err(error) if error.downcast_ref::<Problem>().is_some_and(|problem| problem.code == "unknown_operation") => {
                bail!(
                    "{} doesn't make approved changes itself yet: approve it at the link, then run the same command again with --approval {}",
                    session.origin,
                    approval.id
                )
            }
            Err(error) => return Err(error),
        };
        match poll_decision(&reply["data"]) {
            Poll::Waiting => delay = next_delay(delay),
            Poll::Done(result) => {
                crate::style::done("Approved");
                return Ok(result);
            }
            Poll::Stopped { code, message } => {
                return Err(Problem { status: 409, code, detail: message, hint: None, details: None }.into());
            }
        }
    }
}

fn build_input(operation: &Value, words: &[String]) -> Result<Map<String, Value>> {
    let properties = operation["input"]["properties"].as_object().cloned().unwrap_or_default();
    let mut input = Map::new();
    let mut words = words.iter().peekable();
    while let Some(word) = words.next() {
        let Some(flag) = word.strip_prefix("--") else { bail!("unexpected {word}; arguments look like --collection posts") };
        let (key, inline) = match flag.split_once('=') {
            Some((key, value)) => (key.replace('-', "_"), Some(value.to_string())),
            None => (flag.replace('-', "_"), None),
        };
        let schema = properties.get(&key).with_context(|| {
            format!(
                "{} takes no --{}; it takes {}",
                operation["name"].as_str().unwrap_or_default(),
                key.replace('_', "-"),
                properties.keys().map(|key| format!("--{}", key.replace('_', "-"))).collect::<Vec<_>>().join(", ")
            )
        })?;
        let kind = schema["type"].as_str().unwrap_or("string");
        let raw = match inline {
            Some(value) => value,
            None if kind == "boolean" && words.peek().is_none_or(|next| next.starts_with("--")) => "true".into(),
            None => words.next().cloned().with_context(|| format!("--{} needs a value", key.replace('_', "-")))?,
        };
        input.insert(key.clone(), convert(&key, kind, &raw)?);
    }
    Ok(input)
}

fn convert(key: &str, kind: &str, raw: &str) -> Result<Value> {
    let text = if raw == "-" {
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path).with_context(|| format!("couldn't read {path}"))?
    } else {
        raw.to_string()
    };
    Ok(match kind {
        "integer" => json!(text.trim().parse::<i64>().with_context(|| format!("--{key} takes a whole number"))?),
        "boolean" => json!(matches!(text.trim(), "true" | "yes" | "1")),
        "object" | "array" => serde_json::from_str(&text).with_context(|| format!("--{key} takes JSON"))?,
        _ => json!(text),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation() -> Value {
        json!({ "name": "update_entry", "input": { "properties": {
            "id": { "type": "integer" }, "lock_version": { "type": "integer" }, "data": { "type": "object" }, "dry_run": { "type": "boolean" }
        } } })
    }

    fn args(text: &[&str]) -> Vec<String> {
        text.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn arguments_take_the_types_the_site_declares() {
        let input =
            build_input(&operation(), &args(&["--id", "7", "--lock-version=2", "--data", "{\"title\":\"Hi\"}", "--dry-run"])).unwrap();
        assert_eq!(Value::Object(input), json!({ "id": 7, "lock_version": 2, "data": { "title": "Hi" }, "dry_run": true }));
    }

    #[test]
    fn a_mistyped_argument_names_the_ones_that_exist() {
        let error = build_input(&operation(), &args(&["--titel", "x"])).unwrap_err().to_string();
        assert!(error.contains("--lock-version"), "{error}");
        assert!(build_input(&operation(), &args(&["--id", "seven"])).is_err());
    }

    #[test]
    fn global_flags_after_the_operation_still_apply() {
        let (rest, globals) =
            globals(args(&["list_entries", "--pick", "entries.0.title", "--json", "--site=a.test", "--collection", "posts"]));
        assert_eq!(rest, args(&["list_entries", "--collection", "posts"]));
        assert!(globals.json);
        assert_eq!(globals.site.as_deref(), Some("a.test"));
        assert_eq!(globals.pick.as_deref(), Some("entries.0.title"));
    }

    #[test]
    fn file_is_taken_out_of_the_arguments_before_the_site_sees_them() {
        let (rest, file) = split_file(&args(&["--folder", "Blog", "--file", "photo.jpg", "--alt=A cat"])).unwrap();
        assert_eq!(rest, args(&["--folder", "Blog", "--alt=A cat"]));
        assert_eq!(file.as_deref(), Some("photo.jpg"));
        assert_eq!(split_file(&args(&["--file=a.png"])).unwrap().1.as_deref(), Some("a.png"));
        assert!(split_file(&args(&["--file"])).is_err());
    }

    fn input(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_file_uploads_under_its_own_name_unless_told_otherwise() {
        let plan = upload_plan("upload_asset", "/tmp/shots/photo.jpg", &input(json!({ "folder": "Blog" }))).unwrap();
        assert_eq!(plan, UploadPlan { path: "/tmp/shots/photo.jpg".into(), filename: "photo.jpg".into() });
        let renamed = upload_plan("upload_asset", "photo.jpg", &input(json!({ "filename": "cat.jpg" }))).unwrap();
        assert_eq!(renamed.filename, "cat.jpg");
    }

    #[test]
    fn a_file_is_one_way_to_give_an_asset_and_only_for_upload_asset() {
        for key in ["upload", "url", "content_base64"] {
            let error = upload_plan("upload_asset", "a.jpg", &input(json!({ key: "x" }))).unwrap_err().to_string();
            assert!(error.contains(&key.replace('_', "-")), "{error}");
        }
        assert!(upload_plan("create_entry", "a.jpg", &Map::new()).is_err());
    }

    #[test]
    fn an_unavailable_operation_says_what_it_needs_and_how_to_get_it() {
        let catalogue =
            json!({ "data": [], "unavailable": [{ "name": "upload_asset", "title": "Upload an asset", "needs": "Assets: Write" }] });
        let message = unavailable_message(&catalogue, "upload_asset", "example.com").unwrap();
        assert_eq!(
            message,
            "This connection can't upload an asset: it needs Assets: Write. Change its access under Account → Connected apps on the site, or run `nibble auth login example.com` again."
        );
        assert_eq!(unavailable_message(&catalogue, "no_such", "example.com"), None);
        assert_eq!(unavailable_message(&json!({ "data": [] }), "upload_asset", "example.com"), None, "older sites list none");
    }

    #[test]
    fn waiting_goes_on_until_the_site_has_made_the_change_or_given_up() {
        assert_eq!(poll_decision(&json!({ "status": "pending" })), Poll::Waiting);
        assert_eq!(poll_decision(&json!({ "status": "approved" })), Poll::Waiting, "approved, but not yet made");
        assert_eq!(poll_decision(&json!({ "status": "done", "result": { "id": 4 } })), Poll::Done(json!({ "id": 4 })));
        let failed = poll_decision(&json!({ "status": "failed", "error": { "code": "conflict", "message": "Someone else changed it" } }));
        assert_eq!(failed, Poll::Stopped { code: "conflict".into(), message: "Someone else changed it".into() });
        for (status, code) in [("denied", "approval_denied"), ("expired", "approval_expired"), ("surprising", "error")] {
            assert!(matches!(poll_decision(&json!({ "status": status })), Poll::Stopped { code: found, .. } if found == code), "{status}");
        }
    }

    #[test]
    fn polling_slows_down_but_never_beyond_ten_seconds() {
        let mut delay = FIRST_POLL;
        let mut seen = vec![delay];
        for _ in 0..8 {
            delay = next_delay(delay);
            seen.push(delay);
        }
        assert!(seen.windows(2).all(|pair| pair[1] >= pair[0]));
        assert_eq!(seen.last(), Some(&LONGEST_POLL));
        assert_eq!(seen[0], Duration::from_secs(2));
    }

    #[test]
    fn only_approval_required_with_a_link_is_something_to_wait_for() {
        let body = json!({ "code": "approval_required", "detail": "Ada has to approve", "details": { "approval": "appr_1", "approval_url": "https://a.test/cp/approvals/appr_1" } });
        let waiting = approval_of(&Problem::from_body(409, &body).into()).unwrap();
        assert_eq!((waiting.id.as_str(), waiting.url.as_str()), ("appr_1", "https://a.test/cp/approvals/appr_1"));
        let other = Problem::from_body(409, &json!({ "code": "approval_pending", "detail": "not yet" }));
        assert!(approval_of(&other.into()).is_none());
        assert!(approval_of(&anyhow::anyhow!("plain")).is_none());
    }
}
