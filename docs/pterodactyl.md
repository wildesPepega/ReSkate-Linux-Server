# Pterodactyl

The egg is [`pterodactyl/egg-reskate.json`](../pterodactyl/egg-reskate.json) (also attached to every
release). It uses the `ghcr.io/parkervcp/yolks:debian` image.

## Import the egg

1. Admin area → **Nests** → **Import Egg**.
2. Choose `egg-reskate.json` and a nest (create one called e.g. "Games" if needed).

## Create a server

1. Admin area → **Servers** → **Create New**, pick the **ReSkate** egg.
2. Assign **two UDP allocations**:
   - the **primary** allocation is the game port (`SERVER_PORT`),
   - put the second port into the **Query Port** variable. It must differ from the game port.
3. Leave **Download URL** at its default to install the latest release from this repository, or point
   it at any reachable copy of `ReSkateServer-linux-x64.tar.gz`. Empty it to upload the files yourself.
4. Memory: 256 MB is plenty for a typical server; disk: 200 MB (SteamCMD included).

## What the install script does

1. Installs SteamCMD and runs it once.
2. Copies `steamclient.so` to `.steam/sdk64/` and next to the server.
3. If **Download URL** is set: downloads the archive and unpacks it into the server folder.
   Otherwise: upload the contents of `ReSkateServer-linux-x64.tar.gz` via the file manager or SFTP.

**Updates install themselves**: the server downloads a new release in the background and
installs it once nobody is connected, restarting in place (see
[installation.md](installation.md#updates)). Reinstalling the server (Settings → Reinstall Server)
also updates it; the config, `Mods/`, `plugins/` and logs are kept.

## Variables

| Variable | Env | Default | |
|---|---|---|---|
| Query Port | `QUERY_PORT` | `27016` | Second UDP allocation, Steam server queries |
| Download URL | `DOWNLOAD_URL` | latest release of this repo | Archive to install, empty = manual upload |

## Startup

```
./ReSkateServer --port {{SERVER_PORT}} --query-port {{QUERY_PORT}}
```

- The panel detects a successful start on the line `is up on`.
- **Stop** sends `quit`, so the server signs out of Steam cleanly.
- The panel console accepts every server command (`help`, `status`, `kick …`, see
  [commands.md](commands.md)).
- Edit `ReSkateServer.json` in the file manager while the server is stopped (or change settings
  with console commands while it runs – they are saved to the file). See
  [configuration.md](configuration.md).

## Plugins

Create a `plugins/` folder in the file manager and upload `.lua` files into it (examples are in
`examples/plugins/` after installation). Type `plugins reload` in the panel console to load
changes without a restart. See [plugins.md](plugins.md). A reinstall does not touch `plugins/`.

## Custom maps

Create a `Mods/` folder in the file manager and upload the map's mod folder into it (see
[installation.md](installation.md#custom-maps)).
