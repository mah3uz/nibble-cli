# nibble

The command-line tool for [Nibble](https://nibble.ink). It starts a site, runs a site's tasks, and works on the
content of any Nibble site you sign in to, as you and never as more than you.

## Start a site

```sh
nibble new my-site            # checks this computer, fetches the latest release, verifies it, and installs
```

## Run a site's tasks

Inside a site's folder, any word that isn't one of the tool's own runs a task: `nibble check` is
`bin/rails nibble:check`, `nibble schema show collections/posts` is `bin/rails nibble:schema:show collections/posts`.

## Work on a site's content

```sh
nibble auth login example.com          # opens the site in your browser to sign in and choose what the CLI may do
nibble remote                          # what this connection can do on the site
nibble remote list-entries --collection posts
nibble remote update-entry --id 12 --lock-version 3 --data '{"title":"Better title"}' --dry-run
```

Every operation comes from the site itself, so `nibble remote` always matches the site you're signed in to.
Changes to entries are drafts until someone who may publish does. Add `--dry-run` to see a change without saving it.

- **More than one site or account:** `nibble auth list`, `nibble auth switch example.com:you@example.com`, or
  `--site` on any command.
- **A folder that always means one site:** put `site = "example.com"` in `.nibble.toml` and run `nibble auth allow`
  there once. A `.nibble.toml` you haven't allowed is refused, so a cloned repository can't point your commands at a
  site you didn't choose.
- **Scripts and CI:** create a token under *Connected apps* in the site's Control Plane, then set `NIBBLE_SITE` and
  `NIBBLE_TOKEN`.
- **A server without a browser:** `nibble auth login example.com --device`, if the site allows signing in with a code.

Sign-ins are kept in this system's keychain. Where there is none, `--insecure-storage` keeps them in a file only you
can read, and `nibble doctor` says so.

## AI apps

```sh
nibble mcp install --client claude-code   # or codex, cursor; claude and chatgpt print where to paste the address
nibble skill install --client claude      # the site's own guide as a skill; `nibble skill sync` refreshes it
```

The site must have Agent access turned on by an administrator.

## Output

On a terminal, lists print as tables. Otherwise, or with `--json`, every command prints
`{"ok": true, "data": …, "site": …}` or `{"ok": false, "error": {"code", "message", "hint"}}`. `--pick entries.0.title`
prints one value.

## Developing

```sh
cargo test                 # unit tests
script/contract            # the CLI against a Nibble checkout (../ by default) on a test database
```
