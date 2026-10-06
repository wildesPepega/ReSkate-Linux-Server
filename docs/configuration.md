# Configuration – `ReSkateServer.json`

Written on the first start next to the binary (or wherever `--config` points). Every change made from
the console or by an admin in game is saved back to this file.

## Default file

```json
{
  "activity_log": true,
  "admins": [],
  "announce_throwdowns": true,
  "auto_update": true,
  "bans": [],
  "boosts": true,
  "discord": {
    "events": ["start", "stop", "update", "join", "leave", "throwdown", "vote", "anticheat", "admin"],
    "style": "embed",
    "webhook": ""
  },
  "distances": {
    "full_rate_return": 50,
    "half_rate_return": 150,
    "half_rate_start": 60,
    "low_rate_start": 170
  },
  "enforce_tuning": true,
  "global_bans": true,
  "layers": {},
  "listed": true,
  "map": "San Vansterdam",
  "map_pool": [],
  "map_rotation_minutes": 0,
  "max_players": 16,
  "name": "ReSkate server",
  "no_bail": true,
  "noclip": true,
  "object_placement": "everyone",
  "parks": {
    "construction": "skatepark_01",
    "financial": "flumppark_08",
    "historic": "megapark_05"
  },
  "parties": true,
  "party_size": 8,
  "password": "",
  "score_allow": [],
  "score_check": "warn",
  "speed_check": "warn",
  "status": { "enabled": true, "players": true },
  "tps": 30,
  "voice_chat": true,
  "voice_range": 300.0,
  "votes": {
    "cooldown_seconds": 60,
    "kick": { "enabled": false, "percent": 60 },
    "map": { "enabled": false, "percent": 60 },
    "seconds": 30,
    "time_of_day": { "enabled": false, "percent": 50 }
  },
  "welcome": "",
  "world_layer_sync": false
}
```

## Fields

### General

| Field | Default | |
|---|---|---|
| `name` | `"ReSkate server"` | Shown in the server browser (1–64 characters). |
| `map` | `"San Vansterdam"` | The map everyone skates, named like the game's load command: `"San Vansterdam"`, `"Isle of Grom"`, `"Super Ultra Mega Resort"`, `"Stadium 1"`, or a custom map such as `"bbcity"`. |
| `map_pool` | `[]` | The maps players may vote for and the rotation goes through, in order, e.g. `["San Vansterdam", "Isle of Grom", "bbcity"]`. Empty allows every map the server knows. Admins can still change to any map. Also `map-pool` in the console. |
| `map_rotation_minutes` | `0` | Minutes on each map before the server moves to the next one in `map_pool` (0: off, at most 1440). Players get a minute's warning; the clock waits while nobody is on and while a map vote runs, and starts over whenever the map changes. Also `rotation` in the console. |
| `max_players` | `16` | 1–249. |
| `password` | `""` | Empty for anyone; otherwise players type it to join. |
| `welcome` | `""` | A chat line sent to each player as they join. |
| `listed` | `true` | `false` hides the server from the browser; players then need the code. |
| `auto_update` | `true` | Install new Linux builds on their own once the server is empty (see [installation.md](installation.md#updates)). |
| `global_bans` | `true` | Turn away players the ReSkate team has banned from multiplayer ("You are banned from ReSkate multiplayer."), also when they are already on. The list is read from `api.reskate.dev` at startup and every ten minutes (a minute after a failure); while it cannot be read, the bans already read hold. `false` lets them in; the server's own `bans` apply either way. |
| `admins` | `[]` | SteamID64s **as strings** who may change settings in game, e.g. `["76561198000000000"]`. |
| `bans` | `[]` | Players who can never join. Managed with `ban` / `unban`. |

### Gameplay

| Field | Default | |
|---|---|---|
| `object_placement` | `"everyone"` | `everyone`, `admins` (only admins can build) or `nobody`. |
| `noclip` | `true` | Let players use noclip (and tp). Admins always can. |
| `no_bail` | `true` | Let players use No Bail. Admins always can. |
| `boosts` | `true` | Let players use the forward and up boosts. Admins always can. |
| `enforce_tuning` | `true` | Players skate with the game's own `Gameplay/SkatePhysicsTuning`, not edited copies (edited tuning would otherwise show on their skater for everyone). |
| `parks` | see above | Layout for each park lot (`construction`, `historic`, `financial`), e.g. `"skatepark_01"`, or `"empty"`. |
| `announce_throwdowns` | `true` | Tell everyone in chat when a throwdown drop is placed. |

### Network and voice

| Field | Default | |
|---|---|---|
| `tps` | `30` | Network updates per second: `20`, `30`, `60` or `120`. |
| `voice_chat` | `true` | Allow voice chat. |
| `voice_range` | `300.0` | How far proximity voice reaches, 50–1000 m. |
| `distances` | see above | When far-away players update less often (metres): `half_rate_start` / `full_rate_return` and `low_rate_start` / `half_rate_return` are the thresholds for dropping to half / low rate and for going back up. |

### Parties

| Field | Default | |
|---|---|---|
| `parties` | `true` | Let players form parties: invite from the game's Social menu, a player card, the ReSkate Multiplayer menu or chat (`/party invite <player>`). Party members join each other's co-op challenges, see each other on the map and talk with `/p <message>`. |
| `party_size` | `8` | Most players in one party, 2–8. |

### Votes

Each vote type is off until enabled.

```json
"votes": {
  "map":         { "enabled": true, "percent": 60 },
  "kick":        { "enabled": true, "percent": 60 },
  "time_of_day": { "enabled": true, "percent": 50 },
  "seconds": 30,
  "cooldown_seconds": 60
}
```

| Key | |
|---|---|
| `map` | `/vote map <map>` |
| `kick` | `/vote kick <player>` – admins cannot be vote-kicked |
| `time_of_day` | `/vote tod <time>` – needs `world_layer_sync` |
| `percent` | share of connected players whose *yes* passes the vote |
| `seconds` | how long a vote runs (default 30) |
| `cooldown_seconds` | how long a player waits before starting another vote (default 60) |

Players vote with `/yes` and `/no` in chat.

### Anti-cheat

| Field | Default | |
|---|---|---|
| `speed_check` | `"warn"` | Catch players whose game runs faster than normal (Cheat Engine speedhack and the like), measured from the timing of what their game sends. `warn` takes them out of throwdowns and co-op challenges until their speed is normal again and tells the admins; `kick` removes them; `off` does not check. |
| `score_check` | `"warn"` | Catch players whose mods change trick scoring (per-trick points, multipliers, throwdown scoring) or skater handling (core physics, wipeouts, trick gestures). Each player's ReSkate checks its mods at launch and reports the result on join. `warn` takes them out of throwdowns and co-op challenges and tells everyone in chat; `kick` removes them; `off` does not check. A player must restart skate. without the mod to take part again. |
| `score_allow` | `[]` | Scoring fingerprints accepted like the game's own, for servers running a scoring mod everyone installs (16 hex digits each; `score-check` lists each player's). |
| `activity_log` | `true` | Log what players do: throwdown drops placed, joins, starts, turns and results; objects placed or removed; load times. |

### World layers

| Field | Default | |
|---|---|---|
| `world_layer_sync` | `false` | Force the `layers` below on every player. Needs `world-layers.json` next to the server. |
| `layers` | `{}` | World layer key → `"on"` / `"off"`. Usually set with the `tod` and `layer` commands. |

### Status page

The server answers `GET http://<ip>:<port>/status` with its live status as JSON, for websites, Discord
bots or monitoring. `<port>` is the game port (`--port`): the game uses it over UDP, the status page
over **TCP**, so no extra port is needed (a Pterodactyl allocation opens both).

```json
"status": { "enabled": true, "players": true }
```

| Key | |
|---|---|
| `enabled` | Serve the status page. Read at startup: restart after changing it. |
| `players` | Include `player_list` (names, admin, time online). `false`: only the counts. |

```json
{
  "name": "ReSkate server", "version": "1.1.0-2", "protocol": 39,
  "map": "San Vansterdam", "players": 2, "max_players": 16,
  "password": false, "listed": true,
  "join_code": "90294023726563330-ff2f9e71a5ad5603",
  "uptime_seconds": 86400,
  "player_list": [
    { "name": "Bob", "admin": false, "online_seconds": 1260 },
    { "name": "Alice", "admin": true, "online_seconds": 300 }
  ],
  "updated": 1791172800
}
```

- `players` counts players who finished joining; `player_list` (or `null` with `"players": false`)
  lists them.
- `join_code` is only given for a listed server, whose code the server browser shows anyway;
  otherwise `null`. The password is never part of it.
- `updated` is a Unix time; the snapshot is refreshed every second. A server that is down does not
  answer at all.
- Also at `/status.json`. Answers carry `Access-Control-Allow-Origin: *`, so a web page on any domain
  can fetch it.

### Discord

Sends console events to a Discord channel through a
[webhook](https://support.discord.com/hc/en-us/articles/228383668) (channel settings →
Integrations → Webhooks → New Webhook → Copy Webhook URL).

```json
"discord": {
  "webhook": "https://discord.com/api/webhooks/123456789/abcdef...",
  "events": ["start", "stop", "update", "join", "leave", "throwdown", "vote", "anticheat", "admin"],
  "style": "embed"
}
```

| Key | |
|---|---|
| `webhook` | The webhook URL; empty turns Discord off. Read at startup: restart after changing it. Keep it secret, anyone with it can post to the channel. |
| `events` | What to send, from the list below, or `["all"]`. |
| `style` | `"embed"` (default): one embed per event, with a title, a colour per event, the server's name and the time, shown in each reader's own time zone. `"text"`: one compact line per event, handy for a busy chat feed. |

| Event | Console lines |
|---|---|
| `start` | the server is up (`… is up on <map> for N players.`) |
| `stop` | shutting down, restarting for an update |
| `update` | a new release found, downloaded, installed; a new ReSkate version without a Linux build yet |
| `join` | a player joined |
| `leave` | a player left, with the reason |
| `throwdown` | throwdown drops placed and removed, players joining, leads, results |
| `vote` | votes started, passed, failed, cancelled |
| `anticheat` | speed check and score check findings |
| `admin` | commands admins ran in game |
| `command` | `/` commands players typed |
| `chat` | chat messages |
| `party` | party invites, joins and party chat |
| `objects` | objects placed and removed |
| `map` | players finished loading a map |
| `plugins` | plugin messages and errors |
| `server` | everything else (console answers, startup details, errors) |

Events are collected for about a second and sent together (up to 10 embeds per message), so busy
servers stay under Discord's rate limit. The webhook posts under the server's name. Mentions in
player text (`@everyone`, user pings) and link previews are suppressed.

| Event | Embed colour |
|---|---|
| `start`, `join` | green |
| `stop`, `leave` | red |
| `update` | blurple |
| `throwdown` | orange |
| `anticheat` | yellow |
| `admin` | dark orange |
| `vote` | purple |
| `chat`, `party` | light blue |
| `map` | teal |
| `plugins` | pink |
| others | grey |

Console: `discord` shows what is sent, `discord test` posts a test message.

Changed from older versions: `port` and `query_port` are no longer settings. The server takes the
ports from `--port` and `--query-port` (a hosting panel's allocations), default 27015 and 27016,
and removes the old keys from the file.
