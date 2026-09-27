# Changelog

## Unreleased

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
