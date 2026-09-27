use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join("vendor/nibble/lib/nibble.rb").is_file() && dir.join("bin/rails").is_file())
        .map(Path::to_path_buf)
}

pub fn version(root: &Path) -> Option<String> {
    let released = fs::read_to_string(root.join("vendor/nibble/VERSION"))
        .ok()
        .and_then(|text| text.lines().find_map(|line| line.strip_prefix("version: ").map(|version| version.trim().to_string())));
    released.or_else(|| {
        fs::read_to_string(root.join("vendor/nibble/lib/nibble.rb"))
            .ok()?
            .lines()
            .find_map(|line| line.trim().strip_prefix("VERSION = \"").and_then(|rest| rest.split('"').next()).map(str::to_string))
    })
}

fn tasks(root: &Path) -> Result<Vec<String>> {
    let cache = root.join("tmp/nibble-cli-tasks.txt");
    let stamp = format!("{} {}", version(root).unwrap_or_default(), newest(&root.join("vendor/nibble/lib/commands")));
    if let Ok(text) = fs::read_to_string(&cache)
        && let Some((first, rest)) = text.split_once('\n')
        && first == stamp
    {
        return Ok(rest.lines().map(str::to_string).collect());
    }

    let output = Command::new(root.join("bin/rails")).arg("--help").current_dir(root).output().context("couldn't run bin/rails")?;
    let names: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|name| name.strip_prefix("nibble:").map(str::to_string))
        .collect();
    if names.is_empty() {
        bail!("bin/rails listed no nibble tasks; is this site set up? Try `bin/setup`");
    }
    let _ = fs::create_dir_all(root.join("tmp"));
    let _ = fs::write(&cache, format!("{stamp}\n{}", names.join("\n")));
    Ok(names)
}

fn newest(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                newest(&path)
            } else {
                entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |age| age.as_secs())
            }
        })
        .max()
        .unwrap_or(0)
}

pub fn resolve(root: &Path, words: &[String]) -> Result<(String, Vec<String>)> {
    longest_match(&tasks(root)?, words)
}

pub fn longest_match(known: &[String], words: &[String]) -> Result<(String, Vec<String>)> {
    let leading: Vec<&String> = words.iter().take_while(|word| !word.starts_with('-')).take(3).collect();
    for length in (1..=leading.len()).rev() {
        let name = leading[..length].iter().map(|word| word.as_str()).collect::<Vec<_>>().join(":");
        if known.contains(&name) {
            return Ok((name, words[length..].to_vec()));
        }
    }
    let asked = leading.iter().map(|word| word.as_str()).collect::<Vec<_>>().join(" ");
    bail!("no task called `nibble {asked}` here; tasks: {}", known.join(", ").replace(':', " "))
}

pub fn run(root: &Path, task: &str, args: &[String]) -> Result<i32> {
    let mut command = Command::new(root.join("bin/rails"));
    command.arg(format!("nibble:{task}")).args(args).current_dir(root);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(error).context("couldn't run bin/rails")
    }
    #[cfg(not(unix))]
    {
        Ok(command.status().context("couldn't run bin/rails")?.code().unwrap_or(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn the_longest_task_name_wins_and_the_rest_are_its_arguments() {
        let known = words("check schema:show generate:view upgrade");
        assert_eq!(
            longest_match(&known, &words("schema show collections/posts")).unwrap(),
            ("schema:show".into(), words("collections/posts"))
        );
        assert_eq!(longest_match(&known, &words("upgrade 0.19.0")).unwrap(), ("upgrade".into(), words("0.19.0")));
        assert_eq!(longest_match(&known, &words("check --verbose")).unwrap(), ("check".into(), words("--verbose")));
        assert!(longest_match(&known, &words("schema drop")).is_err());
    }
}
