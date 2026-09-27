use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const NEEDS: [(&str, &str); 7] = [
    ("ruby", "the application"),
    ("bundle", "Ruby's gems (comes with Ruby)"),
    ("node", "the asset build and server-side rendering"),
    ("npm", "JavaScript dependencies"),
    ("sqlite3", "the database"),
    ("vips", "image resizing (package: libvips / libvips-tools)"),
    ("ffmpeg", "video thumbnails"),
];

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

fn downloader() -> reqwest::blocking::Client {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let host = attempt.url().host_str().unwrap_or("").to_string();
        let https = attempt.url().scheme() == "https";
        if attempt.previous().len() > 5 || !https {
            attempt.stop()
        } else if host == "github.com" || host.ends_with(".githubusercontent.com") {
            attempt.follow()
        } else {
            attempt.error(format!("the download was redirected to {host}, which isn't GitHub"))
        }
    });
    reqwest::blocking::Client::builder()
        .redirect(policy)
        .user_agent(concat!("nibble-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(600))
        .build()
        .expect("an HTTP client")
}

fn latest(repository: &str) -> Result<String> {
    let response = crate::http::client().get(format!("https://github.com/{repository}/releases/latest")).send()?;
    let location = response.headers().get("location").and_then(|value| value.to_str().ok()).unwrap_or_default();
    location
        .rsplit_once("/releases/tag/v")
        .map(|(_, version)| version.to_string())
        .with_context(|| format!("{repository} has no release to install yet"))
}

fn ask_name() -> Result<String> {
    if !std::io::stdin().is_terminal() {
        return Ok("nibble".into());
    }
    eprint!("  Name your site; its folder is named after it (nibble): ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub fn run(name: Option<String>, version: Option<String>, install_args: &[String]) -> Result<i32> {
    let missing: Vec<String> =
        NEEDS.iter().filter(|(program, _)| !on_path(program)).map(|(program, why)| format!("{program} — {why}")).collect();
    if !missing.is_empty() {
        bail!("this computer is missing:\n    {}", missing.join("\n    "));
    }

    let name = match name {
        Some(name) => name,
        None => ask_name()?,
    };
    let folder = name.split_whitespace().collect::<Vec<_>>().join("-");
    let folder = if folder.is_empty() { "nibble".to_string() } else { folder };
    let dir = PathBuf::from(&folder);
    if dir.exists() {
        bail!("{folder} already exists; choose another name");
    }

    let work = std::env::temp_dir().join(format!("nibble-new-{}", std::process::id()));
    fs::create_dir_all(&work)?;
    let result = fetch_and_unpack(&work, &dir, version);
    let _ = fs::remove_dir_all(&work);
    result?;

    let mut install = Command::new("ruby");
    install.arg("vendor/nibble/bin/install").args(install_args).current_dir(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(install.exec()).context("couldn't start the installer")
    }
    #[cfg(not(unix))]
    {
        Ok(install.status()?.code().unwrap_or(1))
    }
}

fn fetch_and_unpack(work: &Path, dir: &Path, version: Option<String>) -> Result<()> {
    let archive = if let Ok(local) = std::env::var("NIBBLE_ARCHIVE") {
        let local = PathBuf::from(local);
        let sums = local.parent().unwrap_or(Path::new(".")).join("SHA256SUMS");
        fs::copy(&local, work.join(local.file_name().context("NIBBLE_ARCHIVE has no file name")?))?;
        fs::copy(&sums, work.join("SHA256SUMS")).context("NIBBLE_ARCHIVE needs its SHA256SUMS beside it")?;
        work.join(local.file_name().unwrap())
    } else {
        let repository = std::env::var("NIBBLE_REPOSITORY").unwrap_or_else(|_| "mah3uz/nibble".into());
        let version = match version.or_else(|| std::env::var("NIBBLE_VERSION").ok()) {
            Some(version) => version,
            None => latest(&repository)?,
        };
        eprintln!("  Fetching Nibble {version}");
        let base = format!("https://github.com/{repository}/releases/download/v{version}");
        let client = downloader();
        let file = format!("nibble-{version}.tar.gz");
        for name in [file.as_str(), "SHA256SUMS"] {
            let response =
                client.get(format!("{base}/{name}")).send()?.error_for_status().with_context(|| format!("couldn't download {name}"))?;
            fs::write(work.join(name), response.bytes()?)?;
        }
        work.join(file)
    };

    verify(&archive, &work.join("SHA256SUMS"))?;
    let unpacked = work.join("unpacked");
    tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(&archive)?)).unpack(&unpacked)?;
    let source = fs::read_dir(&unpacked)?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir() && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("nibble-")))
        .context("the archive doesn't hold a nibble- folder")?;
    fs::create_dir_all(dir.join("vendor"))?;
    fs::rename(&source, dir.join("vendor/nibble")).or_else(|_| copy_dir(&source, &dir.join("vendor/nibble")))?;
    eprintln!("  Unpacked Nibble into {}", dir.display());
    Ok(())
}

fn verify(archive: &Path, sums: &Path) -> Result<()> {
    let name = archive.file_name().unwrap().to_string_lossy().to_string();
    let listed = fs::read_to_string(sums)?
        .lines()
        .find_map(|line| line.split_once("  ").filter(|(_, file)| file.trim() == name).map(|(sum, _)| sum.to_string()))
        .with_context(|| format!("SHA256SUMS doesn't list {name}"))?;
    let actual: String = Sha256::digest(fs::read(archive)?).iter().map(|byte| format!("{byte:02x}")).collect();
    if actual != listed {
        bail!("the download doesn't match its checksum; nothing was installed");
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
