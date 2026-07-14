#!/usr/bin/env bash
# tablo installer — macOS & Linux. Pulls the latest build from GitHub Releases.
#   curl -fsSL https://raw.githubusercontent.com/unravel-team/tablo/main/install.sh | bash
# (or run it from a checkout). No build toolchain required.
set -euo pipefail

REPO="unravel-team/tablo"
API="https://api.github.com/repos/${REPO}/releases/latest"

say() { printf '\033[1;32m==>\033[0m %s\n' "$1"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

# First asset download URL whose filename matches the given regex.
asset_url() {
  curl -fsSL "$API" \
    | grep -o '"browser_download_url": *"[^"]*"' \
    | sed 's/.*"\(https[^"]*\)"/\1/' \
    | grep -iE "$1" \
    | head -1
}

os="$(uname -s)"
case "$os" in
  Darwin)
    say "Fetching the latest tablo release…"
    url="$(asset_url '\.dmg$')" || true
    [ -n "${url:-}" ] || die "no macOS .dmg in the latest release yet"
    tmp="$(mktemp -d)"
    trap 'hdiutil detach "$tmp/mnt" -quiet 2>/dev/null || true; rm -rf "$tmp"' EXIT
    say "Downloading $(basename "$url")…"
    curl -fsSL "$url" -o "$tmp/tablo.dmg"
    mkdir -p "$tmp/mnt"
    hdiutil attach "$tmp/tablo.dmg" -nobrowse -quiet -mountpoint "$tmp/mnt"
    app="$(/usr/bin/find "$tmp/mnt" -maxdepth 1 -name '*.app' | head -1)"
    [ -n "$app" ] || die "no .app inside the dmg"
    say "Installing to /Applications…"
    rm -rf "/Applications/$(basename "$app")"
    cp -R "$app" /Applications/
    # Strip Gatekeeper quarantine so it opens without the unsigned-app prompt.
    xattr -dr com.apple.quarantine "/Applications/$(basename "$app")" 2>/dev/null || true
    say "Done — launch tablo from /Applications or Spotlight."
    ;;
  Linux)
    say "Fetching the latest tablo release…"
    url="$(asset_url '\.AppImage$')" || true
    [ -n "${url:-}" ] || die "no Linux .AppImage in the latest release yet"
    dest="${HOME}/.local/bin"
    mkdir -p "$dest"
    out="$dest/tablo.AppImage"
    say "Downloading $(basename "$url")…"
    curl -fsSL "$url" -o "$out"
    chmod +x "$out"
    say "Installed to $out"
    case ":$PATH:" in
      *":$dest:"*) : ;;
      *) say "Tip: add $dest to your PATH to run 'tablo.AppImage' from anywhere." ;;
    esac
    ;;
  *)
    die "unsupported OS: $os — on Windows use install.ps1"
    ;;
esac
