[private]
default:
    @just --list

# Formatting, lints, unit tests, then the contract test against a Nibble checkout (.. unless given)
test *nibble:
    script/ci {{nibble}}

# An optimised build for this computer, at target/release/nibble
build:
    cargo build --release

# Build, then install nibble into ~/.local/bin
install: build
    #!/usr/bin/env bash
    set -euo pipefail
    bin="$HOME/.local/bin"
    mkdir -p "$bin"
    install -m 755 target/release/nibble "$bin/nibble"
    echo "installed $("$bin/nibble" --version) at $bin/nibble"
    case ":$PATH:" in
      *":$bin:"*)
        found="$(command -v nibble)"
        if [[ "$found" == "$bin/nibble" ]]; then
          echo "~/.local/bin is on your PATH, so you're all set: run nibble"
        else
          echo "~/.local/bin is on your PATH, but $found comes first; remove it, or put ~/.local/bin earlier in PATH"
        fi ;;
      *)
        case "$(basename "${SHELL:-sh}")" in
          zsh) rc="~/.zshrc" line='export PATH="$HOME/.local/bin:$PATH"' ;;
          bash) rc="~/.bashrc" line='export PATH="$HOME/.local/bin:$PATH"' ;;
          fish) rc="~/.config/fish/config.fish" line='fish_add_path ~/.local/bin' ;;
          *) rc="your shell's startup file" line='export PATH="$HOME/.local/bin:$PATH"' ;;
        esac
        echo "~/.local/bin isn't on your PATH, so your shell won't find nibble yet. Add this line to $rc:"
        echo "  $line"
        echo "then open a new terminal, or run it once in this one." ;;
    esac

# Release a version: checks everything, commits, tags and offers to push; the pushed tag makes GitHub build it
release version:
    script/release {{version}}
