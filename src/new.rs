use crate::style::{DIM, GOOD, NAME, STRONG, paint};
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

pub const NIBBLE_REPOSITORY: &str = "mah3uz/nibble";

const MISE: &str = "Ruby and Node: install mise (https://mise.jdx.dev), then run: mise use --global ruby@latest node@lts";
const BREW: &str = "The rest, on macOS: brew install sqlite vips ffmpeg";
const DEBIAN: &str = "The rest, on Debian or Ubuntu: sudo apt install sqlite3 libvips-tools ffmpeg";
const ARCH: &str = "The rest, on Arch Linux: sudo pacman -S --needed sqlite libvips ffmpeg";

// Distributions package a Ruby and Node older than Nibble needs, so those come from mise and the rest from the system.
fn install_hints(missing: &[&str], os: &str, os_release: &str) -> Vec<&'static str> {
    let mut hints = Vec::new();
    if missing.iter().any(|program| ["ruby", "bundle", "node", "npm"].contains(program)) {
        hints.push(MISE);
    }
    if !missing.iter().any(|program| ["sqlite3", "vips", "ffmpeg"].contains(program)) {
        return hints;
    }
    let family: Vec<&str> = os_release
        .lines()
        .filter_map(|line| line.strip_prefix("ID=").or_else(|| line.strip_prefix("ID_LIKE=")))
        .flat_map(|value| value.trim_matches('"').split_whitespace())
        .collect();
    let like = |names: &[&str]| family.iter().any(|id| names.contains(id));
    match os {
        "macos" => hints.push(BREW),
        "linux" => match (like(&["debian", "ubuntu"]), like(&["arch", "manjaro", "endeavouros"])) {
            (true, false) => hints.push(DEBIAN),
            (false, true) => hints.push(ARCH),
            _ => hints.extend([DEBIAN, ARCH]),
        },
        _ => {}
    }
    hints
}

fn parse_version(text: &str) -> Vec<u64> {
    text.trim().trim_start_matches(['v', 'V']).split('.').map_while(|part| part.parse().ok()).collect()
}

// A release names the Ruby and Node it needs; checking them here beats a failure deep inside bundle or npm install.
fn too_old(declared: &str, ruby: Option<&str>, node: Option<&str>) -> Vec<String> {
    let needs = |key: &str| declared.lines().find_map(|line| line.strip_prefix(&format!("{key}: ")).map(|value| value.trim().to_string()));
    let mut problems = Vec::new();
    for (key, label, found, tool) in [("ruby", "Ruby", ruby, "ruby"), ("node", "Node", node, "node")] {
        let (Some(needed), Some(found)) = (needs(key), found) else { continue };
        if parse_version(found) < parse_version(&needed) {
            let major = needed.split('.').next().unwrap_or(&needed);
            problems.push(format!(
                "{label} {needed} or newer is needed, and this computer has {}. With mise (https://mise.jdx.dev): mise use --global {tool}@{}",
                found.trim().trim_start_matches('v'),
                if key == "node" { major.to_string() } else { needed.clone() }
            ));
        }
    }
    problems
}

fn installed_version(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn folder_for(name: &str) -> String {
    let folder = name.split_whitespace().collect::<Vec<_>>().join("-");
    if folder.is_empty() { "nibble".to_string() } else { folder }
}

// A folder only this user can open, under a name no one can claim first, so the release can't be swapped after it is verified.
fn private_work_dir() -> Result<PathBuf> {
    let mut suffix = [0u8; 8];
    rand::fill(&mut suffix[..]);
    let name: String = suffix.iter().map(|byte| format!("{byte:02x}")).collect();
    let work = std::env::temp_dir().join(format!("nibble-new-{name}"));
    fs::create_dir(&work).with_context(|| format!("couldn't create {}", work.display()))?;
    crate::config::private_dir(&work)?;
    Ok(work)
}

fn ask_name() -> Result<String> {
    if !std::io::stdin().is_terminal() {
        return Ok("nibble".into());
    }
    anstream::eprint!("  {} {} ", paint(STRONG, "Name your site;"), paint(DIM, "its folder is named after it (nibble):"));
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub fn run(name: Option<String>, version: Option<String>, install_args: &[String]) -> Result<i32> {
    let missing: Vec<(&str, &str)> = NEEDS.iter().copied().filter(|(program, _)| !on_path(program)).collect();
    if !missing.is_empty() {
        let os_release = fs::read_to_string("/etc/os-release").unwrap_or_default();
        let programs: Vec<&str> = missing.iter().map(|(program, _)| *program).collect();
        let hint: String = install_hints(&programs, std::env::consts::OS, &os_release).iter().map(|hint| format!("\n  {hint}")).collect();
        let listed: Vec<String> = missing.iter().map(|(program, why)| format!("{program} — {why}")).collect();
        bail!("this computer is missing:\n    {}{hint}", listed.join("\n    "));
    }
    anstream::eprintln!("  {} Everything needed is here", paint(GOOD, "✓"));

    let asking = name.is_none() && std::io::stdin().is_terminal();
    let mut folder = folder_for(&name.map_or_else(ask_name, Ok)?);
    while PathBuf::from(&folder).exists() {
        if !asking {
            bail!("{folder} already exists; choose another name");
        }
        anstream::eprintln!("  {} {folder} already exists; choose another name", paint(crate::style::BAD, "✗"));
        folder = folder_for(&ask_name()?);
    }
    let dir = PathBuf::from(&folder);

    let work = private_work_dir()?;
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
        let repository = std::env::var("NIBBLE_REPOSITORY").unwrap_or_else(|_| NIBBLE_REPOSITORY.into());
        let version = match version.or_else(|| std::env::var("NIBBLE_VERSION").ok()) {
            Some(version) => version,
            None => latest(&repository)?,
        };
        anstream::eprintln!("  {} Nibble {}", paint(NAME, "Fetching"), paint(STRONG, &version));
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
    let declared = fs::read_to_string(source.join("VERSION")).unwrap_or_default();
    let ruby = installed_version("ruby", &["-e", "print RUBY_VERSION"]);
    let node = installed_version("node", &["--version"]);
    let problems = too_old(&declared, ruby.as_deref(), node.as_deref());
    if !problems.is_empty() {
        bail!("nothing was installed:\n    {}", problems.join("\n    "));
    }
    fs::create_dir_all(dir.join("vendor"))?;
    fs::rename(&source, dir.join("vendor/nibble")).or_else(|_| copy_dir(&source, &dir.join("vendor/nibble")))?;
    let version = crate::project::version(dir).unwrap_or_default();
    anstream::eprintln!("  {} Unpacked Nibble {} into {}", paint(GOOD, "✓"), paint(STRONG, version), paint(STRONG, dir.display()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruby_and_node_come_from_mise_and_the_rest_from_this_systems_packages() {
        let arch = "ID=arch\n";
        assert_eq!(install_hints(&["ruby", "vips"], "linux", arch), [MISE, ARCH]);
        assert_eq!(install_hints(&["node"], "linux", arch), [MISE], "no package line when only Ruby or Node is missing");
        assert_eq!(install_hints(&["ffmpeg"], "linux", "ID=endeavouros\nID_LIKE=arch\n"), [ARCH], "Arch's derivatives use pacman too");
        assert_eq!(install_hints(&["vips"], "linux", "ID=\"linuxmint\"\nID_LIKE=\"ubuntu debian\"\n"), [DEBIAN]);
        assert_eq!(install_hints(&["vips"], "linux", ""), [DEBIAN, ARCH], "unknown: show the lines people can adapt");
        assert_eq!(install_hints(&["sqlite3"], "macos", ""), [BREW]);
    }

    #[test]
    fn a_ruby_or_node_older_than_the_release_needs_is_named_before_installing() {
        let declared = "version: 0.19.0\nruby: 4.0.6\nnode: 24.0.0\n";
        assert!(too_old(declared, Some("4.0.6"), Some("v26.10.0")).is_empty());
        assert!(too_old(declared, Some("4.1.0"), Some("v24.0.0")).is_empty());
        let problems = too_old(declared, Some("3.4.10"), Some("v18.19.1"));
        assert_eq!(problems.len(), 2);
        assert!(problems[0].contains("Ruby 4.0.6 or newer") && problems[0].contains("has 3.4.10") && problems[0].contains("ruby@4.0.6"));
        assert!(problems[1].contains("Node 24.0.0 or newer") && problems[1].contains("node@24"));
        assert!(too_old("version: 0.1.0\n", Some("2.0.0"), None).is_empty(), "an older release that names no minimums");
    }

    #[test]
    fn a_site_name_becomes_one_folder_name() {
        assert_eq!(folder_for("  Tide water  site "), "Tide-water-site");
        assert_eq!(folder_for("   "), "nibble", "an empty answer takes the default the question offers");
    }

    #[test]
    fn each_install_works_in_a_new_folder_only_its_user_can_open() {
        let (first, second) = (private_work_dir().unwrap(), private_work_dir().unwrap());
        assert_ne!(first, second, "a name someone could guess and create first would let them swap the release");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&first).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = (fs::remove_dir(first), fs::remove_dir(second));
    }
}
