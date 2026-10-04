# Installation (without a panel)

## Requirements

- Linux x86_64 with glibc 2.31 or newer
- `steamclient.so` from SteamCMD – the server signs in to Steam anonymously through it
- Players and the server must run the **same ReSkate version** (this build: 1.0.5)

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
| `--port <port>` | game port, overrides `port` from the config |
| `--query-port <port>` | query port, overrides `query_port` |
| `--config <file>` | use another config file (default `ReSkateServer.json` next to the binary) |
| `--no-update` | do not check for new ReSkate releases |
| `--help` | print usage |

Commands are read from stdin, so the server works in `tmux`, `screen`, Docker and panels.

## Stopping

Type `quit`, press Ctrl+C or send SIGTERM / SIGHUP. The server signs out of Steam before exiting.

## Ports

| Port | Default | Protocol | Purpose |
|---|---|---|---|
| `port` | 27015 | UDP | Steam game server port |
| `query_port` | 27016 | UDP | Steam server queries (ping in the browser) |

Opening them is optional: players connect through Steam's relay network. With open ports the browser
shows the ping and joins are a little faster.

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
| `ReSkateServer.log` | log file, appended to |
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

The server checks for a new ReSkate release on startup, every 30 minutes and on the `update` command,
and reports it in the console. Unlike the Windows server it cannot install the update itself: replace
the files with a new Linux build (`ReSkateServer.json`, `Mods/` and logs are kept). Disable the check
with `"auto_update": false` or `--no-update`.

## Troubleshooting

| Message | Fix |
|---|---|
| `is steamclient.so in ~/.steam/sdk64 or next to the server?` | Install SteamCMD (step 1), or copy `steamclient.so` next to the binary |
| `libsteam_api.so: cannot open shared object file` | Keep `libsteam_api.so` in the same folder as `ReSkateServer` |
| `version 'GLIBC_2.xx' not found` | The system is too old; use a newer distro or build from source |
| Players see "version mismatch" | Players and server must run the same ReSkate version |
