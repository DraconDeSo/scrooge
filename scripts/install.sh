#!/usr/bin/env sh
# Installs the latest release of Scrooge on macOS and Linux.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/DraconDeSo/scrooge/main/scripts/install.sh | sh
set -eu

REPO="${SCROOGE_REPO:-DraconDeSo/scrooge}"
VERSION="${SCROOGE_VERSION:-latest}"
BIN_DIR="${SCROOGE_BIN_DIR:-$HOME/.local/bin}"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Error: '$1' is required to install Scrooge." >&2
    exit 1
  fi
}

need_cmd curl
need_cmd uname

OS="$(uname -s)"
ARCH="$(uname -m)"

if [ "$VERSION" = "latest" ]; then
  API_URL="https://api.github.com/repos/$REPO/releases/latest"
else
  case "$VERSION" in
    v*) TAG="$VERSION" ;;
    *)  TAG="v$VERSION" ;;
  esac
  API_URL="https://api.github.com/repos/$REPO/releases/tags/$TAG"
fi

TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t scrooge-install)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

echo "==> Fetching release metadata from $REPO..."
RELEASE_JSON="$TMP_DIR/release.json"
curl -fsSL -H "Accept: application/vnd.github+json" -H "User-Agent: scrooge-installer" "$API_URL" -o "$RELEASE_JSON"

find_asset_url() {
  pattern="$1"
  grep -oE "\"browser_download_url\"[[:space:]]*:[[:space:]]*\"[^\"]*${pattern}\"" "$RELEASE_JSON" \
    | head -n 1 \
    | sed -E 's/.*"([^"]+)"$/\1/'
}

verify_checksum() {
  file_path="$1"
  file_name="$(basename "$file_path")"
  sums_url="$(find_asset_url "SHA256SUMS\\.txt" || true)"
  if [ -z "$sums_url" ]; then
    return 0
  fi
  curl -fsSL "$sums_url" -o "$TMP_DIR/SHA256SUMS.txt"
  expected="$(grep " ${file_name}\$" "$TMP_DIR/SHA256SUMS.txt" | awk '{print $1}' | head -n 1 || true)"
  if [ -z "$expected" ]; then
    return 0
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file_path" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$file_path" | awk '{print $1}')"
  else
    return 0
  fi
  if [ "$expected" != "$actual" ]; then
    echo "Error: SHA-256 mismatch for $file_name (expected $expected, got $actual)" >&2
    exit 1
  fi
  echo "==> Verified SHA-256 checksum for $file_name."
}

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64|aarch64) PATTERN="_aarch64\\.dmg" ;;
      x86_64)        PATTERN="_x64\\.dmg" ;;
      *) echo "Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
    esac
    URL="$(find_asset_url "$PATTERN")"
    if [ -z "$URL" ]; then
      echo "Error: could not find macOS DMG asset in release." >&2
      exit 1
    fi
    DMG="$TMP_DIR/$(basename "$URL")"
    echo "==> Downloading $(basename "$URL")..."
    curl -fL "$URL" -o "$DMG"
    verify_checksum "$DMG"
    MOUNT_DIR="$TMP_DIR/mnt"
    mkdir -p "$MOUNT_DIR"
    hdiutil attach "$DMG" -nobrowse -quiet -mountpoint "$MOUNT_DIR"
    APP_DEST="/Applications/Scrooge.app"
    if [ ! -w "/Applications" ]; then
      APP_DEST="$HOME/Applications/Scrooge.app"
      mkdir -p "$HOME/Applications"
    fi
    rm -rf "$APP_DEST"
    cp -R "$MOUNT_DIR/Scrooge.app" "$APP_DEST"
    hdiutil detach "$MOUNT_DIR" -quiet || true
    xattr -dr com.apple.quarantine "$APP_DEST" 2>/dev/null || true
    echo "==> Installed Scrooge to $APP_DEST"
    ;;

  Linux)
    if [ "$ARCH" != "x86_64" ] && [ "$ARCH" != "amd64" ]; then
      echo "Unsupported Linux architecture: $ARCH (x86_64 required)" >&2
      exit 1
    fi
    if command -v apt-get >/dev/null 2>&1 && command -v sudo >/dev/null 2>&1; then
      URL="$(find_asset_url "_amd64\\.deb")"
      if [ -n "$URL" ]; then
        DEB="$TMP_DIR/$(basename "$URL")"
        echo "==> Downloading $(basename "$URL")..."
        curl -fL "$URL" -o "$DEB"
        verify_checksum "$DEB"
        sudo apt-get install -y "$DEB"
        echo "==> Installed Scrooge (.deb)."
        exit 0
      fi
    elif command -v dnf >/dev/null 2>&1 && command -v sudo >/dev/null 2>&1; then
      URL="$(find_asset_url "\\.x86_64\\.rpm")"
      if [ -n "$URL" ]; then
        RPM="$TMP_DIR/$(basename "$URL")"
        echo "==> Downloading $(basename "$URL")..."
        curl -fL "$URL" -o "$RPM"
        verify_checksum "$RPM"
        sudo dnf install -y "$RPM"
        echo "==> Installed Scrooge (.rpm)."
        exit 0
      fi
    fi

    URL="$(find_asset_url "_amd64\\.AppImage")"
    if [ -z "$URL" ]; then
      echo "Error: could not find Linux AppImage in release." >&2
      exit 1
    fi
    APPIMAGE="$TMP_DIR/$(basename "$URL")"
    echo "==> Downloading $(basename "$URL")..."
    curl -fL "$URL" -o "$APPIMAGE"
    verify_checksum "$APPIMAGE"
    mkdir -p "$BIN_DIR"
    cp "$APPIMAGE" "$BIN_DIR/scrooge"
    chmod +x "$BIN_DIR/scrooge"
    echo "==> Installed Scrooge AppImage to $BIN_DIR/scrooge"
    ;;

  *)
    echo "Unsupported operating system: $OS" >&2
    exit 1
    ;;
esac
