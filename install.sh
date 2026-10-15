#!/bin/sh
# Install mira on Linux:
#   curl -fsSL https://raw.githubusercontent.com/IGUNUBLUE/mira/main/install.sh | sh
#
# Downloads the latest release binary (checksum verified) or, without a
# release for your machine, builds it from source with cargo. Installs into
# ~/.local (binary, desktop entry, icon); nothing needs root.
#
# Options (after `sh -s --` when piping):
#   --version vX.Y.Z   install that release instead of the latest
#   --source           build from source even when a binary exists
#   --uninstall        remove what this script installed
# Environment: MIRA_PREFIX (default ~/.local)
set -eu

REPO="IGUNUBLUE/mira"
PREFIX="${MIRA_PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor/scalable/apps"
VERSION="latest"
FROM_SOURCE=0

say() { printf '%s\n' "mira: $*"; }
die() { printf '%s\n' "mira: error: $*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
    --source) FROM_SOURCE=1; shift ;;
    --uninstall)
      rm -f "$BIN/mira" "$APPS/mira.desktop" "$ICONS/mira.svg"
      say "removed mira from $PREFIX (your config and state in ~/.config/mira and ~/.local/state/mira are kept)"
      exit 0 ;;
    -h|--help) sed -n '2,15p' "$0" 2>/dev/null || true; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ "$(uname -s)" = "Linux" ] || die "mira currently supports Linux only"
case "$(uname -m)" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) ARCH=""; FROM_SOURCE=1 ;;
esac
has curl || die "curl is required"
has tar || die "tar is required"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

install_files() { # <dir with mira, mira.desktop, mira.svg>
  mkdir -p "$BIN" "$APPS" "$ICONS"
  install -m 0755 "$1/mira" "$BIN/mira"
  install -m 0644 "$1/mira.svg" "$ICONS/mira.svg"
  # Absolute Exec: launchers don't always have ~/.local/bin on their PATH.
  sed "s|^Exec=.*|Exec=$BIN/mira|" "$1/mira.desktop" > "$APPS/mira.desktop"
  chmod 0644 "$APPS/mira.desktop"
  if has update-desktop-database; then update-desktop-database "$APPS" >/dev/null 2>&1 || true; fi
  if has gtk-update-icon-cache; then gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true; fi
}

from_release() {
  if [ "$VERSION" = "latest" ]; then
    TAG="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)" || TAG=""
  else
    TAG="$VERSION"
  fi
  [ -n "$TAG" ] || return 1
  NAME="mira-$TAG-$ARCH-linux"
  URL="https://github.com/$REPO/releases/download/$TAG/$NAME.tar.gz"
  say "downloading $NAME"
  curl -fsSL "$URL" -o "$TMP/$NAME.tar.gz" || return 1
  curl -fsSL "$URL.sha256" -o "$TMP/$NAME.tar.gz.sha256" || die "missing checksum for $NAME"
  if has sha256sum; then
    (cd "$TMP" && sha256sum -c "$NAME.tar.gz.sha256" >/dev/null) || die "checksum mismatch for $NAME"
  else
    say "warning: sha256sum not found, checksum not verified"
  fi
  tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"
  install_files "$TMP/$NAME"
  say "installed mira $TAG to $BIN/mira"
}

from_source() {
  has cargo || die "no release binary for this machine and cargo is not installed (https://rustup.rs)"
  has git || die "git is required to build from source"
  say "building from source (a few minutes)…"
  REF="main"; [ "$VERSION" = "latest" ] || REF="$VERSION"
  git clone --quiet --depth 1 --branch "$REF" "https://github.com/$REPO.git" "$TMP/src"
  cargo build --release --locked --manifest-path "$TMP/src/Cargo.toml"
  mkdir -p "$TMP/pkg"
  cp "$TMP/src/target/release/mira" "$TMP/src/assets/mira.desktop" "$TMP/src/assets/mira.svg" "$TMP/pkg/"
  install_files "$TMP/pkg"
  say "built and installed mira ($REF) to $BIN/mira"
}

if [ "$FROM_SOURCE" = 1 ] || ! from_release; then
  from_source
fi

# What mira works with; none of these block the install.
has git || say "note: install git, mira's Changes panel and history use it"
has herdr || say "note: install herdr (https://herdr.dev) for agent tracking; without it mira runs a plain shell"
if has fc-list && ! fc-list 2>/dev/null | grep -qi "nerd font"; then
  say "note: a Nerd Font (https://www.nerdfonts.com) gives mira its icons"
fi
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) say "note: add $BIN to your PATH to run 'mira' from a terminal" ;;
esac
say "done. Open mira from your app launcher, or run: mira [folder]"
