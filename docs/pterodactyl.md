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

   The game port's allocation also serves the [status page](configuration.md#status-page) at
   `http://<ip>:<game port>/status` over TCP; Pterodactyl opens allocations for TCP and UDP.
3. Leave **Download URL** at its default to install the latest release from this repository, or point
   it at any reachable copy of `ReSkateServer-linux-x64.tar.gz`. Empty it to upload the files yourself.
4. Memory: 256 MB is plenty for a typical server; disk: 200 MB (SteamCMD included).

## What the install script does

1. Installs SteamCMD and runs it once.
2. Copies `steamclient.so` to `.steam/sdk64/` and next to the server.
3. If **Download URL** is set: downloads the archive and unpacks it into the server folder.
   Otherwise: upload the contents of `ReSkateServer-linux-x64.tar.gz` via the file manager or SFTP.

**Updates install themselves**: the server downloads a new release in the background and
installs it within seconds of the last player leaving, restarting in place (see
[installation.md](installation.md#updates)). Reinstalling the server (Settings → Reinstall Server)
also updates it; the config, `Mods/`, `plugins/` and logs are kept.

## Variables

| Variable | Env | Default | |
|---|---|---|---|
| Query Port | `QUERY_PORT` | `27016` | Second UDP allocation, Steam server queries |
| Download URL | `DOWNLOAD_URL` | latest release of this repo | Archive to install, empty = manual upload |
| Map Mods | `MAP_MODS` | empty | Thunderstore links of custom maps, separated by commas |

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

Type `map-mods add <Thunderstore link>` in the panel console (admins: `/map-mods add …` in game).
The server fetches only the map's `reskate-levels.json` into `Mods/` in the background, without a
restart, and keeps it up to date at every start. `map-mods` lists them, `map-mods remove <map>`
takes one out. Or paste the links into the **Map Mods** variable (Startup tab; several separated by
commas) and restart; clear a link there to remove that map again. See
[installation.md](installation.md#maps-from-thunderstore).

Maps that are not on Thunderstore: create a `Mods/` folder in the file manager and upload the map's
mod folder into it (see [installation.md](installation.md#custom-maps)).

The Map Mods variable comes with the egg from release 1.1.3-1: import the egg again (Nests → ReSkate
→ Import, or update it from its `update_url`) to get it. Older eggs keep working.
