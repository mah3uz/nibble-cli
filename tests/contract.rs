use serde_json::Value;
use std::process::Command;

fn nibble(args: &[&str]) -> Value {
    let site = std::env::var("NIBBLE_CONTRACT_SITE").unwrap();
    let token = std::env::var("NIBBLE_CONTRACT_TOKEN").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nibble"))
        .args(args)
        .env("NIBBLE_SITE", site)
        .env("NIBBLE_TOKEN", token)
        .env("NIBBLE_CONFIG_DIR", std::env::temp_dir().join("nibble-contract-config"))
        .output()
        .expect("the nibble binary runs");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&output.stdout)))
}

fn complete(words: &[&str], current: &str) -> Vec<String> {
    let site = std::env::var("NIBBLE_CONTRACT_SITE").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nibble"))
        .args(["__complete", &format!("--current={current}"), "--", "nibble"])
        .args(words)
        .env("NIBBLE_SITE", site)
        .env("NIBBLE_TOKEN", std::env::var("NIBBLE_CONTRACT_TOKEN").unwrap())
        .env("NIBBLE_CONFIG_DIR", std::env::temp_dir().join("nibble-contract-config"))
        .output()
        .expect("the nibble binary runs");
    String::from_utf8_lossy(&output.stdout).lines().skip(1).filter_map(|line| line.split('\t').next().map(str::to_string)).collect()
}

// Runs against a live site: script/contract boots this checkout's Nibble and sets the variables.
#[test]
fn the_cli_and_the_site_agree_on_the_management_api() {
    if std::env::var("NIBBLE_CONTRACT_SITE").is_err() {
        eprintln!("skipped: run script/contract to test against a real site");
        return;
    }
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("nibble-contract-config"));

    let who = nibble(&["remote", "whoami"]);
    assert_eq!(who["ok"], true, "{who}");
    assert!(who["data"]["person"]["email"].is_string());

    let operations = nibble(&["remote"]);
    let names: Vec<&str> = operations["data"].as_array().unwrap().iter().filter_map(|op| op["name"].as_str()).collect();
    assert!(names.contains(&"list_entries") && names.contains(&"create_entry"), "{names:?}");

    let site = nibble(&["remote", "describe_site"]);
    let collection = site["data"]["collections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["written_in_files"] != true)
        .and_then(|item| item["handle"].as_str())
        .expect("a collection stored in the database")
        .to_string();
    assert_eq!(nibble(&["remote", "list_entries", "--collection", &collection, "--per-page", "2"])["ok"], true);

    // Completion reads what `remote` cached from the site, so the site's answers must keep the shape it reads.
    assert!(complete(&["remote"], "").contains(&"create-entry".to_string()), "operations complete after `remote` has run");
    assert!(complete(&["remote", "list-entries", "--collection"], "").contains(&collection), "collections complete from describe_site");
    assert!(!complete(&["remote", "create-entry", "--collection", &collection, "--blueprint"], "").is_empty(), "so do its blueprints");

    let rehearsal =
        nibble(&["remote", "create_entry", "--collection", &collection, "--data", r#"{"title":"Contract check"}"#, "--dry-run"]);
    assert_eq!(rehearsal["data"]["saved"], false, "{rehearsal}");

    let created =
        nibble(&["remote", "create_entry", "--collection", &collection, "--data", r#"{"title":"Contract draft"}"#, "--pick", "id"]);
    let id = created.as_i64().expect("--pick gives a script the bare value, not the envelope").to_string();
    let refused = nibble(&["remote", "transition_entry", "--id", &id, "--action", "publish"]);
    assert_eq!(refused["error"]["code"], "forbidden", "a Draft token never publishes: {refused}");
    assert_eq!(refused["ok"], false);
    assert!(refused["error"]["hint"].is_string(), "refusals carry a hint: {refused}");
}

// `nibble new` runs before any site exists, so it carries its own copy of where Nibble comes from.
#[test]
fn nibble_new_downloads_from_the_repository_nibble_names() {
    let Ok(checkout) = std::env::var("NIBBLE_CONTRACT_CHECKOUT") else {
        eprintln!("skipped: run script/contract to compare with a Nibble checkout");
        return;
    };
    let nibble = std::fs::read_to_string(format!("{checkout}/vendor/nibble/lib/nibble.rb")).unwrap();
    let named = nibble.split("REPOSITORY = \"https://github.com/").nth(1).and_then(|rest| rest.split(".git\"").next()).unwrap();
    let ours = std::fs::read_to_string("src/new.rs").unwrap();
    assert!(ours.contains(&format!("pub const NIBBLE_REPOSITORY: &str = \"{named}\";")), "Nibble says {named}");
}
