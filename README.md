# ReSkate Linux Server

> [!CAUTION]
> **AI-generated port.** This server was ported from the original C++ Windows server to Rust
> **entirely by Claude Opus 5.5** (Anthropic's AI model, via Claude Code). Unit tests pass and it runs
> in production with 18 players on a custom map, but the code has not been audited line by line. Use it
> at your own risk and please [report issues](https://github.com/wildesPepega/ReSkate-Linux-Server/issues).

A native Linux port of the [ReSkate](https://github.com/Dingo-Shenanigans/ReSkate) dedicated server,
rewritten in Rust. It speaks the same network protocol as the Windows `ReSkateServer.exe`, so players
join it straight from the game – no Wine, no Windows VM.

> ReSkate is a fan project for **skate.** and is not affiliated with or endorsed by Electronic Arts or
> Full Circle. This repository is an unofficial port of the ReSkate dedicated server and is not
> maintained by the ReSkate team.

| | |
|---|---|
| Based on | ReSkate **1.1.0**, network protocol **39** |
| Platform | Linux x86_64, glibc 2.31 or newer (Debian 11+, Ubuntu 20.04+, …) |
| Language | Rust (edition 2021) |
| License | GPL-3.0 (same as ReSkate) |

## Why a native Linux server?

- **No Windows, no Wine.** Runs on any cheap Linux VPS, root server or game panel. Not having to
  run a Windows VM or Wine saves a lot of RAM and CPU, and removes a whole class of compatibility
  problems.
- **Small footprint.** One 3.4 MB binary plus `libsteam_api.so`. See the numbers below.
- **Fast start.** Up and listed in the server browser in about one second.
- **Rust.** Memory-safe by design: no buffer overflows or use-after-free bugs in the network code
  that parses untrusted player packets. Built with LTO and full optimisation.
- **Clean process handling.** `quit`, Ctrl+C, SIGTERM and SIGHUP all sign out of Steam before
  exiting, so Docker, systemd and Pterodactyl stop it cleanly.
- **Panel ready.** Reads commands from stdin and comes with a Pterodactyl egg that installs and
  updates itself from the latest release.
- **Drop-in compatible.** Same protocol, commands and `ReSkateServer.json` as the Windows server:
  copy your config and `Mods/` over and keep going.

### Real-world numbers

A live Pterodactyl server with **18 players on a modded (custom) map**:

| | |
|---|---|
| CPU | **4–5.5 %** |
| Memory | **~17 MiB** (container memory as shown by Pterodactyl) |
| Network in | ~690 KiB/s (~1,200 packets/s) |
| Network out | ~21 KiB/s (~270 packets/s) |

An idle server on a desktop machine used ~1 % CPU; its RSS of ~48 MB counts Steam's own shared
libraries, which the panel's container figure does not.

## Features

Everything the Windows dedicated server does, plus a few things it doesn't:

- Listed in the in-game server browser (**Multiplayer → Servers**) or joinable by code
- All console and admin commands, admin commands in chat (`/kick`, `/map`, …)
- Player votes (map, kick, time of day), parties and party chat (`/party`, `/p`)
- Speed check and score check (anti-cheat), activity log
- Object sync / park editor, throwdowns, voice relay with proximity range
- World layer sync (time of day), park layouts, password, bans
- Custom maps from a `Mods/` folder
- Clean shutdown on `quit`, Ctrl+C, SIGTERM and SIGHUP (signs out of Steam first)
- **Lua plugins**: custom `/commands`, automatic messages, timers, events and saved data ([docs](docs/plugins.md))
- **Self-updating**: installs new releases on its own seconds after the server is empty, after checking
  the download and test-starting it ([docs](docs/installation.md#updates))
- **Status page**: live JSON at `http://<ip>:<port>/status` on the game port, for websites and bots ([docs](docs/configuration.md#status-page))
- **Discord webhooks** for console events: joins, leaves, start/stop, updates, throwdowns, votes, anticheat, … ([docs](docs/configuration.md#discord))
- Rejoining players replace their old connection instead of being turned away
- Daily log files, two weeks kept
- Ready-made **Pterodactyl egg**

## Quick start

```bash
# 1. steamclient.so comes from SteamCMD (needs lib32gcc-s1 on Debian/Ubuntu)
mkdir -p ~/steamcmd && cd ~/steamcmd
curl -sSL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz | tar -xz
./steamcmd.sh +quit
mkdir -p ~/.steam/sdk64
ln -sf ~/steamcmd/linux64/steamclient.so ~/.steam/sdk64/steamclient.so

# 2. Download and start the server
cd ~
curl -sSLO https://github.com/wildesPepega/ReSkate-Linux-Server/releases/latest/download/ReSkateServer-linux-x64.tar.gz
tar -xzf ReSkateServer-linux-x64.tar.gz
cd ReSkateServer-linux-x64
./ReSkateServer
```

The first start writes `ReSkateServer.json`. Set at least `name` and `admins`, then restart. Type
`help` in the console for all commands.

Players connect through Steam's relay network, so **no ports have to be opened**. If you forward
UDP 27015 and 27016, the server browser also shows the ping and joins are a bit faster.

## Downloads

- **[Latest release](https://github.com/wildesPepega/ReSkate-Linux-Server/releases/latest)** –
  `ReSkateServer-linux-x64.tar.gz` and `egg-reskate.json`
- The same build is also committed under [`builds/`](builds/)

The archive contains:

| File | Purpose |
|---|---|
| `ReSkateServer` | the server binary |
| `libsteam_api.so` | Steamworks API, must stay next to the binary |
| `world-layers.json` | world layer catalogue (time of day etc.), optional |
| `README.md`, `docs/` | this documentation |
| `examples/plugins/` | example Lua plugins (copy them into `plugins/`) |
| `LICENSE.txt`, `licenses/` | GPL-3.0 and third-party licenses |

## Documentation

| Topic | |
|---|---|
| [Installation](docs/installation.md) | Running on a plain Linux box, systemd service, ports, custom maps |
| [Pterodactyl](docs/pterodactyl.md) | Importing and using the egg |
| [Configuration](docs/configuration.md) | Every field of `ReSkateServer.json` |
| [Commands](docs/commands.md) | Console, admin and chat commands |
| [Plugins](docs/plugins.md) | Lua plugins: custom commands, automatic messages, API reference |
| [Building](docs/building.md) | Compiling from source, packaging, tests, code layout |
| [Differences](docs/differences.md) | What differs from the Windows server |

## Plugins

Extend the server with small Lua scripts in a `plugins/` folder, no recompiling needed:

```lua
-- plugins/info.lua
reskate.command("discord", { description = "Our Discord server" }, function(player)
    return "Join us: https://discord.gg/your-invite"
end)

reskate.automessage({
    interval = 600,
    messages = { "Welcome to {server}! Type /help for our commands." },
})
```

Type `plugins reload` in the console and `/discord` works in game. See the
[plugin docs](docs/plugins.md) for the full API and [`examples/plugins/`](examples/plugins/).

## Building from source

```bash
cargo build --release      # target/release/ReSkateServer
cargo test --release       # protocol, codec and rule tests
./package-linux.sh         # builds dist/ReSkateServer-linux-x64.tar.gz
```

Needs Rust 1.80+ and a C compiler (for zstd). See [docs/building.md](docs/building.md).

## Repository layout

```
├── src/                 Rust source code
├── dist-assets/         files shipped in the package (libsteam_api.so, world-layers.json, licenses)
├── examples/plugins/    example Lua plugins
├── pterodactyl/         Pterodactyl egg
├── builds/              prebuilt Linux x64 package
├── docs/                documentation
├── package-linux.sh     build + package script
└── Cargo.toml
```

## Credits and license

The original ReSkate project, its Windows dedicated server and the network protocol are by the
[ReSkate team](https://github.com/Dingo-Shenanigans/ReSkate). This port is a derivative work and is
licensed under the **GNU General Public License v3.0**, see [LICENSE](LICENSE).

`libsteam_api.so` is part of the Steamworks SDK redistributables by Valve Corporation. zstd is
licensed under the BSD license (`dist-assets/zstd-LICENSE.txt`); Rust crate licenses are listed in
`licenses/rust-crates.txt` inside the package.
