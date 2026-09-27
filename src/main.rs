mod agents;
mod api;
mod auth;
mod complete;
mod config;
mod http;
mod new;
mod output;
mod project;
mod remote;
mod store;
mod style;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use config::{Account, Config, Profile, Site, Storage};
use output::Output;
use serde_json::{Value, json};
use style::{BAD, CHANGE, DIM, GOOD, HEADING, NAME, STRONG, paint};

/// Nibble: start a site, run its tasks, and work on the content of the sites you're signed in to.
#[derive(Parser)]
#[command(name = "nibble", version, allow_external_subcommands = true, disable_help_subcommand = true, styles = style::HELP)]
struct Cli {
    /// Print JSON (the default when the output isn't a terminal)
    #[arg(long, global = true)]
    json: bool,
    /// The site to use, like example.com or example.com:you@example.com
    #[arg(long, global = true, env = "NIBBLE_SITE")]
    site: Option<String>,
    /// Print only this part of the result, like entries.0.title
    #[arg(long, global = true)]
    pick: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new Nibble site in a folder named after it
    New {
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
        /// Options for the installer, after --
        #[arg(last = true)]
        install: Vec<String>,
    },
    /// Sign in to sites, and choose which one commands use
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Work on a signed-in site's content: `nibble remote` lists what you can do
    Remote {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        words: Vec<String>,
    },
    /// Connect AI apps to a site over MCP
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Install the site's guide as a skill for AI agents
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// Check this computer, your sign-ins and the current site
    Doctor,
    /// Print the script that completes nibble's commands in your shell
    Completion { shell: complete::Shell },
    #[command(name = "__complete", hide = true)]
    Complete {
        #[arg(long, allow_hyphen_values = true)]
        line: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        word: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        current: Option<String>,
        #[arg(last = true, allow_hyphen_values = true)]
        words: Vec<String>,
    },
    #[command(external_subcommand)]
    Task(Vec<String>),
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Sign in to a site in your browser
    Login {
        url: String,
        /// Sign in on another device with a code, for servers without a browser
        #[arg(long)]
        device: bool,
        /// Print the address instead of opening a browser
        #[arg(long)]
        no_browser: bool,
        /// Keep the sign-in in a file only you can read, when this system has no keychain
        #[arg(long)]
        insecure_storage: bool,
    },
    /// Sign out and disconnect the CLI from the site
    Logout { profile: Option<String> },
    /// List the sites and accounts you're signed in to
    List,
    /// Use this site (and account) when none is named
    Switch { profile: String },
    /// Show which site and account commands will use
    Status,
    /// Let the .nibble.toml in this folder choose the site
    Allow,
}

#[derive(Subcommand)]
enum McpCommand {
    /// Add the site's MCP server to an AI app
    Install {
        #[arg(long, value_parser = mcp_clients())]
        client: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Print the site's MCP address
    Url,
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Write the site's guide as a skill
    Install {
        #[arg(long, value_parser = skill_clients())]
        client: Option<String>,
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
    /// Refresh every installed site skill whose site has changed
    Sync,
}

fn mcp_clients() -> clap::builder::PossibleValuesParser {
    use clap::builder::PossibleValue;
    clap::builder::PossibleValuesParser::new([
        PossibleValue::new("claude-code").help("Claude Code, added for you"),
        PossibleValue::new("codex").help("Codex, added for you"),
        PossibleValue::new("cursor").help("Cursor, added for you"),
        PossibleValue::new("claude").help("Claude's apps: prints where to paste the address"),
        PossibleValue::new("chatgpt").help("ChatGPT: prints where to paste the address"),
        PossibleValue::new("claude-desktop").hide(true),
    ])
}

fn skill_clients() -> clap::builder::PossibleValuesParser {
    use clap::builder::PossibleValue;
    clap::builder::PossibleValuesParser::new([
        PossibleValue::new("claude").help("Claude Code's skills, in ~/.claude/skills"),
        PossibleValue::new("codex").help("Codex's skills, in ~/.codex/skills"),
        PossibleValue::new("claude-code").hide(true),
    ])
}

fn main() {
    let cli = Cli::parse();
    let output = Output::new(cli.json);
    let code = match run(cli, output) {
        Ok(code) => code,
        Err(error) => {
            output.failure(&error);
            1
        }
    };
    std::process::exit(code);
}

fn run(cli: Cli, output: Output) -> Result<i32> {
    let Some(command) = cli.command else {
        let cwd = std::env::current_dir()?;
        Cli::command().print_help()?;
        if let Some(root) = project::root(&cwd) {
            anstream::println!(
                "\n{} {}: anything else runs its tasks, like `nibble check` or `nibble upgrade`.",
                paint(HEADING, "In a site"),
                paint(STRONG, root.display())
            );
        }
        return Ok(0);
    };
    match command {
        Command::New { name, version, install } => new::run(name, version, &install),
        Command::Auth { command } => auth_command(command, cli.site, output).map(|_| 0),
        Command::Remote { words } => {
            let (words, globals) = remote::globals(words);
            let output = if globals.json { Output::new(true) } else { output };
            let mut session = session(globals.site.or(cli.site))?;
            remote::run(&mut session, remote::parse(words), output, globals.pick.as_deref().or(cli.pick.as_deref())).map(|_| 0)
        }
        Command::Mcp { command } => {
            let session = session(cli.site)?;
            match command {
                McpCommand::Install { client, name } => {
                    println!("{}", agents::install_mcp(&client, &session.origin, &session.site.issuer, name)?)
                }
                McpCommand::Url => println!("{}/mcp", session.site.issuer),
            }
            Ok(0)
        }
        Command::Skill { command } => skill_command(command, cli.site).map(|_| 0),
        Command::Doctor => doctor(cli.site, output),
        Command::Completion { shell } => {
            print!("{}", complete::script(shell));
            Ok(0)
        }
        Command::Complete { line, word, current, words } => {
            let (words, current) = match line {
                Some(line) => complete::split_line(&line),
                None => (words, current.unwrap_or_default()),
            };
            let before = words.get(1..).unwrap_or_default();
            let known = complete::Known::load(before, std::env::var("NIBBLE_SITE").ok(), |site| target(site).ok());
            let mut root = Cli::command();
            root.build();
            print!("{}", complete::complete(&root, before, &current, &known).render(word.as_deref(), &current));
            Ok(0)
        }
        Command::Task(words) => {
            let cwd = std::env::current_dir()?;
            let root = project::root(&cwd).with_context(|| {
                format!("`nibble {}` isn't a command, and this isn't a Nibble site folder. `nibble --help` lists commands", words.join(" "))
            })?;
            let (task, args) = project::resolve(&root, &words)?;
            project::run(&root, &task, &args)
        }
    }
}

fn session(site: Option<String>) -> Result<api::Session> {
    if let Ok(token) = std::env::var("NIBBLE_TOKEN") {
        let origin = site.context("NIBBLE_TOKEN needs NIBBLE_SITE (or --site) set to the site's address")?;
        return api::Session::for_token(&origin, token);
    }
    api::Session::for_profile(target(site)?)
}

fn target(site: Option<String>) -> Result<Profile> {
    let config = Config::load()?;
    if let Some(reference) = site {
        return config.find(&reference);
    }
    let cwd = std::env::current_dir()?;
    if let Some((folder, reference)) = config::folder_binding(&cwd) {
        let profile = config.find(&reference)?;
        let key = folder.to_string_lossy().to_string();
        if config.allowed_folders.get(&key) != Some(&profile.key()) {
            bail!(
                "{} in {} points commands at {}. If you trust it, run `nibble auth allow` there; or name a site with --site",
                config::FOLDER_FILE,
                folder.display(),
                profile.label()
            );
        }
        return Ok(profile);
    }
    if let Some(profile) = config.default.as_deref().and_then(|key| config.by_key(key)) {
        return Ok(profile);
    }
    match config.profiles().as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!("you aren't signed in to any site; run `nibble auth login <site>`"),
        _ => bail!("you're signed in to more than one site; choose one with `nibble auth switch <site>` or --site"),
    }
}

fn auth_command(command: AuthCommand, site: Option<String>, output: Output) -> Result<()> {
    match command {
        AuthCommand::Login { url, device, no_browser, insecure_storage } => login(&url, device, !no_browser, insecure_storage, output),
        AuthCommand::Logout { profile } => {
            let profile = target(profile.or(site))?;
            if let Ok(tokens) = store::load(&profile) {
                auth::revoke(&profile.site, tokens.refresh_token.as_deref().unwrap_or(&tokens.access_token));
            }
            store::delete(&profile)?;
            complete::forget(&profile.key());
            let mut config = Config::load()?;
            config.remove(&profile);
            config.save()?;
            style::done(format!("Signed out of {}, and the site has disconnected the CLI.", paint(STRONG, profile.label())));
            Ok(())
        }
        AuthCommand::List => {
            let config = Config::load()?;
            let rows: Vec<Value> = config
                .profiles()
                .iter()
                .map(|profile| {
                    json!({ "site": config::host(&profile.origin), "name": profile.site.name, "account": profile.account.email,
                            "access": profile.account.access, "default": config.default.as_deref() == Some(profile.key().as_str()) })
                })
                .collect();
            if output.json {
                output.success(&json!(rows), None, None);
            } else if rows.is_empty() {
                anstream::println!("Not signed in anywhere. {}", paint(DIM, "`nibble auth login <site>` signs you in."));
            } else {
                for row in rows {
                    let marker = if row["default"] == json!(true) { paint(GOOD, "●") } else { " ".into() };
                    anstream::println!(
                        "{marker} {}{}  {}",
                        paint(STRONG, row["site"].as_str().unwrap()),
                        paint(NAME, format!(":{}", row["account"].as_str().unwrap())),
                        paint(DIM, format!("{}, {} access", row["name"].as_str().unwrap(), row["access"].as_str().unwrap()))
                    );
                }
                anstream::println!(
                    "\n{} {}",
                    paint(GOOD, "●"),
                    paint(DIM, "is used when you don't name a site. `nibble auth switch` changes it.")
                );
            }
            Ok(())
        }
        AuthCommand::Switch { profile } => {
            let mut config = Config::load()?;
            let found = config.find(&profile)?;
            config.default = Some(found.key());
            config.save()?;
            style::done(format!("Commands now use {}.", paint(STRONG, found.label())));
            Ok(())
        }
        AuthCommand::Status => {
            let mut session = session(site)?;
            let who = session.call("whoami", &json!({}))?;
            output.success(&who["data"], Some(&session.origin), None);
            Ok(())
        }
        AuthCommand::Allow => {
            let cwd = std::env::current_dir()?;
            let (folder, reference) = config::folder_binding(&cwd).with_context(|| format!("no {} here or above", config::FOLDER_FILE))?;
            let mut config = Config::load()?;
            let profile = config.find(&reference)?;
            config.allowed_folders.insert(folder.to_string_lossy().to_string(), profile.key());
            config.save()?;
            style::done(format!("Commands in {} now use {}.", paint(STRONG, folder.display()), paint(STRONG, profile.label())));
            Ok(())
        }
    }
}

fn login(url: &str, device: bool, open_browser: bool, insecure_storage: bool, output: Output) -> Result<()> {
    let origin = http::origin(url)?;
    let server = auth::discover(&origin)?;
    let storage = if insecure_storage { Storage::File } else { Storage::Keyring };
    if storage == Storage::Keyring
        && let Err(problem) = store::keyring_status()
    {
        bail!("this system's keychain isn't available ({problem}). Sign in with --insecure-storage to keep it in a file only you can read");
    }
    let tokens = if device { auth::device_login(&server)? } else { auth::browser_login(&server, open_browser)? };

    let site = Site {
        name: config::host(&origin),
        issuer: server.issuer.clone(),
        api: server.api.clone(),
        token_endpoint: server.token_endpoint.clone(),
        revocation_endpoint: server.revocation_endpoint.clone(),
        accounts: Default::default(),
    };
    let who = api::Session::with_tokens(site.clone(), origin.clone(), tokens.clone()).call("whoami", &json!({}))?;
    let data = &who["data"];
    let email = data["person"]["email"].as_str().context("the site didn't say who you are")?.to_string();
    let account = Account {
        name: data["person"]["name"].as_str().unwrap_or_default().to_string(),
        email: email.clone(),
        access: data["connection"]["access"].as_str().unwrap_or("custom").to_string(),
        storage,
    };
    store::save(&origin, &email, storage, &tokens)?;
    let mut config = Config::load()?;
    config.upsert(&origin, Site { name: data["site"]["name"].as_str().unwrap_or(&site.name).to_string(), ..site }, account.clone());
    if config.default.is_none() {
        config.default = Some(format!("{origin}#{email}"));
    }
    config.save()?;
    if let Some(profile) = config.by_key(&format!("{origin}#{email}"))
        && let Ok(mut session) = api::Session::for_profile(profile)
        && let Ok(catalogue) = session.catalogue()
    {
        complete::remember(&mut session, &catalogue);
    }
    if output.json {
        output.success(&json!({ "site": origin, "account": email, "access": account.access }), Some(&origin), None);
    } else {
        style::done(format!(
            "Signed in to {} as {} {}",
            paint(STRONG, config::host(&origin)),
            paint(STRONG, &email),
            paint(DIM, format!("({} access)", account.access))
        ));
    }
    Ok(())
}

fn skill_command(command: SkillCommand, site: Option<String>) -> Result<()> {
    match command {
        SkillCommand::Install { client, dir } => {
            let dir = agents::skills_dir(client.as_deref(), dir)?;
            let mut session = session(site)?;
            let (folder, changed) = agents::install_skill(&mut session, &dir)?;
            style::done(format!(
                "{} {}",
                if changed { "Wrote" } else { "Already current:" },
                paint(STRONG, folder.join("SKILL.md").display())
            ));
        }
        SkillCommand::Sync => {
            let config = Config::load()?;
            let installed = agents::installed_skills();
            if installed.is_empty() {
                anstream::eprintln!("No site skills installed. {}", paint(DIM, "`nibble skill install --client claude` adds one."));
            }
            for (folder, origin) in installed {
                let Some(profile) = config.profiles().into_iter().find(|profile| profile.origin == origin) else {
                    anstream::eprintln!("{} Skipped {}: not signed in to {origin}", paint(CHANGE, "!"), paint(STRONG, folder.display()));
                    continue;
                };
                let mut session = api::Session::for_profile(profile)?;
                let (_, changed) = agents::install_skill(&mut session, folder.parent().unwrap())?;
                style::done(format!("{} {}", if changed { "Updated" } else { "Current:" }, paint(STRONG, folder.display())));
            }
        }
    }
    Ok(())
}

fn doctor(site: Option<String>, output: Output) -> Result<i32> {
    let mut checks: Vec<(String, bool, String)> = Vec::new();
    let mut check = |name: &str, result: Result<String>| match result {
        Ok(detail) => checks.push((name.into(), true, detail)),
        Err(error) => checks.push((name.into(), false, format!("{error:#}"))),
    };

    check(
        "config",
        Config::load().map(|config| format!("{} (signed in to {})", config::dir().unwrap_or_default().display(), config.profiles().len())),
    );
    check(
        "keychain",
        store::keyring_status()
            .map(|_| "available".to_string())
            .map_err(|problem| anyhow::anyhow!("{problem}; sign in with --insecure-storage")),
    );
    let cwd = std::env::current_dir()?;
    if let Some(root) = project::root(&cwd) {
        check("site folder", Ok(format!("{} runs Nibble {}", root.display(), project::version(&root).unwrap_or_else(|| "?".into()))));
        check("site tasks", project::tasks(&root).map(|tasks| format!("{} tasks, ready for completion", tasks.len())));
    }
    match target(site) {
        Ok(profile) => {
            let label = profile.label();
            let storage = profile.account.storage;
            check("sign-in", Ok(format!("{label}{}", if storage == Storage::File { " (kept in a file, not the keychain)" } else { "" })));
            check(
                "site",
                api::Session::for_profile(profile).and_then(|mut session| session.call("whoami", &json!({}))).map(|who| {
                    format!(
                        "{} access, can: {}",
                        who["data"]["connection"]["access"].as_str().unwrap_or("?"),
                        who["data"]["can"].as_object().map(|can| can.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()
                    )
                }),
            );
        }
        Err(error) => check("sign-in", Err(error)),
    }

    let healthy = checks.iter().all(|(_, ok, _)| *ok);
    if output.json {
        let rows: Vec<Value> = checks.iter().map(|(name, ok, detail)| json!({ "check": name, "ok": ok, "detail": detail })).collect();
        println!("{}", json!({ "ok": healthy, "data": rows }));
    } else {
        let width = checks.iter().map(|(name, _, _)| name.chars().count()).max().unwrap_or(0);
        for (name, ok, detail) in &checks {
            let mark = if *ok { paint(GOOD, "✓") } else { paint(BAD, "✗") };
            anstream::println!(
                "{mark} {}  {}",
                paint(STRONG, format!("{name:<width$}")),
                if *ok { paint(DIM, detail) } else { detail.clone() }
            );
        }
    }
    Ok(if healthy { 0 } else { 1 })
}
