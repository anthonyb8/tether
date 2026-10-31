#!/bin/sh
# Installs the latest tether release on Linux or macOS:
#   curl -fsSL https://raw.githubusercontent.com/anthonyb8/tether/main/install.sh | sh
# Set TETHER_VERSION (e.g. v0.0.1) to install a specific release instead.
set -eu

REPO="anthonyb8/tether"

case "$(uname -s)" in
  Linux) OS="linux" ;;
  Darwin) OS="darwin" ;;
  *) echo "tether ships Linux and macOS builds only; elsewhere: cargo install --git https://github.com/${REPO}" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) ARCH="amd64" ;;
  aarch64 | arm64) ARCH="arm64" ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

# Prefer ~/.local/bin when it is on PATH (no sudo needed), else /usr/local/bin.
if echo "$PATH" | tr ':' '\n' | grep -qx "$HOME/.local/bin"; then
  INSTALL_DIR="$HOME/.local/bin"
else
  INSTALL_DIR="/usr/local/bin"
fi

# The latest release, read from the redirect GitHub serves for it rather than
# the API, which rate-limits anonymous callers.
VERSION="${TETHER_VERSION:-}"
if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
  VERSION="${VERSION##*/}"
fi
case "$VERSION" in
  v*) ;;
  *) echo "Could not determine the latest tether release" >&2; exit 1 ;;
esac

FILENAME="tether-${VERSION}-${OS}-${ARCH}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${FILENAME}"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading tether ${VERSION} for ${OS}/${ARCH}..."
curl -fsSL "$URL" -o "${TMPDIR}/${FILENAME}"
curl -fsSL "${URL}.sha256" -o "${TMPDIR}/${FILENAME}.sha256"

# macOS has shasum but not sha256sum; both read the same checksum file.
if command -v sha256sum >/dev/null; then
  CHECK="sha256sum -c"
else
  CHECK="shasum -a 256 -c"
fi
(cd "$TMPDIR" && $CHECK "${FILENAME}.sha256" >/dev/null 2>&1) || {
  echo "Checksum mismatch for ${FILENAME}; nothing was installed" >&2
  exit 1
}
tar xzf "${TMPDIR}/${FILENAME}" -C "$TMPDIR"

# ~/.local/bin can be on PATH before anything has created it.
if [ "$INSTALL_DIR" = "$HOME/.local/bin" ]; then
  mkdir -p "$INSTALL_DIR"
fi
if [ -w "$INSTALL_DIR" ]; then
  install -m 0755 "${TMPDIR}/tether" "${INSTALL_DIR}/tether"
else
  echo "Installing to ${INSTALL_DIR} (requires sudo)..."
  sudo install -d "$INSTALL_DIR"
  sudo install -m 0755 "${TMPDIR}/tether" "${INSTALL_DIR}/tether"
fi

echo "tether ${VERSION} installed to ${INSTALL_DIR}/tether"

# On Linux, a launcher entry for tether browser, so app launchers such as
# rofi, GNOME and KDE find it. It names the binary by its full path, since a
# launcher started at login may not have INSTALL_DIR on its PATH. Rewritten on
# every install, so it follows the binary if it moves.
if [ "$OS" = "linux" ]; then
  APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
  mkdir -p "$APPS_DIR"
  cat > "${APPS_DIR}/tether-browser.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=tether browser
GenericName=Web Browser
Comment=A browser whose localhost is the server's
Exec="${INSTALL_DIR}/tether" browser
Icon=web-browser
Terminal=false
Categories=Network;WebBrowser;Development;
Keywords=tether;proxy;localhost;server;
DESKTOP
  if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
  fi
  echo "Added tether browser to your app launcher"
fi
