# Commands

## Where commands work

- **Server console** (stdin, or the Pterodactyl console): type the command as is. `help` lists them.
- **Admins in game**: the console command `mp server <command>`; replies arrive in chat.
- **Admins in chat**: any command with a `/` in front, e.g. `/kick`, `/map`, `/votes`.
- Admins can also change the map by picking a level in Levels or Travel, and change voice,
  distances, placement and kicks from the ReSkate Multiplayer menu.

`<player>` is the start of a player's name or their SteamID64.

## Server and players

| Command | |
|---|---|
| `status` | Name, map, players, join code. |
| `players` | Connected players and their SteamID64s. |
| `say <text>` | Chat as the server (console only). |
| `kick <player>` | Kick until the server restarts. Admins cannot kick or ban each other; the console can. |
| `ban <player or id> [name]` | Ban for good. |
| `unban <id>` | Lift a ban. |
| `bans` | List bans. |
| `admin add\|remove <player or id>` | Manage admins (console only). |
| `admins` | List admins. |
| `version` | The server's version. |
| `update` | Check for a new release and install it straight away; players are told to rejoin (console only). |
| `plugins [reload]` | List the loaded plugins, or reload them from disk (see [plugins.md](plugins.md)). |
| `quit` (also `exit`, `stop`) | Shut down cleanly. |
| `help` | List commands. |

## Server settings

| Command | |
|---|---|
| `name <text>` | Server name. |
| `password <text\|off>` | Join password. |
| `welcome <text\|off>` | Welcome chat line. |
| `listed on\|off` | Show in the server browser. |
| `map <name>` | Change map, e.g. `map San Vansterdam`, `map grom`, `map bbcity`. |
| `maps` | Maps this server knows (built-in and from `Mods/`). |
| `tps 20\|30\|60\|120` | Network updates per second. |
| `voice on\|off` | Voice chat. |
| `voice-range <m>` | Proximity voice range, 50–1000 m. |
| `distances <full> <half> <half-return> <low>` | Update-rate distances, see [configuration](configuration.md#network-and-voice). |

## Gameplay

| Command | |
|---|---|
| `placement everyone\|admins\|nobody` | Who may place objects. |
| `clear-objects` | Remove all placed objects. |
| `noclip on\|off` | Allow noclip (and tp) for players. |
| `nobail on\|off` | Allow No Bail. |
| `boosts on\|off` | Allow forward/up boosts. |
| `tuning on\|off` | Force the game's own physics tuning. |
| `tpall [player]` | Teleport everyone to you (admins in game) or to a player. |
| `tphere <player>` | Teleport one player to you (admins in game). |
| `park <construction\|historic\|financial> <layout>` | Park lot layout. |
| `announce-throwdowns on\|off` | Chat message when a throwdown is placed. |

## World layers

| Command | |
|---|---|
| `layer-sync on\|off` | Force world layers on every player. |
| `layer <key> default\|on\|off` | Set one world layer. |
| `tod <default\|morning\|noon\|afternoon\|evening\|night\|weatherday\|weathernight>` | Time of day on every map (needs `layer-sync on`). |

## Votes

| Command | |
|---|---|
| `votes` | Show vote settings. |
| `votes map\|kick\|tod on\|off\|<percent>` | Enable/disable a vote type or set its pass percentage. |
| `votes seconds <n>` | Vote duration. |
| `votes cooldown <n>` | Cooldown per player. |
| `vote-cancel` | Cancel the running vote. |

## Parties

| Command | |
|---|---|
| `parties [on\|off]` | List parties, or allow them (`off` ends them all). |
| `party-size <2-8>` | Maximum party size. |

## Anti-cheat and logging

| Command | |
|---|---|
| `speed-check off\|warn\|kick` | Action for players whose game runs fast. |
| `score-check [off\|warn\|kick]` | Action for players whose mods change scoring/physics; without argument, every player's result. |
| `score-allow [<fingerprint>\|remove <fingerprint>]` | Accept a scoring mod's fingerprint (or list them). |
| `activity-log on\|off` | Log player activity. |

## Player chat commands

Available to every player in chat (plus any commands added by [plugins](plugins.md); `/help` lists them):

| Command | |
|---|---|
| `/vote map <map>` | Start a map vote (if enabled). |
| `/vote kick <player>` | Start a kick vote (if enabled). |
| `/vote tod <time>` | Start a time-of-day vote (if enabled). |
| `/yes`, `/no` | Vote. |
| `/party` | Show your party. |
| `/party invite\|join\|kick\|promote <player>` | Invite, join, remove or promote a player. |
| `/party accept\|decline [player]` | Answer an invite or join request. |
| `/party leave` | Leave your party. |
| `/party open\|close` | Let anyone join, or invites only. |
| `/p <message>` | Party chat. |
