# Installation (without a panel)

## Requirements

- Linux x86_64 with glibc 2.31 or newer
- `steamclient.so` from SteamCMD – the server signs in to Steam anonymously through it
- Players and the server must run the **same ReSkate version** (this build: 1.1.2)

## 1. Install SteamCMD

SteamCMD provides `steamclient.so`. On 64-bit Debian/Ubuntu it needs the 32-bit GCC runtime:

```bash
sudo apt install lib32gcc-s1        # Debian / Ubuntu
```

```bash
mkdir -p ~/steamcmd && cd ~/steamcmd
curl -sSL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz | tar -xz
./steamcmd.sh +quit
mkdir -p ~/.steam/sdk64
ln -sf ~/steamcmd/linux64/steamclient.so ~/.steam/sdk64/steamclient.so
```

The server looks for `steamclient.so` in `~/.steam/sdk64/`. If a `steamclient.so` lies next to the
server binary and `~/.steam/sdk64/` has none, the server creates the link there on startup.

## 2. Unpack and start

```bash
tar -xzf ReSkateServer-linux-x64.tar.gz
cd ReSkateServer-linux-x64
./ReSkateServer
```

Keep `ReSkateServer` and `libsteam_api.so` in the same folder.

On the first start the server writes `ReSkateServer.json` and prints its join code:

```
Wrote a default ReSkateServer.json. Edit it to name the server and add admins.
Signing in to Steam...
ReSkate server is up on San Vansterdam for 16 players.
Steam ID 9029402372656333x, public IP 203.0.113.10.
Join code: 9029402372656333x-ff2f9e71a5ad5603
No admins yet: type "admin add <SteamID64>" to add one.
```

Edit at least `name` and `admins` (see [configuration.md](configuration.md)) and restart, or set them
live from the console (`name My Server`, `admin add 7656119…`). Every change made from the console is
saved back to the file.

The server gets a new Steam ID – and therefore a new join code – on every start. The server browser
always finds it by name.

## Command line options

```
ReSkateServer [--config <file>] [--port <port>] [--query-port <port>] [--no-update]
```

| Option | Effect |
|---|---|
| `--port <port>` | game port (default 27015) |
| `--query-port <port>` | query port (default 27016) |
| `--config <file>` | use another config file (default `ReSkateServer.json` next to the binary) |
| `--no-update` | do not check for new ReSkate releases |
| `--help` | print usage |

Commands are read from stdin, so the server works in `tmux`, `screen`, Docker and panels.

## Stopping

Type `quit`, press Ctrl+C or send SIGTERM / SIGHUP. The server signs out of Steam before exiting.

## Ports

| Port | Default | Protocol | Purpose |
|---|---|---|---|
| `--port` | 27015 | UDP | Steam game server port |
| `--query-port` | 27016 | UDP | Steam server queries (ping in the browser) |
| `--port` | 27015 | TCP | [Status page](configuration.md#status-page) (`/status`) |

Opening the UDP ports is optional: players connect through Steam's relay network. With open ports the
browser shows the ping and joins are a little faster. Open the TCP port if the status page should be
reachable from outside.

## Running as a systemd service

Because the console is stdin, run it inside `tmux`/`screen` if you want to type commands, or use a
plain service and manage it through the config file and in-game admin commands:

```ini
# /etc/systemd/system/reskate.service
[Unit]
Description=ReSkate dedicated server
After=network-online.target
Wants=network-online.target

[Service]
User=reskate
WorkingDirectory=/home/reskate/ReSkateServer-linux-x64
ExecStart=/home/reskate/ReSkateServer-linux-x64/ReSkateServer
Restart=on-failure
KillSignal=SIGTERM

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now reskate
journalctl -u reskate -f
```

SteamCMD has to be set up for the `reskate` user (`/home/reskate/.steam/sdk64/steamclient.so`).

## Files the server creates

| File | |
|---|---|
| `ReSkateServer.json` | configuration, written on first start and on every change |
| `ReSkateServer.log` | today's log |
| `logs/` | earlier days, `logs/ReSkateServer-<date>.log`; the last 14 days are kept |
| `Mods/` | custom maps (create it yourself) |
| `plugins/` | Lua plugins (create it yourself, see [plugins.md](plugins.md)) |

## Custom maps

Copy a custom map's mod folder from the game's `Mods` folder into `Mods/` next to the server. Only its
`reskate-levels.json` is read. The map can then be picked by name (`maps`, `map <name>`). Players need
the same map mod installed to join.

## World layers

`world-layers.json` lets the server force world layers (time of day and so on) on every player
(`world_layer_sync`, `tod`, `layer`). It is shipped in the package. The file from the Windows server
package or from a player's `%LOCALAPPDATA%\ReSkate\cache` works unchanged.

## Updates

The server **updates itself** from this project's
[releases](https://github.com/wildesPepega/ReSkate-Linux-Server/releases):

1. It checks on startup, every 30 minutes, **10 seconds after the last player left** and then
   every 5 minutes while the server stays empty.
2. A newer release is downloaded in the background, checked against GitHub's SHA-256 checksum and
   test-started (`--version`) so that a build that cannot run on this system is never installed.
   A release that comes out while players are on is downloaded right away and waits.
3. Once **nobody is connected**, it replaces its files and restarts in the same process. Hosting
   panels see the server keep running; the console stays attached. At startup, with nobody on yet,
   it installs right away.
4. `ReSkateServer.json`, `Mods/`, `plugins/` and the logs are never touched.

The console command `update` checks and installs straight away; connected players are told to
join again in a minute. Turn automatic updates off with `"auto_update": false` or `--no-update`
(`update` still works then).

When ReSkate itself releases a version that has no Linux build yet, the server says so and keeps
running; it updates once the Linux build is out.

The release check needs to reach `api.github.com` and `github.com` over HTTPS. The frequent checks
of an empty server read the redirect of `github.com/…/releases/latest`, which does not count
against GitHub's API limit of 60 requests an hour per address, so many servers behind one address
are fine; the API is only asked when there is a new release.

## Why players left

The log line for a player leaving carries the reason. For connections that Steam ended it includes
Steam's code:

| Log | Meaning |
|---|---|
| `Disconnected: Steam 1000, closed by the player's game, …` | normal: the player left or closed the game |
| `Disconnected: Steam 4001, timed out: the player stopped answering, …` | the player's connection dropped (their internet, Wi-Fi, a crash) |
| `Disconnected: Steam 5003, timed out, …` | the connection timed out somewhere between player and server |
| `Disconnected: Steam 3xxx, server: …` | a problem on the **server's** side (its network or Steam relay); look at the host |
| `Disconnected: Steam 5005/5006/5008/5009, …` | Steam could not keep a route between player and server |
| `Timed out waiting for gameplay data.` | connected, but the player's game sent nothing for 30 s (frozen or stuck loading) |
| `A player ended their session.` | the player's game said goodbye |
| `Disconnected: rejoined with a new connection.` | the player joined again while their old connection was still open; the new one replaced it |

Many players leaving with 3xxx or 5xxx codes at the same moment point to the server's network.
Single 1000s are just people leaving.

## Troubleshooting

| Message | Fix |
|---|---|
| `is steamclient.so in ~/.steam/sdk64 or next to the server?` | Install SteamCMD (step 1), or copy `steamclient.so` next to the binary |
| `libsteam_api.so: cannot open shared object file` | Keep `libsteam_api.so` in the same folder as `ReSkateServer` |
| `version 'GLIBC_2.xx' not found` | The system is too old; use a newer distro or build from source |
| Players see "version mismatch" | Players and server must run the same ReSkate version |
