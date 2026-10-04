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
  "distances": {
    "full_rate_return": 50,
    "half_rate_return": 150,
    "half_rate_start": 60,
    "low_rate_start": 170
  },
  "enforce_tuning": true,
  "layers": {},
  "listed": true,
  "map": "San Vansterdam",
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
  "port": 27015,
  "query_port": 27016,
  "score_allow": [],
  "score_check": "warn",
  "speed_check": "warn",
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
| `max_players` | `16` | 1–249. |
| `password` | `""` | Empty for anyone; otherwise players type it to join. |
| `welcome` | `""` | A chat line sent to each player as they join. |
| `listed` | `true` | `false` hides the server from the browser; players then need the code. |
| `auto_update` | `true` | Install new Linux builds on their own once the server is empty (see [installation.md](installation.md#updates)). |
| `port`, `query_port` | `27015`, `27016` | Steam game server ports. `--port` / `--query-port` override them. |
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
