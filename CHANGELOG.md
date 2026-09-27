# Changelog

## Unreleased

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
- **`--pick` prints the bare value,** even when piped, so `id=$(nibble remote create-entry … --pick id)` works.
- **Checks the site speaks the same management API,** and says whether the CLI or the site needs upgrading.
