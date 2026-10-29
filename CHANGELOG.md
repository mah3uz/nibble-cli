# Changelog

## Unreleased

## 0.3.4 - 2026-10-30

- **`nibble connect` never overwrites a skill or server it didn't write:** when the skill's folder is a link, or holds
  something that isn't this site's skill, the skill goes into `<name>-site` instead; when a server's name belongs to
  another entry in `.mcp.json`, `.cursor/mcp.json` or `.codex/config.toml`, the site is added as `<name>-site`. It says
  so, and connecting again updates the same one.

## 0.3.3 - 2026-10-30

- **`nibble disconnect` leaves no empty files behind:** when the site's server was the only one in `.mcp.json`,
  `.cursor/mcp.json` or `.codex/config.toml`, the file goes, and so do the project's `.codex`, `.cursor` and `skills`
  folders it leaves empty. It finds a server by its address as well as its name, so one added under an older name is
  removed too. Files with other servers, and folders in your home directory, are kept.

## 0.3.2 - 2026-10-30

- **A site's MCP server doesn't repeat "nibble" in its name:** nibble.ink's is `nibble-ink`, where it was
  `nibble-nibble-ink`; other sites keep `nibble-<host>`. `nibble disconnect <site>` still removes one under the old
  name, so run it, then `nibble connect <site>`.

## 0.3.1 - 2026-10-30

- **`nibble connect` asks which apps to set up:** on a terminal it lists Claude Code, Codex, Cursor, Claude and ChatGPT,
  marks those found on this computer and offers those as the default, instead of setting up every app found. `--client`
  still sets up exactly the apps named without asking, and without a terminal or `--client` it writes nothing and
  prints the command to run. The offer after `nibble auth login` asks the same question.

## 0.3.0 - 2026-10-30

- **`nibble connect [site]` and `nibble disconnect [site]`:** connect the AI apps found (Claude Code, Codex, Cursor) to
  a site in one step, signing in first if needed, and undo it. A site's MCP server and skill install into the current
  project only, since its tools and guide matter only where you work on that site; a folder that is another Nibble
  site's project is refused. `--client` chooses apps. `disconnect` also removes the user-wide entries older versions
  wrote. `nibble mcp install` and `nibble skill install` follow the same rule, and `skill sync` looks in the current
  project. `nibble auth login` now prints `nibble connect <site>` where it can't ask.
- **If you connected an app with 0.2.0,** its MCP server and skill were installed for your whole account, so they load
  in every project and `skill sync` no longer refreshes them. Run `nibble disconnect <site>` once to remove them, then
  `nibble connect <site>` in the folder where you work on that site.

## 0.2.0 - 2026-10-29

- **Upload a file from your computer:** `nibble remote upload-asset --file photo.jpg` streams it to the site and makes
  the asset from it; Tab completes the path.
- **Approvals are waited for:** when a change needs the person's approval, nibble prints their link and waits until
  it's made, then prints the result. `--no-wait` returns at once with the link, and `--json` or a pipe never waits;
  a request left waiting exits with status 2, so a script doesn't take it for done or failed.
- **`nibble remote` lists what this connection can't do,** with the access each needs, and says so when you call one,
  where it used to say there was no such operation.
- **`nibble auth login` offers to connect AI apps:** Claude Code, Codex or Cursor, if installed, are connected over MCP
  with the site's guide installed as a skill; the commands are printed when there's no terminal to ask in.
- **`nibble doctor` says when the sign-in ends** if it goes unused.
- **Works with sites on management API version 1 or 2.** Uploading with `--file` needs a site on version 2, and
  says so otherwise.
- **`nibble new` installs without the shell's `NIBBLE_THEME`,** which names a theme the new site doesn't have.

## 0.1.2 - 2026-10-29

- **A new site's tasks complete straight away:** `nibble new` learns them once the install finishes, where they used
  to appear only after the first task had run.
- **Tasks complete with their whole descriptions,** no longer cut off at 80 columns.
- **`nibble new` says how to turn on Tab completion** when no shell loads it yet.

## 0.1.1 - 2026-09-28

- **Builds for macOS,** on Apple Silicon and Intel, installed by the same script as on Linux.
- **Builds come as `.tar.gz`,** which minimal servers and containers can unpack without installing `xz`.
- **`nibble new` says how to get what's missing:** mise for Ruby and Node, which distributions package too old for
  Nibble, and this system's own packages for the rest — Homebrew, apt, or pacman on Arch Linux and its derivatives.
- **`nibble new` checks Ruby and Node against what the release needs** before installing anything, and says which
  version to get, where it used to fail inside `bundle install` or `npm install`.
- **`nibble new` asks for another name when the folder is taken,** and names the version it unpacked.
- **`nibble new` works in a folder only you can open,** under a name no one can create first, so the release can't be
  swapped between checking it and installing it.

## 0.1.0 - 2026-09-28

- **Start a site:** `nibble new` checks this computer, fetches Nibble's latest release, verifies it against its
  published checksum, and installs it.
- **Run a site's tasks:** inside a site, `nibble check` runs `bin/rails nibble:check`, and so on for every task.
- **Work on any number of sites and accounts:** `nibble auth login` signs in through the site in your browser, or with
  a code where there's none; sign-ins are kept in the system keychain. `nibble remote` runs whatever the site's
  management API offers this connection, with dry runs and structured errors.
- **Connect AI apps:** `nibble mcp install` points Claude Code, Codex or Cursor at a site's MCP server, and
  `nibble skill install` gives them the site's own guide as a skill.
- **Readable on a terminal:** tables, records as aligned fields, and colour for headings, operations, changes and
  errors, with the site's hint under each error. `NO_COLOR` turns it off; pipes, files and `--json` never get it.
- **Completion for bash, zsh, fish and PowerShell** that knows the site: its tasks, your sites and accounts, and the
  operations this connection can run with their arguments and values, such as the collections you may use and the
  blueprints of the one you named. Pressing Tab never contacts the site or runs anything in the folder.
- **`--pick` prints the bare value,** even when piped, so `id=$(nibble remote create-entry … --pick id)` works.
- **Checks the site speaks the same management API,** and says whether the CLI or the site needs upgrading.
