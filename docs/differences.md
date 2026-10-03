# Differences from the Windows server

The Linux port aims for full parity with `ReSkateServer.exe`: same network protocol, commands,
config fields and console messages. The differences are:

| | Windows | Linux (this port) |
|---|---|---|
| Updates | Downloads and installs new releases itself, restarts when empty | Only **reports** new releases (startup, every 30 min, `update`). Replace the files manually. |
| `--export-world-layers` | Reads the game files and writes `world-layers.json` | Not available (needs the game). Use the shipped `world-layers.json`, the one from the Windows package, or a player's `%LOCALAPPDATA%\ReSkate\cache\world-layers.json`. |
| Steam libraries | `steam_api64.dll`, `steamclient64.dll`, `tier0_s64.dll`, `vstdlib_s64.dll` | `libsteam_api.so` (shipped) + `steamclient.so` (from SteamCMD) |
| Packet limit | A player over the per-second packet limit is dropped at once | Dropped only after 4 seconds over the limit in a row, so the burst of queued packets after a network hiccup no longer kicks everyone |
| Shutdown | Closing the window | `quit`, Ctrl+C, SIGTERM, SIGHUP – all sign out of Steam first |

Existing `ReSkateServer.json` files, `Mods/` folders and `world-layers.json` from a Windows server can
be copied over unchanged.
