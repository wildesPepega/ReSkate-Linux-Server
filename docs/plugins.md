# Plugins

Plugins are small [Lua 5.4](https://www.lua.org/manual/5.4/) scripts that add features to the
server without recompiling it:

- your own **chat commands** (`/discord`, `/rules`, …) that answer with text or do something,
- **automatic messages** the server posts on its own,
- **timers** and **events** (a player joins, leaves or chats).

Lua is built into the server binary; nothing has to be installed.

## Quick start

1. Create a `plugins/` folder next to `ReSkateServer`.
2. Put a file `plugins/hello.lua` in it:

   ```lua
   reskate.command("discord", { description = "Our Discord server" }, function(player)
       return "Join us: https://discord.gg/your-invite"
   end)

   reskate.automessage({
       interval = 600,
       messages = { "Welcome to {server}! Type /help for our commands." },
   })
   ```

3. Start the server, or type `plugins reload` in the console. The log shows:

   ```
   [plugins] 1 loaded: hello (/discord, 1 automessage(s))
   ```

4. In game, type `/discord` in chat. `/help` lists the plugin commands too.

Ready-to-use examples are in [`examples/plugins/`](../examples/plugins/) (also in the release
archive): `info.lua`, `automessages.lua`, `greeter.lua`, `chat-filter.lua`. Copy them into
`plugins/` and edit them.

## Files and loading

| | |
|---|---|
| `plugins/<name>.lua` | a plugin in one file |
| `plugins/<name>/main.lua` | a plugin in its own folder |
| `plugins/_<name>.lua` | a leading `_` (or `.`) turns a plugin off |

- Plugins load in alphabetical order when the server starts and on `plugins reload`.
- The plugin's name is its file or folder name. It shows in the log as `[name]`.
- A plugin that fails to load is skipped with an error in the log; the others still load.
- `plugins reload` reloads every plugin from disk. Timers and automatic messages start over.

### Console

| Command | |
|---|---|
| `plugins` | list loaded plugins with their commands, automatic messages and timers |
| `plugins reload` | reload all plugins |

Admins can use both in game too (`/plugins`, `/plugins reload`).

## How plugins run

- Each plugin has its **own Lua state**: globals of one plugin are invisible to the others.
- **Sandbox**: available are the Lua base functions, `string`, `table`, `math`, `utf8`,
  `coroutine` and `os.time`, `os.clock`, `os.date`, `os.difftime`. There is no `io`, no
  `require`, no `dofile`/`loadfile` and no other `os` functions: plugins cannot read or write
  files or start programs.
- **Limits**: one call into a plugin may run at most **250 ms** (loading: 2 s) and a plugin may use
  at most **32 MB** of memory. A plugin that goes over is stopped with an error in the log; the
  server keeps running.
- **Errors** in a plugin never crash the server. They are logged as `[name] error: …`. A player
  whose command failed sees *"That command failed. The server log has the details."*
- **Actions are queued**: messages, `reskate.run` and so on take effect right after the plugin
  function returns, in the order they were called.
- `print(...)` writes to the server log.

---

## API reference

Everything lives in the global table `reskate`.

### Player tables

Functions that take or return a player use a table:

```lua
{ id = "76561198000000000", name = "Alice", admin = true }
```

| Field | Type | |
|---|---|---|
| `id` | string | SteamID64 (a string, so it never loses digits) |
| `name` | string | in-game name |
| `admin` | boolean | listed in `admins` of `ReSkateServer.json` |

Where a function takes a *player*, you can pass a player table, a SteamID64 (string or number) or
the start of a connected player's name (it must match exactly one player).

### Placeholders

`reskate.format` and automatic messages replace these:

| Placeholder | |
|---|---|
| `{server}` | server name |
| `{map}` | current map |
| `{players}` | players online |
| `{max_players}` | player limit |

---

### `reskate.command(name, [options], handler)`

Adds a chat command `/name`.

```lua
reskate.command("spot", {
    description = "Show a spot's location",
    usage = "<name>",
    admin = false,
    aliases = { "s" },
}, function(player, args, line)
    if args[1] == nil then
        return "Usage: /spot <name>"
    end
    return player.name .. " asked for spot " .. args[1]
end)
```

| Option | Default | |
|---|---|---|
| `description` | `""` | shown by `/help <command>` |
| `usage` | `""` | arguments shown by `/help <command>`, e.g. `"<player> [reason]"` |
| `admin` | `false` | only admins may use it; it is hidden from other players' `/help` |
| `aliases` | `{}` | other names for the same command |

The handler gets:

| Argument | |
|---|---|
| `player` | who typed it (player table) |
| `args` | the words after the command, as an array (`/spot big ledge` → `{"big", "ledge"}`) |
| `line` | everything after the command as one string (`"big ledge"`) |

Whatever the handler **returns** is sent back to that player only. Return a string (use `"\n"` for
several lines, up to 12) or `nil` for no answer.

Names are lowercase `a-z`, `0-9`, `-` and `_`, up to 32 characters. The server's own commands
(`help`, `vote`, `party`, `p`, `kick`, `map`, `status`, …) cannot be replaced, and two plugins
cannot register the same name: `reskate.command` raises an error, so the plugin fails to load with
a clear message.

Order in chat: built-in player commands → plugin commands → admin server commands.

### `reskate.automessage(options)` → id

Posts chat messages to everyone at a fixed interval.

```lua
local id = reskate.automessage({
    interval = 300,
    delay = 60,
    order = "random",
    min_players = 3,
    messages = {
        "Type /discord to join our Discord.",
        "{players}/{max_players} skaters on {map}.",
    },
})
```

| Option | Default | |
|---|---|---|
| `messages` | required | array of messages; placeholders are filled in |
| `interval` | `300` | seconds between messages (at least 10) |
| `delay` | `interval` | seconds until the first message |
| `order` | `"sequence"` | `"sequence"` (one after the other, then from the start) or `"random"` |
| `min_players` | `1` | skip while fewer players are online (nothing is sent to an empty server) |

Returns an id for `reskate.cancel`.

### `reskate.every(seconds, handler)` → id

Calls `handler()` every `seconds` (at least 1). Returns an id for `reskate.cancel`.

### `reskate.after(seconds, handler)` → id

Calls `handler()` once after `seconds` (at least 1). Returns an id.

### `reskate.cancel(id)` → boolean

Stops a timer or automatic message of this plugin. Returns `true` if one was stopped.

### `reskate.on(event, handler)`

| Event | Handler | |
|---|---|---|
| `"join"` | `function(player)` | a player finished joining |
| `"leave"` | `function(player, reason)` | a player left; `reason` as in the log (`"Disconnected."`, kick reason, …) |
| `"chat"` | `function(player, text)` | a chat message (not `/` commands). **Return `false`** to keep it from the other players |

Several handlers (also from several plugins) may listen to the same event; each one runs. For
`"chat"`, the message is blocked if any handler returns `false`. Blocked messages are still written
to the log.

### `reskate.broadcast(text)`

Sends a chat message from the server to everyone. `"\n"` splits it into lines (up to 12).

### `reskate.tell(player, text)` → boolean

Sends a chat message to one player. Returns `false` if no connected player matches.

### `reskate.players()` → array

All connected players as player tables.

```lua
for _, p in ipairs(reskate.players()) do
    print(p.name, p.id, p.admin)
end
```

### `reskate.find(query)` → player or `nil`

A connected player by SteamID64 or the start of their name; `nil` if none or several match.

### `reskate.server()` → table

`{ name = "…", map = "…", players = 12, max_players = 16 }`

### `reskate.format(text)` → string

Fills in the placeholders.

### `reskate.run(command)`

Runs a **server console command** with console rights, as if typed into the server console, e.g.
`reskate.run("tod night")` or `reskate.run("kick Bob")`. The command and its answer are logged as
`[plugin] command: answer`. `quit` and `update` are console-only and not available.

> Be careful: console rights include `admin add`, `ban` and every setting. Only install plugins
> you trust.

### `reskate.log(text)`

Writes `[plugin] text` to the server log (same as `print`).

### `reskate.plugin`, `reskate.version`

`reskate.plugin.name` is the plugin's name; `reskate.version` the server version (e.g. `"1.0.5"`).

---

## Examples

### Command with a player argument

```lua
reskate.command("hug", { usage = "<player>", description = "Send someone a hug" }, function(player, args)
    local target = reskate.find(args[1] or "")
    if not target then
        return "Who? Type /hug <player>."
    end
    reskate.tell(target, player.name .. " sends you a hug!")
    return "Hug sent to " .. target.name .. "."
end)
```

### Night every evening (real time)

```lua
reskate.every(60, function()
    local hour = tonumber(os.date("%H"))
    if hour == 22 then
        reskate.run("tod night")
    elseif hour == 10 then
        reskate.run("tod noon")
    end
end)
```

`tod` needs `world_layer_sync` on (see [commands.md](commands.md#world-layers)).

### Admin command that runs server commands

```lua
reskate.command("event", { admin = true, description = "Start the night event" }, function(player)
    reskate.run("tod night")
    reskate.run("tpall " .. player.name)
    reskate.broadcast("Night event starts now!")
end)
```

### Count something per player

Plugin globals live as long as the plugin is loaded (until `plugins reload` or a restart):

```lua
local joins = {}

reskate.on("join", function(player)
    joins[player.id] = (joins[player.id] or 0) + 1
    if joins[player.id] == 1 then
        reskate.tell(player, "First time here today? Type /rules.")
    end
end)
```
