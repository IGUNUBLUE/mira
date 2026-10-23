#!/bin/sh
# Install mira on Linux:
#   curl -fsSL https://raw.githubusercontent.com/IGUNUBLUE/mira/main/install.sh | sh
#
# Downloads the latest release binary (signature and checksums verified) or, without a
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
DOCS="$PREFIX/share/doc/mira"
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
      rm -rf "$DOCS"
      rm -f "$PREFIX/share/mira/install.json"
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
has openssl || die "OpenSSL 3 is required to authenticate releases"
fetch() { curl --disable --proto =https --proto-redir =https --connect-timeout 10 --max-time 600 -fsSL "$@"; }
sha256() { openssl dgst -sha256 "$1" | sed 's/^.*= //'; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

install_files() { # <dir with mira, mira.desktop, mira.svg>
  mkdir -p "$BIN" "$APPS" "$ICONS"
  # Rename a sibling: replacing a running executable with install(1) can fail with ETXTBSY.
  staged="$(mktemp "$BIN/.mira-install-XXXXXX")"
  install -m 0755 "$1/mira" "$staged"
  mv -f "$staged" "$BIN/mira"
  mkdir -p "$PREFIX/share/mira"
  chmod 0700 "$PREFIX/share/mira"
  record="$(mktemp "$PREFIX/share/mira/.install-XXXXXX")"
  printf '{"format":1,"binary_sha256":"%s"}\n' "$(sha256 "$BIN/mira")" > "$record"
  chmod 0600 "$record"
  mv -f "$record" "$PREFIX/share/mira/install.json"
  install -m 0644 "$1/mira.svg" "$ICONS/mira.svg"
  # Absolute Exec: launchers don't always have ~/.local/bin on their PATH.
  sed "s|^Exec=.*|Exec=$BIN/mira|" "$1/mira.desktop" > "$APPS/mira.desktop"
  chmod 0644 "$APPS/mira.desktop"
  # Licenses: mira's own and, in release builds, its dependencies' notices.
  mkdir -p "$DOCS"
  for f in LICENSE-MIT LICENSE-APACHE THIRD_PARTY_LICENSES.html; do
    if [ -f "$1/$f" ]; then install -m 0644 "$1/$f" "$DOCS/$f"; fi
  done
  if has update-desktop-database; then update-desktop-database "$APPS" >/dev/null 2>&1 || true; fi
  if has gtk-update-icon-cache; then gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true; fi
}

from_release() {
  if [ "$VERSION" = "latest" ]; then
    TAG="$(fetch "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)" || TAG=""
  else
    TAG="$VERSION"
  fi
  [ -n "$TAG" ] || return 1
  case "$TAG" in v[0-9]*.[0-9]*.[0-9]*) ;; *) die "invalid release tag" ;; esac
  case "$TAG" in *[!v0-9.]*) die "invalid release tag" ;; esac
  NAME="mira-$TAG-$ARCH-linux"
  URL="https://github.com/$REPO/releases/download/$TAG/$NAME.tar.gz"
  say "downloading $NAME"
  fetch "$URL" -o "$TMP/$NAME.tar.gz" || return 1
  case "$TAG" in
    v0.[0-9].*|v0.10.*)
      # Explicit older releases predate signed metadata; retain checksum compatibility.
      fetch "$URL.sha256" -o "$TMP/$NAME.tar.gz.sha256" || die "missing checksum for $NAME"
      expected="$(sed 's/ .*//' "$TMP/$NAME.tar.gz.sha256")"
      [ "$(sha256 "$TMP/$NAME.tar.gz")" = "$expected" ] || die "checksum mismatch for $NAME"
      ;;
    *)
      fetch "${URL%.tar.gz}.update.json" -o "$TMP/update.json" || die "missing signed metadata"
      fetch "${URL%.tar.gz}.update.json.sig" -o "$TMP/update.sig" || die "missing update signature"
      cat > "$TMP/public.pem" <<'PUBLIC_KEY'
-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEA5qQs74HILRD0c4/fjXSDnZq0zMghzzl/09EZDUB8FtY=
-----END PUBLIC KEY-----
PUBLIC_KEY
      openssl pkeyutl -verify -pubin -rawin -inkey "$TMP/public.pem" -in "$TMP/update.json" -sigfile "$TMP/update.sig" >/dev/null || die "invalid release signature"
      field() { sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p" "$TMP/update.json"; }
      [ "$(field version)" = "${TAG#v}" ] || die "signed version mismatch"
      [ "$(field arch)" = "$ARCH" ] || die "signed architecture mismatch"
      [ "$(field archive)" = "$NAME.tar.gz" ] || die "signed archive mismatch"
      [ "$(sha256 "$TMP/$NAME.tar.gz")" = "$(field archive_sha256)" ] || die "signed archive checksum mismatch"
      ;;
  esac
  tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"
  case "$TAG" in
    v0.[0-9].*|v0.10.*) ;;
    *) [ "$(sha256 "$TMP/$NAME/mira")" = "$(field binary_sha256)" ] || die "signed binary checksum mismatch" ;;
  esac
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
  cp "${CARGO_TARGET_DIR:-$TMP/src/target}/release/mira" "$TMP/src/assets/mira.desktop" "$TMP/src/assets/mira.svg" \
    "$TMP/src/LICENSE-MIT" "$TMP/src/LICENSE-APACHE" "$TMP/pkg/"
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
