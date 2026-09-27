use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const FOLDER_FILE: &str = ".nibble.toml";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub sites: BTreeMap<String, Site>,
    #[serde(default)]
    pub allowed_folders: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub name: String,
    pub issuer: String,
    pub api: String,
    pub token_endpoint: String,
    pub revocation_endpoint: Option<String>,
    #[serde(default)]
    pub accounts: BTreeMap<String, Account>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub email: String,
    pub access: String,
    #[serde(default)]
    pub storage: Storage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Storage {
    #[default]
    Keyring,
    File,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub origin: String,
    pub site: Site,
    pub account: Account,
}

impl Profile {
    pub fn key(&self) -> String {
        format!("{}#{}", self.origin, self.account.email)
    }

    pub fn label(&self) -> String {
        format!("{} as {}", host(&self.origin), self.account.email)
    }
}

#[derive(Debug, Deserialize)]
struct FolderFile {
    site: String,
    account: Option<String>,
}

pub fn dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("NIBBLE_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(dirs::config_dir().context("this system has no config folder")?.join("nibble"))
}

pub fn host(origin: &str) -> String {
    url::Url::parse(origin)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            })
        })
        .unwrap_or_else(|| origin.to_string())
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = dir()?.join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("{} isn't valid", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let dir = dir()?;
        private_dir(&dir)?;
        let path = dir.join("config.toml");
        fs::write(&path, toml::to_string_pretty(self)?)?;
        restrict(&path)
    }

    pub fn profiles(&self) -> Vec<Profile> {
        self.sites
            .iter()
            .flat_map(|(origin, site)| {
                site.accounts.values().map(move |account| Profile { origin: origin.clone(), site: site.clone(), account: account.clone() })
            })
            .collect()
    }

    pub fn find(&self, reference: &str) -> Result<Profile> {
        let (site_part, account_part) = match reference.rsplit_once(':') {
            Some((site, account)) if account.contains('@') => (site, Some(account)),
            _ => (reference, None),
        };
        let wanted = site_part.trim_end_matches('/');
        let matches: Vec<Profile> = self
            .profiles()
            .into_iter()
            .filter(|profile| profile.origin == wanted || host(&profile.origin) == wanted || profile.site.name.eq_ignore_ascii_case(wanted))
            .filter(|profile| account_part.is_none_or(|email| profile.account.email.eq_ignore_ascii_case(email)))
            .collect();
        match matches.len() {
            0 => bail!("no signed-in account matches {reference}; run `nibble auth list`"),
            1 => Ok(matches.into_iter().next().unwrap()),
            _ => {
                bail!("{reference} matches more than one account; name one, like {}:{}", host(&matches[0].origin), matches[0].account.email)
            }
        }
    }

    pub fn by_key(&self, key: &str) -> Option<Profile> {
        self.profiles().into_iter().find(|profile| profile.key() == key)
    }

    pub fn upsert(&mut self, origin: &str, site: Site, account: Account) {
        let entry = self.sites.entry(origin.to_string()).or_insert_with(|| site.clone());
        entry.name = site.name;
        entry.issuer = site.issuer;
        entry.api = site.api;
        entry.token_endpoint = site.token_endpoint;
        entry.revocation_endpoint = site.revocation_endpoint;
        entry.accounts.insert(account.email.clone(), account);
    }

    pub fn remove(&mut self, profile: &Profile) {
        if let Some(site) = self.sites.get_mut(&profile.origin) {
            site.accounts.remove(&profile.account.email);
            if site.accounts.is_empty() {
                self.sites.remove(&profile.origin);
            }
        }
        if self.default.as_deref() == Some(profile.key().as_str()) {
            self.default = None;
        }
        self.allowed_folders.retain(|_, key| *key != profile.key());
    }
}

pub fn folder_binding(start: &Path) -> Option<(PathBuf, String)> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let file = dir.join(FOLDER_FILE);
        if file.is_file() {
            let parsed: FolderFile = toml::from_str(&fs::read_to_string(&file).ok()?).ok()?;
            let reference = match parsed.account {
                Some(account) => format!("{}:{account}", parsed.site),
                None => parsed.site,
            };
            return Some((dir.to_path_buf(), reference));
        }
        current = dir.parent();
    }
    None
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(unix)]
pub fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn restrict(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let mut config = Config::default();
        for (origin, email) in [("https://a.test", "me@a.test"), ("https://a.test", "bot@a.test"), ("https://b.test", "me@b.test")] {
            let site = Site {
                name: "Site".into(),
                issuer: origin.into(),
                api: format!("{origin}/api/v1"),
                token_endpoint: String::new(),
                revocation_endpoint: None,
                accounts: BTreeMap::new(),
            };
            let account = Account { name: "Me".into(), email: email.into(), access: "draft".into(), storage: Storage::Keyring };
            config.upsert(origin, site, account);
        }
        config
    }

    #[test]
    fn a_site_with_two_accounts_must_say_which_one() {
        let config = config();
        assert!(config.find("a.test").is_err(), "guessing between two people's accounts could write as the wrong one");
        assert_eq!(config.find("a.test:bot@a.test").unwrap().account.email, "bot@a.test");
        assert_eq!(config.find("b.test").unwrap().account.email, "me@b.test");
        assert!(config.find("c.test").is_err());
    }

    #[test]
    fn signing_out_forgets_the_account_and_any_folder_it_was_allowed_in() {
        let mut config = config();
        let profile = config.find("b.test").unwrap();
        config.default = Some(profile.key());
        config.allowed_folders.insert("/work".into(), profile.key());
        config.remove(&profile);
        assert!(config.find("b.test").is_err());
        assert!(config.default.is_none());
        assert!(config.allowed_folders.is_empty());
    }
}
