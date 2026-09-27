use crate::api::Session;
use crate::config::{self, Config, Profile};
use clap::{Arg, Command};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

pub fn script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => include_str!("completion/nibble.bash"),
        Shell::Zsh => include_str!("completion/nibble.zsh"),
        Shell::Fish => include_str!("completion/nibble.fish"),
        Shell::Powershell => include_str!("completion/nibble.ps1"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fallback {
    None,
    Files,
    Dirs,
}

#[derive(Debug, PartialEq)]
pub struct Completion {
    pub candidates: Vec<(String, String)>,
    pub fallback: Fallback,
}

impl Completion {
    fn none() -> Self {
        Completion { candidates: Vec::new(), fallback: Fallback::None }
    }

    fn fallback(fallback: Fallback) -> Self {
        Completion { candidates: Vec::new(), fallback }
    }

    fn of(current: &str, candidates: impl IntoIterator<Item = (String, String)>) -> Self {
        let candidates = candidates.into_iter().filter(|(value, _)| value.starts_with(current)).collect();
        Completion { candidates, fallback: Fallback::None }
    }

    fn prefixed(mut self, prefix: &str) -> Self {
        for (value, _) in &mut self.candidates {
            value.insert_str(0, prefix);
        }
        self
    }

    pub fn render(&self, shell_word: Option<&str>, current: &str) -> String {
        let mut text = String::from(match self.fallback {
            Fallback::None => ":none\n",
            Fallback::Files => ":files\n",
            Fallback::Dirs => ":dirs\n",
        });
        // Bash splits words at : and =, and replaces only the part after the last one, so it gets that part alone.
        let cut = shell_word.map_or(0, |word| current.len().saturating_sub(word.len()));
        for (value, help) in &self.candidates {
            let value = value.get(cut..).unwrap_or(value);
            text.push_str(&format!("{value}\t{}\n", help.replace(['\t', '\n'], " ")));
        }
        text
    }
}

pub struct Account {
    pub host: String,
    pub email: String,
    pub name: String,
    pub access: String,
    pub default: bool,
}

#[derive(Default)]
pub struct Known {
    pub accounts: Vec<Account>,
    pub tasks: Vec<(String, String)>,
    pub site: Option<Value>,
}

impl Known {
    pub fn load(words: &[String], site_from_env: Option<String>, target: impl Fn(Option<String>) -> Option<Profile>) -> Self {
        let config = Config::load().unwrap_or_default();
        let accounts = config
            .profiles()
            .iter()
            .map(|profile| Account {
                host: config::host(&profile.origin),
                email: profile.account.email.clone(),
                name: profile.site.name.clone(),
                access: profile.account.access.clone(),
                default: config.default.as_deref() == Some(profile.key().as_str()),
            })
            .collect();
        let tasks = std::env::current_dir().ok().and_then(|cwd| crate::project::root(&cwd)).map(|root| crate::project::cached_tasks(&root));
        let named = named_site(words).or(site_from_env);
        let key = if std::env::var_os("NIBBLE_TOKEN").is_some() {
            named.as_deref().and_then(|site| crate::http::origin(site).ok()).map(|origin| token_key(&origin))
        } else {
            target(named).map(|profile| profile.key())
        };
        Known { accounts, tasks: tasks.unwrap_or_default(), site: key.and_then(|key| cached(&key)) }
    }
}

fn named_site(words: &[String]) -> Option<String> {
    let mut words = words.iter();
    let mut found = None;
    while let Some(word) = words.next() {
        if word == "--site" {
            found = words.next().cloned();
        } else if let Some(site) = word.strip_prefix("--site=") {
            found = Some(site.to_string());
        }
    }
    found
}

pub fn complete(root: &Command, before: &[String], current: &str, known: &Known) -> Completion {
    let mut command = root;
    let mut positionals = 0;
    let mut used: Vec<String> = Vec::new();
    let mut index = 0;
    while index < before.len() {
        let word = &before[index];
        if word == "--" {
            return Completion::none();
        }
        if let Some(flag) = word.strip_prefix("--") {
            let (name, inline) = flag.split_once('=').map_or((flag, false), |(name, _)| (name, true));
            used.push(name.to_string());
            if !inline && let Some(arg) = long(command, name).filter(|arg| arg.get_action().takes_values()) {
                if index + 1 == before.len() {
                    return values(arg, current, known);
                }
                index += 1;
            }
        } else if !word.starts_with('-') {
            match command.find_subcommand(word).filter(|sub| !sub.is_hide_set()) {
                Some(sub) if sub.get_name() == "remote" => return remote(&before[index + 1..], current, known),
                Some(sub) => {
                    command = sub;
                    positionals = 0;
                }
                None if std::ptr::eq(command, root) => return task(&before[index..], current, known),
                None => positionals += 1,
            }
        }
        index += 1;
    }

    if let Some((name, value)) = current.strip_prefix("--").and_then(|flag| flag.split_once('=')) {
        return match long(command, name) {
            Some(arg) => values(arg, value, known).prefixed(&format!("--{name}=")),
            None => Completion::none(),
        };
    }
    if current.starts_with('-') {
        return flags(command, &used, current);
    }
    let mut candidates: Vec<(String, String)> = command
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .map(|sub| (sub.get_name().to_string(), sub.get_about().map(|about| about.to_string()).unwrap_or_default()))
        .collect();
    if std::ptr::eq(command, root) {
        candidates.extend(task_steps(&known.tasks, "", |_| true));
    }
    let positional = command.get_positionals().filter(|arg| !arg.is_last_set()).nth(positionals);
    match positional {
        Some(arg) if candidates.is_empty() => values(arg, current, known),
        _ if candidates.is_empty() => flags(command, &used, current),
        _ => Completion::of(current, candidates),
    }
}

fn long<'a>(command: &'a Command, name: &str) -> Option<&'a Arg> {
    command.get_arguments().find(|arg| arg.get_long() == Some(name))
}

fn flags(command: &Command, used: &[String], current: &str) -> Completion {
    let candidates = command.get_arguments().filter(|arg| !arg.is_hide_set()).filter_map(|arg| {
        let name = arg.get_long()?;
        let repeatable = matches!(arg.get_action(), clap::ArgAction::Append | clap::ArgAction::Count);
        (repeatable || !used.iter().any(|seen| seen == name))
            .then(|| (format!("--{name}"), arg.get_help().map(|help| help.to_string()).unwrap_or_default()))
    });
    Completion::of(current, candidates)
}

fn values(arg: &Arg, current: &str, known: &Known) -> Completion {
    let choices: Vec<(String, String)> = arg
        .get_possible_values()
        .into_iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| (value.get_name().to_string(), value.get_help().map(|help| help.to_string()).unwrap_or_default()))
        .collect();
    if !choices.is_empty() {
        return Completion::of(current, choices);
    }
    match arg.get_id().as_str() {
        "site" | "profile" => Completion::of(current, accounts(known)),
        "url" => {
            let mut hosts: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for account in &known.accounts {
                hosts.entry(&account.host).or_default().push(&account.email);
            }
            Completion::of(
                current,
                hosts.into_iter().map(|(host, emails)| (host.to_string(), format!("signed in as {}", emails.join(", ")))),
            )
        }
        "dir" => Completion::fallback(Fallback::Dirs),
        _ => Completion::none(),
    }
}

fn accounts(known: &Known) -> Vec<(String, String)> {
    known
        .accounts
        .iter()
        .map(|account| {
            let shared = known.accounts.iter().filter(|other| other.host == account.host).count() > 1;
            let value = if shared { format!("{}:{}", account.host, account.email) } else { account.host.clone() };
            let default = if account.default { ", used when no site is named" } else { "" };
            (value, format!("{} as {} ({} access{default})", account.name, account.email, account.access))
        })
        .collect()
}

fn task_steps<'a>(tasks: &'a [(String, String)], path: &'a str, keep: impl Fn(&str) -> bool + 'a) -> Vec<(String, String)> {
    let mut steps: BTreeMap<String, String> = BTreeMap::new();
    for (name, about) in tasks {
        let rest = if path.is_empty() { Some(name.as_str()) } else { name.strip_prefix(path).and_then(|rest| rest.strip_prefix(':')) };
        let Some(rest) = rest else { continue };
        let (step, deeper) = rest.split_once(':').map_or((rest, false), |(step, _)| (step, true));
        if !keep(step) {
            continue;
        }
        let full = if path.is_empty() { step.to_string() } else { format!("{path}:{step}") };
        let help = if deeper { format!("{} tasks", full.replace(':', " ")) } else { about.clone() };
        let entry = steps.entry(step.to_string()).or_default();
        if entry.is_empty() || !deeper {
            *entry = help;
        }
    }
    steps.into_iter().collect()
}

fn task(typed: &[String], current: &str, known: &Known) -> Completion {
    let words: Vec<&str> = typed.iter().map(String::as_str).take_while(|word| !word.starts_with('-')).collect();
    let path = words.join(":");
    if words.len() < typed.len() || known.tasks.iter().any(|(name, _)| *name == path) {
        return Completion::fallback(Fallback::Files);
    }
    let steps = task_steps(&known.tasks, &path, |_| true);
    if steps.is_empty() { Completion::fallback(Fallback::Files) } else { Completion::of(current, steps) }
}

const REMOTE_FLAGS: [(&str, &str); 4] = [
    ("--help", "Show the operation's arguments"),
    ("--json", "Print JSON"),
    ("--site", "The site to use"),
    ("--pick", "Print only this part of the result"),
];

fn remote(words: &[String], current: &str, known: &Known) -> Completion {
    let operations: Vec<Value> = known.site.as_ref().and_then(|site| site["operations"].as_array().cloned()).unwrap_or_default();
    let mut operation: Option<&Value> = None;
    let mut given: Vec<(String, Option<String>)> = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        let last = index + 1 == words.len();
        if word == "--site" || word == "--pick" {
            if last {
                return if word == "--site" { Completion::of(current, accounts(known)) } else { Completion::none() };
            }
            index += 1;
        } else if let Some(flag) = word.strip_prefix("--") {
            let (key, inline) = flag.split_once('=').map_or((flag, None), |(key, value)| (key, Some(value.to_string())));
            let key = key.replace('-', "_");
            if let Some(op) = operation {
                let schema = &op["input"]["properties"][&key];
                if inline.is_none() && !schema.is_null() && !is_boolean(schema) {
                    if last {
                        return property_values(op, &key, &given, current, known);
                    }
                    given.push((key, words.get(index + 1).cloned()));
                    index += 1;
                } else {
                    given.push((key, inline));
                }
            }
        } else if operation.is_none() {
            let name = word.replace('-', "_");
            operation = operations.iter().find(|op| op["name"] == json!(name));
            if operation.is_none() {
                return Completion::none();
            }
        }
        index += 1;
    }

    let Some(op) = operation else {
        if current.starts_with('-') {
            return Completion::of(current, REMOTE_FLAGS.iter().map(|(flag, help)| (flag.to_string(), help.to_string())));
        }
        let hyphens = !current.contains('_');
        let candidates = operations.iter().filter_map(|op| {
            let name = op["name"].as_str()?;
            let name = if hyphens { name.replace('_', "-") } else { name.to_string() };
            let title = op["title"].as_str().unwrap_or_default();
            let changes = op["annotations"]["readOnlyHint"] != json!(true);
            Some((name, if changes { format!("{title} (changes content)") } else { title.to_string() }))
        });
        return Completion::of(current, candidates);
    };

    if let Some((key, value)) = current.strip_prefix("--").and_then(|flag| flag.split_once('=')) {
        let key = key.replace('-', "_");
        return property_values(op, &key, &given, value, known).prefixed(&current[..current.len() - value.len()]);
    }
    if !current.is_empty() && !current.starts_with('-') {
        return Completion::none();
    }
    let required: Vec<&str> =
        op["input"]["required"].as_array().map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut candidates: Vec<(String, String)> = op["input"]["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !given.iter().any(|(seen, _)| seen == *key))
        .map(|(key, schema)| {
            let about = schema["description"].as_str().map(str::to_string).unwrap_or_else(|| kind(schema));
            let about = if required.contains(&key.as_str()) { format!("{about} (required)") } else { about };
            (format!("--{}", key.replace('_', "-")), about)
        })
        .collect();
    candidates.extend(REMOTE_FLAGS.iter().map(|(flag, help)| (flag.to_string(), help.to_string())));
    Completion::of(current, candidates)
}

fn kind(schema: &Value) -> String {
    match &schema["type"] {
        Value::Array(types) => types.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" or "),
        other => other.as_str().unwrap_or("value").to_string(),
    }
}

fn is_boolean(schema: &Value) -> bool {
    kind(schema).split(" or ").any(|kind| kind == "boolean")
}

fn property_values(op: &Value, key: &str, given: &[(String, Option<String>)], current: &str, known: &Known) -> Completion {
    let schema = &op["input"]["properties"][key];
    if let Some(choices) = schema["enum"].as_array() {
        return Completion::of(current, choices.iter().filter_map(Value::as_str).map(|choice| (choice.to_string(), String::new())));
    }
    if is_boolean(schema) {
        return Completion::of(current, [("true".to_string(), String::new()), ("false".to_string(), String::new())]);
    }
    let kinds = kind(schema);
    if kinds.contains("object") || kinds.contains("array") {
        return files_as_input(current);
    }
    let site = known.site.as_ref().map(|site| &site["site"]).unwrap_or(&Value::Null);
    let value_of = |name: &str| given.iter().find(|(seen, _)| seen == name).and_then(|(_, value)| value.clone());
    let items = |list: &str| -> Vec<(String, String)> {
        site[list]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| Some((item["handle"].as_str()?.to_string(), item["title"].as_str().unwrap_or_default().to_string())))
            .collect()
    };
    let name = op["name"].as_str().unwrap_or_default();
    let candidates = match key {
        "collection" => items("collections"),
        "taxonomy" => items("taxonomies"),
        "handle" if name.ends_with("_global") => items("globals"),
        "handle" if name.ends_with("_navigation") => items("navigation"),
        "handle" => match value_of("kind").as_deref() {
            Some("collection") => items("collections"),
            Some("taxonomy") => items("taxonomies"),
            Some("global") => items("globals"),
            Some("navigation") => items("navigation"),
            _ => [items("collections"), items("taxonomies"), items("globals"), items("navigation")].concat(),
        },
        "blueprint" => {
            let collection = value_of("collection");
            site["collections"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| collection.as_deref().is_none_or(|wanted| item["handle"] == json!(wanted)))
                .flat_map(|item| {
                    let owner = item["handle"].as_str().unwrap_or_default().to_string();
                    item["blueprints"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(move |blueprint| Some((blueprint.as_str()?.to_string(), format!("a {owner} blueprint"))))
                })
                .collect()
        }
        "locale" => site["locales"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|locale| {
                let code = locale["code"].as_str()?.to_string();
                Some((code, if locale["default"] == json!(true) { "the default".to_string() } else { String::new() }))
            })
            .collect(),
        _ => Vec::new(),
    };
    Completion::of(current, candidates)
}

fn files_as_input(current: &str) -> Completion {
    let Some(path) = current.strip_prefix('@') else {
        return Completion::of(
            current,
            [
                ("-".to_string(), "read it from standard input".to_string()),
                ("@".to_string(), "read it from a file: @change.json".to_string()),
            ],
        );
    };
    let (dir, stem) = match path.rfind('/') {
        Some(slash) => (&path[..=slash], &path[slash + 1..]),
        None => ("", path),
    };
    let entries = fs::read_dir(if dir.is_empty() { Path::new(".") } else { Path::new(dir) }).into_iter().flatten().flatten();
    let candidates = entries.filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && !stem.starts_with('.') {
            return None;
        }
        let slash = if entry.path().is_dir() { "/" } else { "" };
        Some((format!("@{dir}{name}{slash}"), String::new()))
    });
    Completion::of(current, candidates.collect::<Vec<_>>())
}

// Completion reads this instead of asking the site, so pressing Tab never waits on the network or touches the keychain.
fn cache_path(key: &str) -> Option<PathBuf> {
    let digest = Sha256::digest(key.as_bytes());
    let name: String = digest.iter().take(12).map(|byte| format!("{byte:02x}")).collect();
    Some(config::dir().ok()?.join("completion").join(format!("{name}.json")))
}

pub fn token_key(origin: &str) -> String {
    format!("{origin}#token")
}

fn cached(key: &str) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(cache_path(key)?).ok()?).ok()
}

const SITE_FRESH_FOR: i64 = 3600;

pub fn remember(session: &mut Session, catalogue: &Value) {
    let Some(path) = cache_path(&session.key()) else { return };
    let mut entry = cached(&session.key()).unwrap_or_else(|| json!({}));
    entry["operations"] = catalogue["data"].clone();
    let stale = entry["site_at"].as_i64().is_none_or(|at| crate::store::now() - at > SITE_FRESH_FOR);
    let describable = catalogue["data"].as_array().is_some_and(|ops| ops.iter().any(|op| op["name"] == json!("describe_site")));
    if stale
        && describable
        && let Ok(site) = session.call("describe_site", &json!({}))
    {
        entry["site"] = site["data"].clone();
        entry["site_at"] = json!(crate::store::now());
    }
    let Some(dir) = path.parent() else { return };
    if config::private_dir(dir).is_ok() && fs::write(&path, entry.to_string()).is_ok() {
        let _ = config::restrict(&path);
    }
}

pub fn remember_site(key: &str, site: &Value) {
    let (Some(path), Some(mut entry)) = (cache_path(key), cached(key)) else { return };
    entry["site"] = site.clone();
    entry["site_at"] = json!(crate::store::now());
    if fs::write(&path, entry.to_string()).is_ok() {
        let _ = config::restrict(&path);
    }
}

pub fn forget(key: &str) {
    if let Some(path) = cache_path(key) {
        let _ = fs::remove_file(path);
    }
}

// Bash hands over the whole line; this finds the words of the command being completed.
pub fn split_line(line: &str) -> (Vec<String>, String) {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(open), _) if ch == open => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            (Some(_), _) => word.push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    started = true;
                }
            }
            (None, ';' | '&' | '|' | '(' | ')') => {
                words.clear();
                word.clear();
                started = false;
            }
            (None, ch) if ch.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, ch) => {
                word.push(ch);
                started = true;
            }
        }
    }
    let current = if started || quote.is_some() { word } else { String::new() };
    let command = words.iter().position(|word| !word.contains('=') || word.starts_with('-')).unwrap_or(words.len());
    (words.split_off(command), current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn root() -> Command {
        let mut command = crate::Cli::command();
        command.build();
        command
    }

    fn known() -> Known {
        let account = |host: &str, email: &str, default: bool| Account {
            host: host.into(),
            email: email.into(),
            name: host.into(),
            access: "draft".into(),
            default,
        };
        Known {
            accounts: vec![
                account("tidewater.example", "ada@tidewater.example", true),
                account("tidewater.example", "bo@tidewater.example", false),
                account("harbour.example", "cy@harbour.example", false),
            ],
            tasks: vec![
                ("check".into(), "Check the schema".into()),
                ("schema:show".into(), "Show the schema".into()),
                ("schema:types".into(), "Write the types".into()),
                ("content:export".into(), "Export content".into()),
            ],
            site: Some(json!({
                "operations": [
                    { "name": "list_entries", "title": "List entries", "annotations": { "readOnlyHint": true },
                      "input": { "properties": { "collection": { "type": "string" }, "status": { "type": "string", "enum": ["draft", "published"] },
                                                 "locale": { "type": "string" } }, "required": ["collection"] } },
                    { "name": "create_entry", "title": "Create an entry", "annotations": { "readOnlyHint": false },
                      "input": { "properties": { "collection": { "type": "string" }, "blueprint": { "type": "string" },
                                                 "data": { "type": "object" }, "dry_run": { "type": "boolean" } } } },
                    { "name": "get_global", "title": "Read a global set", "annotations": { "readOnlyHint": true },
                      "input": { "properties": { "handle": { "type": "string" } } } },
                    { "name": "describe_schema", "title": "Describe fields", "annotations": { "readOnlyHint": true },
                      "input": { "properties": { "kind": { "type": "string", "enum": ["collection", "global"] }, "handle": { "type": "string" } } } }
                ],
                "site": {
                    "locales": [ { "code": "en", "default": true }, { "code": "fr", "default": false } ],
                    "collections": [ { "handle": "posts", "title": "Posts", "blueprints": ["post", "video"] },
                                     { "handle": "pages", "title": "Pages", "blueprints": ["page"] } ],
                    "taxonomies": [ { "handle": "topics", "title": "Topics" } ],
                    "globals": [ { "handle": "company", "title": "Company" } ],
                    "navigation": [ { "handle": "main", "title": "Main menu" } ]
                }
            })),
        }
    }

    fn values(line: &str) -> Vec<String> {
        complete_line(line).candidates.into_iter().map(|(value, _)| value).collect()
    }

    fn complete_line(line: &str) -> Completion {
        let (words, current) = split_line(line);
        complete(&root(), &words[1..], &current, &known())
    }

    #[test]
    fn the_first_word_offers_the_commands_and_this_sites_tasks_together() {
        let offered = values("nibble ");
        for word in ["auth", "remote", "completion", "check", "schema", "content"] {
            assert!(offered.contains(&word.to_string()), "{word} missing from {offered:?}");
        }
        assert!(!offered.contains(&"__complete".to_string()), "the completion command itself stays hidden");
        assert_eq!(values("nibble sc"), ["schema"]);
    }

    #[test]
    fn a_task_group_offers_its_tasks_and_a_whole_task_hands_over_to_files() {
        assert_eq!(values("nibble schema "), ["show", "types"]);
        assert_eq!(complete_line("nibble schema show ").fallback, Fallback::Files);
    }

    #[test]
    fn sites_are_offered_by_host_unless_one_host_has_several_accounts() {
        assert_eq!(
            values("nibble auth switch "),
            ["tidewater.example:ada@tidewater.example", "tidewater.example:bo@tidewater.example", "harbour.example"]
        );
        assert_eq!(values("nibble --site h"), ["harbour.example"]);
        assert_eq!(values("nibble doctor --site=h"), ["--site=harbour.example"]);
        assert_eq!(values("nibble auth login "), ["harbour.example", "tidewater.example"]);
    }

    #[test]
    fn fixed_choices_come_from_the_command_itself() {
        assert_eq!(values("nibble mcp install --client c"), ["claude-code", "codex", "cursor", "claude", "chatgpt"]);
        assert_eq!(values("nibble completion "), ["bash", "zsh", "fish", "powershell"]);
        assert_eq!(complete_line("nibble skill install --dir ").fallback, Fallback::Dirs);
    }

    #[test]
    fn flags_already_given_are_not_offered_again() {
        let offered = values("nibble auth login example.com --device --");
        assert!(offered.contains(&"--no-browser".to_string()));
        assert!(!offered.contains(&"--device".to_string()));
    }

    #[test]
    fn remote_offers_this_connections_operations_with_what_they_change() {
        let offered = complete_line("nibble remote ").candidates;
        assert!(offered.contains(&("create-entry".to_string(), "Create an entry (changes content)".to_string())));
        assert_eq!(values("nibble remote list_"), ["list_entries"], "underscores typed, underscores offered");
    }

    #[test]
    fn an_operations_flags_come_from_its_input_and_skip_what_is_given() {
        let offered = values("nibble remote list-entries --collection posts ");
        assert!(offered.contains(&"--status".to_string()) && offered.contains(&"--locale".to_string()));
        assert!(!offered.contains(&"--collection".to_string()));
        assert!(complete_line("nibble remote list-entries --").candidates.contains(&("--collection".into(), "string (required)".into())));
    }

    #[test]
    fn an_operations_values_come_from_the_site_and_follow_the_other_arguments() {
        assert_eq!(values("nibble remote list-entries --collection "), ["posts", "pages"]);
        assert_eq!(values("nibble remote list-entries --status "), ["draft", "published"]);
        assert_eq!(values("nibble remote list-entries --locale "), ["en", "fr"]);
        assert_eq!(values("nibble remote create-entry --collection posts --blueprint "), ["post", "video"], "only that collection's");
        assert_eq!(values("nibble remote get-global --handle "), ["company"]);
        assert_eq!(values("nibble remote describe-schema --kind global --handle "), ["company"]);
        assert_eq!(values("nibble remote list-entries --collection=p"), ["--collection=posts", "--collection=pages"]);
        assert_eq!(
            values("nibble remote create-entry --dry-run "),
            ["--collection", "--blueprint", "--data", "--help", "--json", "--site", "--pick"]
        );
    }

    #[test]
    fn structured_input_offers_standard_input_or_a_file() {
        assert_eq!(values("nibble remote create-entry --data "), ["-", "@"]);
        let dir = std::env::temp_dir().join("nibble-complete-test");
        fs::create_dir_all(dir.join("drafts")).unwrap();
        fs::write(dir.join("change.json"), "{}").unwrap();
        let typed = format!("@{}/c", dir.display());
        let offered = values(&format!("nibble remote create-entry --data {typed}"));
        assert_eq!(offered, [format!("@{}/change.json", dir.display())]);
    }

    #[test]
    fn remote_offers_nothing_it_would_have_to_guess() {
        let mut empty = known();
        empty.site = None;
        let (words, current) = split_line("nibble remote ");
        assert!(complete(&root(), &words[1..], &current, &empty).candidates.is_empty(), "no cache yet means no operations");
        assert!(values("nibble remote no-such-operation --").is_empty());
    }

    #[test]
    fn the_line_is_read_like_a_shell_reads_it() {
        assert_eq!(
            split_line("cd site && NIBBLE_SITE=x nibble remote 'list entries' --site=a:b"),
            (vec!["nibble".into(), "remote".into(), "list entries".into()], "--site=a:b".into())
        );
        assert_eq!(split_line("nibble auth "), (vec!["nibble".into(), "auth".into()], String::new()));
    }

    #[test]
    fn bash_gets_only_the_part_after_its_last_word_break() {
        let current = "--site=tidewater.example:b";
        let completion = complete_line(&format!("nibble doctor {current}"));
        assert_eq!(
            completion.render(Some("b"), current),
            ":none\nbo@tidewater.example\ttidewater.example as bo@tidewater.example (draft access)\n"
        );
        assert!(
            completion.render(None, current).contains("--site=tidewater.example:bo@tidewater.example\t"),
            "other shells get whole words"
        );
    }
}
