use crate::api::Session;
use crate::config::host;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER: &str = ".nibble-skill.json";

pub fn home() -> Result<PathBuf> {
    dirs::home_dir().context("this system has no home folder")
}

pub fn server_name(origin: &str) -> String {
    let slug: String = host(origin).chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let slug = slug.trim_matches('-');
    if slug == "nibble" || slug.starts_with("nibble-") { slug.to_string() } else { format!("nibble-{slug}") }
}

pub struct Added {
    pub done: String,
    pub next: String,
}

impl Added {
    fn new(done: impl Into<String>, next: impl Into<String>) -> Self {
        Added { done: done.into(), next: next.into() }
    }

    pub fn line(&self) -> String {
        if self.next.is_empty() { self.done.clone() } else { format!("{}. {}", self.done, self.next) }
    }
}

pub fn claude_args(verb: &str, name: &str, url: &str, project: bool) -> Vec<String> {
    let scope = if project { "project" } else { "user" };
    let mut args: Vec<&str> = vec!["mcp", verb];
    if verb == "add" {
        args.extend(["--transport", "http"]);
    }
    args.extend(["--scope", scope, name]);
    if verb == "add" {
        args.push(url);
    }
    args.into_iter().map(str::to_string).collect()
}

pub fn mcp_file(client: &str, root: &Path) -> Option<PathBuf> {
    match client {
        "codex" => Some(root.join(".codex/config.toml")),
        "cursor" => Some(root.join(".cursor/mcp.json")),
        _ => None,
    }
}

pub fn install_mcp(client: &str, origin: &str, issuer: &str, name: Option<String>, project: &Path) -> Result<Added> {
    let url = format!("{issuer}/mcp");
    let name = name.unwrap_or_else(|| server_name(origin));
    let root = project;
    match client {
        "claude-code" => {
            let args = claude_args("add", &name, &url, true);
            if which("claude") {
                let mut command = Command::new("claude");
                command.args(&args);
                command.current_dir(project);
                if !command.status()?.success() {
                    bail!("`claude {}` failed", args.join(" "));
                }
                Ok(Added::new(format!("Added {name} to Claude Code"), "In Claude Code, run /mcp to sign in"))
            } else {
                Ok(Added::new("Claude Code isn't on this computer's PATH", format!("Once it is, run: claude {}", args.join(" "))))
            }
        }
        "codex" => {
            let path = mcp_file(client, root).unwrap();
            let mut table: toml::Table = if path.exists() { fs::read_to_string(&path)?.parse()? } else { toml::Table::new() };
            let servers = table.entry("mcp_servers").or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let servers = servers.as_table_mut().context("mcp_servers in Codex's config isn't a table")?;
            let mut server = toml::Table::new();
            server.insert("url".into(), toml::Value::String(url.clone()));
            servers.insert(name.clone(), toml::Value::Table(server));
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, toml::to_string_pretty(&table)?)?;
            Ok(Added::new(
                format!("Added {name} to {}", path.display()),
                format!("Now run: codex mcp login {name}. Codex reads a project's config only once you trust the project"),
            ))
        }
        "cursor" => {
            let path = mcp_file(client, root).unwrap();
            let mut config: Value = if path.exists() { serde_json::from_str(&fs::read_to_string(&path)?)? } else { json!({}) };
            if !config["mcpServers"].is_object() {
                config["mcpServers"] = json!({});
            }
            config["mcpServers"][&name] = json!({ "url": url });
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, serde_json::to_string_pretty(&config)?)?;
            Ok(Added::new(format!("Added {name} to {}", path.display()), "Cursor asks you to sign in the first time it's used"))
        }
        "claude" | "claude-desktop" | "chatgpt" => Ok(Added::new(
            format!("{} can't be set up from here", if client == "chatgpt" { "ChatGPT" } else { "Claude" }),
            format!(
                "Open Settings → Connectors → Add custom connector, and paste {url}. It signs you in on the site, which must be reachable from the internet for hosted apps"
            ),
        )),
        other => bail!("unknown client {other}; use claude-code, codex, cursor, claude or chatgpt"),
    }
}

fn is_ours(key: &str, entry_url: Option<&str>, name: &str, url: &str) -> bool {
    key == name || entry_url == Some(url)
}

pub fn remove_mcp(client: &str, origin: &str, issuer: &str, root: &Path, project: bool) -> Result<Option<String>> {
    let url = format!("{issuer}/mcp");
    let name = server_name(origin);
    match client {
        "claude-code" => {
            if !which("claude") {
                return Ok(None);
            }
            let mut command = Command::new("claude");
            command.args(claude_args("remove", &name, &url, project)).current_dir(root);
            let removed = command.output().is_ok_and(|output| output.status.success());
            Ok(removed.then(|| format!("Removed {name} from Claude Code ({} scope)", if project { "project" } else { "user" })))
        }
        "codex" => {
            let path = mcp_file(client, root).unwrap();
            let Ok(text) = fs::read_to_string(&path) else { return Ok(None) };
            let mut table: toml::Table = text.parse()?;
            let Some(servers) = table.get_mut("mcp_servers").and_then(toml::Value::as_table_mut) else { return Ok(None) };
            let before = servers.len();
            servers.retain(|key, value| !is_ours(key, value.get("url").and_then(toml::Value::as_str), &name, &url));
            if servers.len() == before {
                return Ok(None);
            }
            fs::write(&path, toml::to_string_pretty(&table)?)?;
            Ok(Some(format!("Removed the site from {}", path.display())))
        }
        "cursor" => {
            let path = mcp_file(client, root).unwrap();
            let Ok(text) = fs::read_to_string(&path) else { return Ok(None) };
            let mut config: Value = serde_json::from_str(&text)?;
            let Some(servers) = config["mcpServers"].as_object_mut() else { return Ok(None) };
            let before = servers.len();
            servers.retain(|key, value| !is_ours(key, value["url"].as_str(), &name, &url));
            if servers.len() == before {
                return Ok(None);
            }
            fs::write(&path, serde_json::to_string_pretty(&config)?)?;
            Ok(Some(format!("Removed the site from {}", path.display())))
        }
        _ => Ok(None),
    }
}

pub fn remove_skills(root: &Path, origin: &str) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for base in [".claude/skills", ".codex/skills"] {
        let Ok(entries) = fs::read_dir(root.join(base)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if marker_origin(&path).as_deref() == Some(origin) && fs::remove_dir_all(&path).is_ok() {
                removed.push(path);
            }
        }
    }
    removed
}

pub struct Plan {
    pub client: String,
    pub skills: Option<PathBuf>,
}

pub fn plan(detected: &[&str], only: &[String], root: &Path) -> Vec<Plan> {
    let clients: Vec<String> = if only.is_empty() { detected.iter().map(|client| client.to_string()).collect() } else { only.to_vec() };
    clients
        .into_iter()
        .map(|client| {
            let skills = skill_client(&client).map(|skill| skills_in(skill, root));
            Plan { client, skills }
        })
        .collect()
}

fn skills_in(skill_client: &str, root: &Path) -> PathBuf {
    root.join(if skill_client == "codex" { ".codex/skills" } else { ".claude/skills" })
}

fn on_path(dirs: &[PathBuf], program: &str) -> bool {
    dirs.iter().any(|dir| dir.join(program).is_file())
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|paths| std::env::split_paths(&paths).collect()).unwrap_or_default()
}

fn which(program: &str) -> bool {
    on_path(&path_dirs(), program)
}

pub fn detect_clients(path: &[PathBuf], home: &Path) -> Vec<&'static str> {
    let mut found = Vec::new();
    if on_path(path, "claude") {
        found.push("claude-code");
    }
    if on_path(path, "codex") {
        found.push("codex");
    }
    if on_path(path, "cursor") || home.join(".cursor").is_dir() {
        found.push("cursor");
    }
    found
}

pub fn installed_clients() -> Vec<&'static str> {
    home().map(|home| detect_clients(&path_dirs(), &home)).unwrap_or_default()
}

pub fn client_label(client: &str) -> &'static str {
    match client {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "claude" => "Claude",
        "chatgpt" => "ChatGPT",
        _ => "Cursor",
    }
}

pub const CHOICES: [&str; 5] = ["claude-code", "codex", "cursor", "claude", "chatgpt"];

pub fn parse_choice(answer: &str, found: &[&str]) -> Result<Vec<String>> {
    let answer = answer.trim().to_lowercase();
    if answer.is_empty() {
        return Ok(found.iter().map(|client| client.to_string()).collect());
    }
    if answer == "none" || answer == "0" {
        return Ok(Vec::new());
    }
    let mut chosen: Vec<String> = Vec::new();
    for part in answer.split(',') {
        let number = part.trim().parse::<usize>().ok().filter(|number| (1..=CHOICES.len()).contains(number));
        let Some(number) = number else { bail!("`{}` isn't one of the numbers listed", part.trim()) };
        let client = CHOICES[number - 1].to_string();
        if !chosen.contains(&client) {
            chosen.push(client);
        }
    }
    Ok(chosen)
}

pub fn skill_client(client: &str) -> Option<&'static str> {
    match client {
        "claude-code" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    }
}

pub fn skills_dir(client: Option<&str>, dir: Option<PathBuf>, project: &Path) -> Result<PathBuf> {
    match (client, dir) {
        (_, Some(dir)) => Ok(dir),
        (Some("claude") | Some("claude-code"), None) => Ok(skills_in("claude", project)),
        (Some("codex"), None) => Ok(skills_in("codex", project)),
        (Some(other), None) => bail!("unknown client {other}; use claude or codex, or --dir"),
        (None, None) => bail!("say where: --client claude, --client codex, or --dir PATH"),
    }
}

pub fn install_skill(session: &mut Session, dir: &Path) -> Result<(PathBuf, bool)> {
    let guide = session.call("get_guide", &json!({}))?;
    let data = &guide["data"];
    let name = data["name"].as_str().context("the site's guide has no name")?;
    let fingerprint = data["fingerprint"].as_str().unwrap_or_default();
    let folder = dir.join(name);
    let marker = folder.join(MARKER);
    let previous: Option<Value> = fs::read_to_string(&marker).ok().and_then(|text| serde_json::from_str(&text).ok());
    if previous.as_ref().and_then(|value| value["fingerprint"].as_str()) == Some(fingerprint) {
        return Ok((folder, false));
    }

    let cli_note = format!(
        "\n\n## Without the MCP tools\n\nIf the site's tools aren't connected, run the same operations from a terminal: \
         `nibble remote <operation> --site {} --<argument> <value>`. `nibble remote` lists them.\n",
        host(&session.origin)
    );
    let skill = format!(
        "---\nname: {name}\ndescription: {}\n---\n\n{}{cli_note}",
        serde_json::to_string(data["description"].as_str().unwrap_or_default())?,
        data["markdown"].as_str().unwrap_or_default()
    );
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("SKILL.md"), skill)?;
    fs::write(&marker, json!({ "origin": session.origin, "fingerprint": fingerprint }).to_string())?;
    Ok((folder, true))
}

fn marker_origin(folder: &Path) -> Option<String> {
    let text = fs::read_to_string(folder.join(MARKER)).ok()?;
    serde_json::from_str::<Value>(&text).ok()?["origin"].as_str().map(str::to_string)
}

pub fn installed_skills(project: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for base in [".claude/skills", ".codex/skills"] {
        let Ok(entries) = fs::read_dir(project.join(base)) else { continue };
        for entry in entries.flatten() {
            if let Some(origin) = marker_origin(&entry.path()) {
                found.push((entry.path(), origin));
            }
        }
    }
    found
}

fn site_folder(start: &Path) -> Option<&Path> {
    start.ancestors().find(|dir| dir.join("config/nibble.yml").is_file() && dir.join("vendor/nibble").is_dir())
}

fn site_hosts(folder: &Path) -> Vec<String> {
    let text = fs::read_to_string(folder.join("config/nibble.yml")).unwrap_or_default();
    text.lines()
        .filter_map(|line| line.strip_prefix("  url:"))
        .map(|value| host(value.trim().trim_matches(|c| c == '"' || c == '\'')))
        .filter(|value| !value.is_empty() && !local_host(value))
        .collect()
}

fn local_host(host: &str) -> bool {
    let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
    name == "localhost"
        || name.starts_with("127.")
        || name == "[::1]"
        || [".localhost", ".test", ".local"].iter().any(|end| name.ends_with(end))
}

pub fn guard_project(start: &Path, origin: &str) -> Result<()> {
    let Some(folder) = site_folder(start) else { return Ok(()) };
    let target = host(origin);
    let hosts = site_hosts(folder);
    if hosts.contains(&target) {
        return Ok(());
    }
    match hosts.first() {
        Some(own) => bail!(
            "{} is the project for {own}; connect {target} from its own folder or a folder that isn't a Nibble site",
            folder.display()
        ),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(dir: &Path, name: &str) {
        fs::write(dir.join(name), "").unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nibble-agents-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn apps_are_found_by_their_command_on_the_path() {
        let (bin, home) = (scratch("bin"), scratch("home"));
        tool(&bin, "claude");
        tool(&bin, "codex");
        assert_eq!(detect_clients(std::slice::from_ref(&bin), &home), ["claude-code", "codex"]);
        assert_eq!(detect_clients(&[scratch("empty")], &home), Vec::<&str>::new(), "nothing installed offers nothing");
    }

    #[test]
    fn cursor_is_found_by_its_command_or_its_folder_at_home() {
        let (bin, home) = (scratch("cursor-bin"), scratch("cursor-home"));
        assert!(detect_clients(std::slice::from_ref(&bin), &home).is_empty());
        fs::create_dir(home.join(".cursor")).unwrap();
        assert_eq!(detect_clients(std::slice::from_ref(&bin), &home), ["cursor"]);
        let (bin, home) = (scratch("cursor-bin2"), scratch("cursor-home2"));
        tool(&bin, "cursor");
        assert_eq!(detect_clients(&[bin], &home), ["cursor"]);
    }

    #[test]
    fn connect_plans_every_detected_app_unless_told_which() {
        let root = scratch("plan");
        let detected = ["claude-code", "codex", "cursor"];
        let all = plan(&detected, &[], &root);
        assert_eq!(all.iter().map(|plan| plan.client.as_str()).collect::<Vec<_>>(), detected);
        assert_eq!(all[0].skills, Some(root.join(".claude/skills")));
        assert_eq!(all[1].skills, Some(root.join(".codex/skills")));
        assert_eq!(all[2].skills, None, "Cursor has no skills to write");

        let chosen = plan(&detected, &["cursor".into(), "chatgpt".into()], &root);
        assert_eq!(chosen.iter().map(|plan| plan.client.as_str()).collect::<Vec<_>>(), ["cursor", "chatgpt"]);
        assert!(plan(&[], &[], &root).is_empty(), "nothing found and nothing asked for plans nothing");
    }

    #[test]
    fn the_answer_chooses_the_apps_and_enter_takes_those_found() {
        let found = ["claude-code", "cursor"];
        assert_eq!(parse_choice("", &found).unwrap(), ["claude-code", "cursor"]);
        assert_eq!(parse_choice("  \n", &found).unwrap(), ["claude-code", "cursor"]);
        assert!(parse_choice("", &[]).unwrap().is_empty(), "nothing found means nothing by default");
        assert_eq!(parse_choice("1,3", &found).unwrap(), ["claude-code", "cursor"]);
        assert_eq!(parse_choice("2, 5", &found).unwrap(), ["codex", "chatgpt"]);
        assert_eq!(parse_choice("2,2", &found).unwrap(), ["codex"]);
        assert!(parse_choice("none", &found).unwrap().is_empty());
        assert!(parse_choice("0", &found).unwrap().is_empty());
    }

    #[test]
    fn a_server_name_doesnt_repeat_nibble() {
        assert_eq!(server_name("https://nibble.ink"), "nibble-ink");
        assert_eq!(server_name("https://tidewater.example"), "nibble-tidewater-example");
        assert_eq!(server_name("http://localhost:3100"), "nibble-localhost-3100");
        assert_eq!(server_name("https://nibbler.shop"), "nibble-nibbler-shop");
    }

    #[test]
    fn a_number_outside_the_list_is_refused() {
        for answer in ["6", "1,9", "codex", "1,,2", "-1"] {
            assert!(parse_choice(answer, &["codex"]).is_err(), "{answer}");
        }
    }

    #[test]
    fn claude_is_added_and_removed_in_the_scope_asked_for() {
        assert_eq!(
            claude_args("add", "nibble-x", "https://x/mcp", true),
            ["mcp", "add", "--transport", "http", "--scope", "project", "nibble-x", "https://x/mcp"]
        );
        assert_eq!(claude_args("remove", "nibble-x", "https://x/mcp", false), ["mcp", "remove", "--scope", "user", "nibble-x"]);
    }

    #[test]
    fn a_project_connection_is_written_inside_the_project_and_can_be_repeated() {
        let project = scratch("project");
        for _ in 0..2 {
            install_mcp("codex", "https://a.example", "https://a.example", None, &project).unwrap();
            install_mcp("cursor", "https://a.example", "https://a.example", None, &project).unwrap();
        }
        let codex: toml::Table = fs::read_to_string(project.join(".codex/config.toml")).unwrap().parse().unwrap();
        assert_eq!(codex["mcp_servers"]["nibble-a-example"]["url"].as_str(), Some("https://a.example/mcp"));
        assert_eq!(codex["mcp_servers"].as_table().unwrap().len(), 1, "connecting again replaces the entry");
        let cursor: Value = serde_json::from_str(&fs::read_to_string(project.join(".cursor/mcp.json")).unwrap()).unwrap();
        assert_eq!(cursor["mcpServers"]["nibble-a-example"]["url"], "https://a.example/mcp");
    }

    #[test]
    fn disconnecting_a_site_leaves_other_sites_and_other_servers_alone() {
        let root = scratch("disconnect");
        install_mcp("codex", "https://a.example", "https://a.example", None, &root).unwrap();
        install_mcp("codex", "https://b.example", "https://b.example", None, &root).unwrap();
        install_mcp("cursor", "https://a.example", "https://a.example", None, &root).unwrap();
        install_mcp("cursor", "https://b.example", "https://b.example", None, &root).unwrap();
        let custom_name = Some("work-cms".to_string());
        install_mcp("cursor", "https://a.example", "https://a.example", custom_name, &root).unwrap();
        let cursor_path = root.join(".cursor/mcp.json");
        let mut cursor: Value = serde_json::from_str(&fs::read_to_string(&cursor_path).unwrap()).unwrap();
        cursor["mcpServers"]["figma"] = json!({ "url": "https://figma.example/mcp" });
        fs::write(&cursor_path, cursor.to_string()).unwrap();

        assert!(remove_mcp("codex", "https://a.example", "https://a.example", &root, true).unwrap().is_some());
        assert!(remove_mcp("cursor", "https://a.example", "https://a.example", &root, true).unwrap().is_some());
        assert!(remove_mcp("cursor", "https://a.example", "https://a.example", &root, true).unwrap().is_none(), "nothing left to remove");

        let codex: toml::Table = fs::read_to_string(root.join(".codex/config.toml")).unwrap().parse().unwrap();
        assert_eq!(codex["mcp_servers"].as_table().unwrap().keys().collect::<Vec<_>>(), ["nibble-b-example"]);
        let cursor: Value = serde_json::from_str(&fs::read_to_string(cursor_path).unwrap()).unwrap();
        let mut kept: Vec<_> = cursor["mcpServers"].as_object().unwrap().keys().cloned().collect();
        kept.sort();
        assert_eq!(kept, ["figma", "nibble-b-example"], "an entry under another name but this site's address goes too");
    }

    #[test]
    fn disconnecting_removes_only_the_sites_skill_folders() {
        let root = scratch("skills");
        for (dir, name, origin) in [
            (".claude/skills", "nibble-a", "https://a.example"),
            (".codex/skills", "nibble-a", "https://a.example"),
            (".claude/skills", "nibble-b", "https://b.example"),
        ] {
            let folder = root.join(dir).join(name);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(MARKER), json!({ "origin": origin }).to_string()).unwrap();
        }
        fs::create_dir_all(root.join(".claude/skills/hand-written")).unwrap();

        assert_eq!(remove_skills(&root, "https://a.example").len(), 2);

        assert!(!root.join(".claude/skills/nibble-a").exists());
        assert!(!root.join(".codex/skills/nibble-a").exists());
        assert!(root.join(".claude/skills/nibble-b").exists(), "another site's skill stays");
        assert!(root.join(".claude/skills/hand-written").exists(), "a skill nibble didn't write stays");
    }

    fn site(name: &str, url: &str) -> PathBuf {
        let dir = scratch(name);
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::create_dir_all(dir.join("vendor/nibble")).unwrap();
        fs::write(
            dir.join("config/nibble.yml"),
            format!("default: &default\n  theme: crumbs\n  url: {url}\n  locales:\n    - code: en\n      url_prefix: \"\"\n"),
        )
        .unwrap();
        fs::create_dir_all(dir.join("site/themes")).unwrap();
        dir
    }

    #[test]
    fn another_sites_project_is_refused_and_its_own_is_allowed() {
        let project = site("guard", "https://a.example");
        let inside = project.join("site/themes");
        assert!(guard_project(&inside, "https://a.example").is_ok(), "a site's own folder, from below it too");
        let error = guard_project(&inside, "https://b.example").unwrap_err().to_string();
        assert!(error.contains("is the project for a.example; connect b.example from its own folder"), "{error}");
        assert!(guard_project(&scratch("plain"), "https://b.example").is_ok(), "a folder that isn't a Nibble site takes any site");
    }

    #[test]
    fn a_site_folder_naming_only_local_addresses_takes_any_site() {
        let project = site("guard-local", "http://localhost:3100");
        assert!(guard_project(&project, "https://nibble.ink").is_ok(), "a local address doesn't say which site this is");
        fs::write(
            project.join("config/nibble.yml"),
            "default: &default\n  url: http://localhost:3100\nproduction:\n  url: https://a.example\n",
        )
        .unwrap();
        assert!(guard_project(&project, "https://nibble.ink").is_err(), "a real address does");
        assert!(guard_project(&project, "https://a.example").is_ok());
    }

    #[test]
    fn disconnect_cleans_what_older_versions_wrote_user_wide() {
        let home = scratch("old-home");
        install_mcp("codex", "https://a.example", "https://a.example", None, &home).unwrap();
        install_mcp("cursor", "https://a.example", "https://a.example", None, &home).unwrap();
        let folder = home.join(".claude/skills/nibble-a");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(MARKER), json!({ "origin": "https://a.example" }).to_string()).unwrap();

        assert!(remove_mcp("codex", "https://a.example", "https://a.example", &home, false).unwrap().is_some());
        assert!(remove_mcp("cursor", "https://a.example", "https://a.example", &home, false).unwrap().is_some());
        assert_eq!(remove_skills(&home, "https://a.example"), [folder]);
    }

    #[test]
    fn a_connection_goes_only_into_the_project_it_is_given() {
        let (project, home) = (scratch("only-project"), scratch("untouched-home"));
        install_mcp("codex", "https://a.example", "https://a.example", None, &project).unwrap();
        assert!(project.join(".codex/config.toml").exists());
        assert!(!home.join(".codex").exists());
        let dir = skills_dir(Some("claude"), None, &project).unwrap();
        assert_eq!(dir, project.join(".claude/skills"));
        assert_eq!(skills_dir(Some("codex"), None, &project).unwrap(), project.join(".codex/skills"));
    }
}
