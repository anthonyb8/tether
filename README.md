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

## Share a port with your phone

On the server, with a dev server on port 5173:

```sh
tether share 5173      # prints https://<host>.<tailnet>.ts.net:5173 and a QR code
tether shares          # lists what is shared
tether origin 5173     # prints the address alone, for a start script's allowed hosts
tether unshare 5173
```

The phone needs only the Tailscale app, signed in to the same tailnet.
`share` runs `tailscale serve`, so the port is reachable from your tailnet and never the public internet.
Once per tailnet, turn on MagicDNS and HTTPS Certificates at https://login.tailscale.com/admin/dns; the first `share` on a tailnet without Serve prints the link to enable it and waits.

## Release

Bump `version` in `Cargo.toml`, then push a matching tag:

```sh
git tag v0.0.1 && git push origin v0.0.1
```

`.github/workflows/release.yml` refuses a tag that does not match `Cargo.toml`, runs the tests, builds static musl binaries for Linux and native binaries for macOS, each for x86_64 and arm64, and publishes them with checksums as a GitHub release.
