use crate::api::Session;
use crate::config::host;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER: &str = ".nibble-skill.json";

fn home() -> Result<PathBuf> {
    dirs::home_dir().context("this system has no home folder")
}

pub fn server_name(origin: &str) -> String {
    let slug: String = host(origin).chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    format!("nibble-{}", slug.trim_matches('-'))
}

pub fn install_mcp(client: &str, origin: &str, issuer: &str, name: Option<String>) -> Result<String> {
    let url = format!("{issuer}/mcp");
    let name = name.unwrap_or_else(|| server_name(origin));
    match client {
        "claude-code" => {
            let args = ["mcp", "add", "--transport", "http", "--scope", "user", name.as_str(), url.as_str()];
            if which("claude") {
                let status = Command::new("claude").args(args).status()?;
                if !status.success() {
                    bail!("`claude {}` failed", args.join(" "));
                }
                Ok(format!("Added {name} to Claude Code. In Claude Code, run /mcp to sign in."))
            } else {
                Ok(format!("Claude Code isn't on this computer's PATH. Once it is, run:\n  claude {}", args.join(" ")))
            }
        }
        "codex" => {
            let path = home()?.join(".codex/config.toml");
            let mut table: toml::Table = if path.exists() { fs::read_to_string(&path)?.parse()? } else { toml::Table::new() };
            let servers = table.entry("mcp_servers").or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let servers = servers.as_table_mut().context("mcp_servers in Codex's config isn't a table")?;
            let mut server = toml::Table::new();
            server.insert("url".into(), toml::Value::String(url.clone()));
            servers.insert(name.clone(), toml::Value::Table(server));
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, toml::to_string_pretty(&table)?)?;
            Ok(format!("Added {name} to {}. Now run: codex mcp login {name}", path.display()))
        }
        "cursor" => {
            let path = home()?.join(".cursor/mcp.json");
            let mut config: Value = if path.exists() { serde_json::from_str(&fs::read_to_string(&path)?)? } else { json!({}) };
            if !config["mcpServers"].is_object() {
                config["mcpServers"] = json!({});
            }
            config["mcpServers"][&name] = json!({ "url": url });
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, serde_json::to_string_pretty(&config)?)?;
            Ok(format!("Added {name} to {}. Cursor asks you to sign in the first time it's used.", path.display()))
        }
        "claude" | "claude-desktop" | "chatgpt" => Ok(format!(
            "In {}, open Settings → Connectors → Add custom connector, and paste:\n  {url}\nIt signs you in on the site. The site must be reachable from the internet for hosted apps.",
            if client == "chatgpt" { "ChatGPT" } else { "Claude" }
        )),
        other => bail!("unknown client {other}; use claude-code, codex, cursor, claude or chatgpt"),
    }
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

pub fn skills_dir(client: Option<&str>, dir: Option<PathBuf>) -> Result<PathBuf> {
    match (client, dir) {
        (_, Some(dir)) => Ok(dir),
        (Some("claude") | Some("claude-code"), None) => Ok(home()?.join(".claude/skills")),
        (Some("codex"), None) => Ok(home()?.join(".codex/skills")),
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

pub fn installed_skills() -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for base in [".claude/skills", ".codex/skills"] {
        let Ok(home) = home() else { continue };
        let Ok(entries) = fs::read_dir(home.join(base)) else { continue };
        for entry in entries.flatten() {
            let marker = entry.path().join(MARKER);
            if let Some(origin) = fs::read_to_string(&marker)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|value| value["origin"].as_str().map(str::to_string))
            {
                found.push((entry.path(), origin));
            }
        }
    }
    found
}
