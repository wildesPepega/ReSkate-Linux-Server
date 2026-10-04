# Differences from the Windows server

The Linux port aims for full parity with `ReSkateServer.exe`: same network protocol, commands,
config fields and console messages. The differences are:

| | Windows | Linux (this port) |
|---|---|---|
| Updates | Downloads and installs new releases itself, restarts when empty | The same, from this project's Linux releases, with a checksum and a test start before installing; restarts in the same process |
| Rejoining | A player whose old connection is still open is turned away until it times out | The new connection replaces the old one |
| Logs | One growing `ReSkateServer.log` | A new `ReSkateServer.log` each day; earlier days in `logs/`, 14 days kept |
| `--export-world-layers` | Reads the game files and writes `world-layers.json` | Not available (needs the game). Use the shipped `world-layers.json`, the one from the Windows package, or a player's `%LOCALAPPDATA%\ReSkate\cache\world-layers.json`. |
| Steam libraries | `steam_api64.dll`, `steamclient64.dll`, `tier0_s64.dll`, `vstdlib_s64.dll` | `libsteam_api.so` (shipped) + `steamclient.so` (from SteamCMD) |
| Packet limit | A player over the per-second packet limit is dropped at once | Dropped only after 4 seconds over the limit in a row, so the burst of queued packets after a network hiccup no longer kicks everyone |
| Gameplay timeout | A player whose game sends nothing for 10 s is dropped | 30 s: games freeze for longer while loading a newly placed throwdown drop (Spot Battles especially), which dropped several players at once |
| Discord | – | Console events to a Discord webhook ([configuration.md](configuration.md#discord)) |
| Ports | `port` and `query_port` in `ReSkateServer.json` | Only `--port` / `--query-port` (the panel's allocations); the old keys are removed from the file |
| Plugins | – | Lua plugins for custom commands, automatic messages, timers and events ([plugins.md](plugins.md)) |
| Shutdown | Closing the window | `quit`, Ctrl+C, SIGTERM, SIGHUP – all sign out of Steam first |

Existing `ReSkateServer.json` files, `Mods/` folders and `world-layers.json` from a Windows server can
be copied over unchanged.
