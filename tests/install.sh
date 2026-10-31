#!/bin/sh
# Runs install.sh the way a user does, against a fake release served by a
# stand-in for curl, and checks it installs a working tether to ~/.local/bin
# and refuses a download whose checksum does not match.
#   tests/install.sh path/to/tether
set -eu

BINARY="$1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

case "$(uname -s)" in Linux) OS=linux ;; Darwin) OS=darwin ;; esac
case "$(uname -m)" in x86_64 | amd64) ARCH=amd64 ;; aarch64 | arm64) ARCH=arm64 ;; esac
VERSION=v9.9.9
NAME="tether-${VERSION}-${OS}-${ARCH}.tar.gz"

# The release, laid out as GitHub serves its downloads.
RELEASE="$WORK/release"
mkdir -p "$RELEASE"
cp "$BINARY" "$WORK/tether"
tar czf "$RELEASE/$NAME" -C "$WORK" tether
(cd "$RELEASE" && shasum -a 256 "$NAME" > "$NAME.sha256")

# curl that answers "-o <file> <url>" from $RELEASE by the URL's last segment.
mkdir -p "$WORK/shims"
cat > "$WORK/shims/curl" <<SHIM
#!/bin/sh
out=""; url=""
while [ \$# -gt 0 ]; do
  case "\$1" in -o) out="\$2"; shift 2 ;; -*) shift ;; *) url="\$1"; shift ;; esac
done
cp "$RELEASE/\${url##*/}" "\$out"
SHIM
chmod +x "$WORK/shims/curl"

run_install() {
  HOME="$WORK/home" PATH="$WORK/shims:$WORK/home/.local/bin:$PATH" TETHER_VERSION="$VERSION" \
    sh "$ROOT/install.sh"
}

fail() { echo "FAIL: $*" >&2; exit 1; }

# A good download installs to ~/.local/bin, creating it, and runs.
mkdir -p "$WORK/home"
run_install > "$WORK/out" 2>&1 || { cat "$WORK/out" >&2; fail "install exited non-zero"; }
[ -x "$WORK/home/.local/bin/tether" ] || fail "tether not installed to ~/.local/bin"
"$WORK/home/.local/bin/tether" --help | grep -q "share" || fail "installed tether does not print its help"
echo "ok - installs to ~/.local/bin"

# A tampered download is refused and installs nothing.
rm "$WORK/home/.local/bin/tether"
printf 'tampered' >> "$RELEASE/$NAME"
if run_install > "$WORK/out" 2>&1; then fail "install accepted a bad checksum"; fi
grep -q "Checksum mismatch" "$WORK/out" || { cat "$WORK/out" >&2; fail "no checksum error shown"; }
[ ! -e "$WORK/home/.local/bin/tether" ] || fail "tether installed despite a bad checksum"
echo "ok - refuses a download whose checksum does not match"
