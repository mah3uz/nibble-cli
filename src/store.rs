use crate::config::{self, Profile, Storage};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

const SERVICE: &str = "nibble";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

impl Tokens {
    pub fn fresh(&self) -> bool {
        self.expires_at - 60 > now()
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0)
}

fn user(origin: &str, email: &str) -> String {
    format!("{origin} {email}")
}

fn file(origin: &str, email: &str) -> Result<PathBuf> {
    let name = hex(&Sha256::digest(user(origin, email).as_bytes()));
    Ok(config::dir()?.join("credentials").join(format!("{}.json", &name[..32])))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn save(origin: &str, email: &str, storage: Storage, tokens: &Tokens) -> Result<()> {
    let json = serde_json::to_string(tokens)?;
    match storage {
        Storage::Keyring => keyring::Entry::new(SERVICE, &user(origin, email))
            .and_then(|entry| entry.set_password(&json))
            .map_err(|error| {
                anyhow!("couldn't save to this system's keychain ({error}). Sign in with --insecure-storage to keep it in a file only you can read")
            }),
        Storage::File => {
            let path = file(origin, email)?;
            config::private_dir(path.parent().unwrap())?;
            fs::write(&path, json)?;
            config::restrict(&path)
        }
    }
}

pub fn load(profile: &Profile) -> Result<Tokens> {
    let (origin, email) = (&profile.origin, &profile.account.email);
    let json = match profile.account.storage {
        Storage::Keyring => keyring::Entry::new(SERVICE, &user(origin, email))
            .and_then(|entry| entry.get_password())
            .map_err(|error| anyhow!("no saved sign-in for {} ({error}); run `nibble auth login {origin}`", profile.label()))?,
        Storage::File => fs::read_to_string(file(origin, email)?).with_context(|| format!("no saved sign-in for {}", profile.label()))?,
    };
    serde_json::from_str(&json).context("the saved sign-in is unreadable; sign in again")
}

pub fn delete(profile: &Profile) -> Result<()> {
    match profile.account.storage {
        Storage::Keyring => {
            if let Ok(entry) = keyring::Entry::new(SERVICE, &user(&profile.origin, &profile.account.email)) {
                let _ = entry.delete_credential();
            }
        }
        Storage::File => {
            let _ = fs::remove_file(file(&profile.origin, &profile.account.email)?);
        }
    }
    Ok(())
}

pub fn keyring_status() -> Result<(), String> {
    keyring::Entry::store_status().as_ref().map(|_| ()).map_err(|error| error.to_string())
}
