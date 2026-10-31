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

## Connect the laptop

On the laptop:

```sh
tether connect [host]
```

It holds one SSH connection to the server carrying a SOCKS proxy on `127.0.0.1:1080`, in which `localhost` is the server's, and reconnects on its own after sleep or a dropped network, waiting 1 s and doubling to 30 s between tries.
It uses the system `ssh`, so the host is anything your `~/.ssh/config` knows, and needs key or agent authentication since it never prompts.

For a browser where `localhost` is the server's:

```sh
tether browser [host]
```

It starts `tether connect` in the background if it is not already running, logging to `~/.cache/tether/connect.log`, and opens a Chromium-family browser (Chromium, Chrome, Brave or Edge) with its own profile in `~/.config/tether/chromium`, or Firefox with its own profile in `~/.config/tether/firefox` when no Chromium-family browser is installed or `browser` names Firefox.
In that profile every request goes through the proxy, `localhost` included, so every dev server, an API the page calls on `127.0.0.1` and OAuth callbacks to `localhost` work unchanged; the rest of the browser is untouched.
`tether open` on the server opens its URL as a new tab in that profile.

Each setting comes from the command line, then the environment, then `~/.config/tether/config.toml`, then the default:

| Setting | Env | Config key | Default |
|---|---|---|---|
| SSH host of the server | `TETHER_HOST` | `host` | `server` |
| Local SOCKS port | `TETHER_PORT` | `port` | `1080` |
| Browser, a command or path | | `browser` | the first Chromium-family browser found, else Firefox |

### Keep it running

To have `connect` start at login and stay up, install the user service from `service/`.

On Linux, with systemd:

```sh
install -Dm644 service/tether-connect.service ~/.config/systemd/user/tether-connect.service
systemctl --user enable --now tether-connect
```

On macOS:

```sh
cp service/dev.tether.connect.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.tether.connect.plist
```

Both run `~/.local/bin/tether connect`, where `install.sh` puts it, and read the host from `~/.config/tether/config.toml`.
A systemd user service does not see an `ssh-agent` started by your shell, so on Linux use a key without a passphrase for the server, or an agent the session exports to systemd (`systemctl --user import-environment SSH_AUTH_SOCK`).

## Open tabs on the laptop from the server

While a laptop is connected, on the server:

```sh
tether open http://localhost:5173   # opens a tab on the laptop; exits 1 with a reason if it cannot
tether notify "tests passed"         # a desktop notification on the laptop, same exit codes as open
tether status                       # connected: <laptop>, for 2h 5m; exits 1 if no laptop is connected
```

Notifications use `notify-send` on Linux (from libnotify) and `osascript` on macOS.
`open` takes only http and https URLs and waits up to 2 seconds for the laptop to confirm, so it works as `BROWSER="tether open"` and a caller can fall back when it exits 1 with `tether: no browser connected`.
`connect` runs `tether relay` on the server over the same SSH connection, so the server needs tether installed too; without it, `connect` keeps the proxy and says so.
When several laptops connect, the most recent gets the tabs, and the one before takes over again when it leaves.

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
