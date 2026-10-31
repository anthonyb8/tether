# tether

Your laptop and phone, tethered to the box your code runs on.

## Install

On Linux or macOS (x86_64 or arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/anthonyb8/tether/main/install.sh | sh
```

It installs the latest release to `~/.local/bin` when that is on PATH, else `/usr/local/bin`, after checking the download against its published SHA-256.
Set `TETHER_VERSION=v0.0.1` before `sh` to install a specific release.
From source: `cargo install --git https://github.com/anthonyb8/tether`.

## Release

Bump `version` in `Cargo.toml`, then push a matching tag:

```sh
git tag v0.0.1 && git push origin v0.0.1
```

`.github/workflows/release.yml` refuses a tag that does not match `Cargo.toml`, runs the tests, builds static musl binaries for Linux and native binaries for macOS, each for x86_64 and arm64, and publishes them with checksums as a GitHub release.
