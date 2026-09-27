use crate::api::Session;
use crate::output::Output;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::io::Read;

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
}

pub fn globals(words: Vec<String>) -> (Vec<String>, Globals) {
    let mut globals = Globals::default();
    let mut rest = Vec::new();
    let mut words = words.into_iter();
    while let Some(word) = words.next() {
        match word.split_once('=').map_or((word.as_str(), None), |(key, value)| (key, Some(value.to_string()))) {
            ("--json", None) => globals.json = true,
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
    if output.json {
        output.success(&catalogue["data"], Some(&session.origin), None);
        return Ok(());
    }
    println!("On {} ({}), this connection can run:\n", session.site.name, session.label);
    for operation in operations(&catalogue) {
        let name = operation["name"].as_str().unwrap_or_default();
        let marker = if operation["annotations"]["readOnlyHint"] == json!(true) { " " } else { "*" };
        println!("  {marker} {name:<22} {}", operation["title"].as_str().unwrap_or_default());
    }
    println!("\n* changes content. `nibble remote <operation> --help` shows its arguments.");
    Ok(())
}

pub fn describe(operation: &Value) {
    println!("{}\n\n{}\n", operation["title"].as_str().unwrap_or_default(), operation["description"].as_str().unwrap_or_default());
    let required: Vec<&str> =
        operation["input"]["required"].as_array().map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if let Some(properties) = operation["input"]["properties"].as_object() {
        println!("Arguments:");
        for (name, schema) in properties {
            let kind = match &schema["type"] {
                Value::Array(types) => types.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("|"),
                other => other.as_str().unwrap_or("any").to_string(),
            };
            let flag = if name == "dry_run" { "--dry-run".to_string() } else { format!("--{}", name.replace('_', "-")) };
            let needed = if required.contains(&name.as_str()) { " (required)" } else { "" };
            println!("  {flag:<20} {kind}{needed}  {}", schema["description"].as_str().unwrap_or_default());
        }
    }
    println!("\nObjects and arrays take JSON, @file.json, or - for standard input.");
}

pub fn run(session: &mut Session, invocation: Invocation, output: Output, pick: Option<&str>) -> Result<()> {
    let Some(name) = invocation.operation else { return list(session, output) };
    let catalogue = session.catalogue()?;
    let operation = operations(&catalogue)
        .iter()
        .find(|operation| operation["name"] == json!(name))
        .with_context(|| format!("this connection has no operation called {name}; `nibble remote` lists them"))?
        .clone();
    if invocation.help {
        describe(&operation);
        return Ok(());
    }

    let input = build_input(&operation, &invocation.input)?;
    let changes = operation["annotations"]["readOnlyHint"] != json!(true);
    if changes && !output.json {
        eprintln!("→ {} on {}", name, session.label);
    }
    let response = session.call(&name, &Value::Object(input))?;
    output.success(&response["data"], Some(&session.origin), pick);
    Ok(())
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
}
