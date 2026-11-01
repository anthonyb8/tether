# tether

Your laptop and phone, tethered to the box your code runs on.

## Install

On Linux or macOS (x86_64 or arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/anthonyb8/tether/main/install.sh | sh
```

It installs the latest release to `~/.local/bin` when that is on PATH, else `/usr/local/bin`, after checking the download against its published SHA-256.
On Linux it also adds `tether browser` to your app launcher (rofi, GNOME, KDE and the like), through `~/.local/share/applications/tether-browser.desktop`.
Set `TETHER_VERSION=v0.0.1` before `sh` to install a specific release.
From source: `cargo install --git https://github.com/anthonyb8/tether`.

## Connect the laptop

On the laptop:

```sh
tether connect [host]      # connects in the background, now and at every login
tether connect --stop      # disconnects, and stops connecting at login
tether connect --foreground [host]   # runs in this terminal instead, until Ctrl-C
tether status              # connected to server, in the background, for 2h 5m
```

`tether connect` sets itself up as a login service, a systemd user unit on Linux or a launchd agent on macOS, starts it, and returns once the connection is up.
Running it again, say with another host, replaces the service.
Its messages go to `journalctl --user -u tether-connect` on Linux and `~/Library/Logs/tether-connect.log` on macOS.
`tether status` on the laptop says whether it is connected, still connecting, or not running, and names anything else holding the proxy port, such as an old `devproxy`.

The connection is one SSH connection to the server carrying a SOCKS proxy on `127.0.0.1:1080`, in which `localhost` is the server's.
It reconnects on its own after sleep or a dropped network, waiting 1 s and doubling to 30 s between tries.
It uses the system `ssh`, so the host is anything your `~/.ssh/config` knows, Tailscale SSH included, and needs key, agent or Tailscale authentication since it never prompts.
On Linux, `tether connect` passes your session's display and `SSH_AUTH_SOCK` to the service, so it can open browser windows and use your agent.
If tabs stop opening after you log in again, your desktop is not sharing its display with systemd: have it run `dbus-update-activation-environment --systemd DISPLAY WAYLAND_DISPLAY` at startup, as most desktops already do.

For a browser where `localhost` is the server's:

```sh
tether browser [host]
```

It runs `tether connect` first if nothing is connected, and opens a Chromium-family browser (Chromium, Chrome, Brave or Edge) with its own profile in `~/.config/tether/chromium`, or Firefox with its own profile in `~/.config/tether/firefox` when no Chromium-family browser is installed or `browser` names Firefox.
In that profile every request goes through the proxy, `localhost` included, so every dev server, an API the page calls on `127.0.0.1` and OAuth callbacks to `localhost` work unchanged; the rest of the browser is untouched.
`tether open` on the server opens its URL as a new tab in that profile.

Each setting comes from the command line, then the environment, then `~/.config/tether/config.toml`, then the default:

| Setting | Env | Config key | Default |
|---|---|---|---|
| SSH host of the server | `TETHER_HOST` | `host` | `server` |
| Local SOCKS port | `TETHER_PORT` | `port` | `1080` |
| Browser, a command or path | | `browser` | the first Chromium-family browser found, else Firefox |

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

## Make an app work on the phone

An app opened on the phone is at `https://<host>.<tailnet>.ts.net:<port>`, not `localhost`.
Anything the app bakes `localhost` into breaks there: its API URL, its auth redirects, a dev server's list of allowed hosts.
No tool can fix that from outside, so each app takes a small, one-time change, the same shape in every stack.

**First, call the API on a relative path.**
When the page calls `/api/...` and the dev server proxies that to the backend, one origin serves everything, and most of the rest below goes away.
Share only the dev server's port.

**Then allow the tailnet address.**
`tether origin <port>` prints it, so a start script can pass it in rather than hard-coding a tailnet name.
The usual settings, by stack:

| Stack | Setting |
|---|---|
| Vite | `server.allowedHosts: ['.ts.net']`, and `server.proxy: { '/api': 'http://localhost:8000' }` for the relative API path |
| Next.js | `allowedDevOrigins: ['*.ts.net']` in `next.config.js`, and `rewrites()` from `/api/:path*` to the backend |
| Supabase | the app's origin in `additional_redirect_urls` under `[auth]` in `supabase/config.toml`, so sign-in redirects back to it; reach the API through the dev server's proxy, or `tether share 54321` and point the client at `tether origin 54321` |
| Rails | `config.hosts << ".ts.net"` in `config/environments/development.rb` |
| Django | `".ts.net"` in `ALLOWED_HOSTS`, and the app's origin from `tether origin` in `CSRF_TRUSTED_ORIGINS` |

Check them against your framework's version: these settings move between releases.

## Wire it into other tools

tether needs no plugin anywhere; it plugs in through plain commands.

- **`$BROWSER`.** On the server, `export BROWSER="tether open"` makes a tool that honours `$BROWSER` and splits it into words, as `gh` does, open its page on the laptop.
  For a tool that takes `$BROWSER` as a single program, point it at a two-line script that runs `exec tether open "$@"`.
  When no laptop is connected, `tether open` exits 1 with `tether: no browser connected`, so a tool that checks the exit status can print the URL instead.
- **Scripts and agents.** `tether open "$url" || echo "$url"` opens a page where you are, or falls back to printing it; `tether notify "done"` says a long task finished.
- **harness.** `harness stack up` prints the app's URL; with `tether browser` on the laptop it opens as printed, since `localhost` there is the server's.
  tether replaces harness's `devproxy`, with the same default host and port, so `tether browser` works where `devproxy browser` did.
- **treetop.** treetop's `o` opens a tree's port in the desktop's browser, or copies the URL to the clipboard over SSH; with `tether browser` open on the laptop, that copied `localhost` URL loads as printed.

## Release

Bump `version` in `Cargo.toml`, then push a matching tag:

```sh
git tag v0.0.1 && git push origin v0.0.1
```

`.github/workflows/release.yml` refuses a tag that does not match `Cargo.toml`, runs the tests, builds static musl binaries for Linux and native binaries for macOS, each for x86_64 and arm64, and publishes them with checksums as a GitHub release.
