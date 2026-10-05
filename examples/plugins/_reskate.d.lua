---@meta
-- API definitions for ReSkate server plugins, for editors with the Lua language server
-- (VS Code: the "Lua" extension). Keep this file next to your plugins for autocompletion and
-- inline docs. The leading "_" makes the server skip it. Reference: docs/plugins.md

---A connected player.
---@class ReskatePlayer
---@field id string SteamID64 as a string
---@field name string In-game name
---@field admin boolean Listed in "admins" of ReSkateServer.json
---@field online_seconds integer Seconds since the player connected (a map change keeps counting)

---A player table, a SteamID64 (string or number) or the start of one connected player's name.
---@alias ReskatePlayerRef ReskatePlayer|string|integer

---@class ReskateServerInfo
---@field name string Server name
---@field map string Current map
---@field players integer Players online
---@field max_players integer Player limit
---@field password boolean A password is set
---@field listed boolean Shown in the server browser
---@field uptime_seconds integer Seconds since the server started

---@class ReskateCommandOptions
---@field description? string Shown by /help <command>
---@field usage? string Arguments shown by /help <command>, e.g. "<player> [reason]"
---@field admin? boolean Only admins may use it (default false)
---@field aliases? string[] Other names for the same command

---@class ReskateAutoMessageOptions
---@field messages string[] Messages; {server}, {map}, {players} and {max_players} are filled in
---@field interval? number Seconds between messages, at least 10 (default 300)
---@field delay? number Seconds until the first message (default: interval)
---@field order? "sequence"|"random" Default "sequence"
---@field min_players? integer Skip while fewer players are online (default 1)

---@class ReskatePluginInfo
---@field name string The plugin's file or folder name

---@class Reskate
---@field plugin ReskatePluginInfo
---@field version string Server version, e.g. "1.0.8-5"
reskate = {}

---Adds the chat command /name. The handler's return value is sent to the player who typed it.
---@param name string
---@param options ReskateCommandOptions
---@param handler fun(player: ReskatePlayer, args: string[], line: string): string?
---@overload fun(name: string, handler: fun(player: ReskatePlayer, args: string[], line: string): string?)
function reskate.command(name, options, handler) end

---Posts chat messages to everyone at a fixed interval.
---@param options ReskateAutoMessageOptions
---@return integer id For reskate.cancel
function reskate.automessage(options) end

---Calls handler every `seconds` (at least 1).
---@param seconds number
---@param handler fun()
---@return integer id For reskate.cancel
function reskate.every(seconds, handler) end

---Calls handler once after `seconds` (at least 1).
---@param seconds number
---@param handler fun()
---@return integer id For reskate.cancel
function reskate.after(seconds, handler) end

---Stops a timer or automatic message of this plugin.
---@param id integer
---@return boolean stopped
function reskate.cancel(id) end

---A player finished joining.
---@param event "join"
---@param handler fun(player: ReskatePlayer)
---@overload fun(event: "leave", handler: fun(player: ReskatePlayer, reason: string))
---@overload fun(event: "chat", handler: fun(player: ReskatePlayer, text: string): boolean?)
---@overload fun(event: "map", handler: fun(map: string))
---@overload fun(event: "stop", handler: fun(reason: string))
function reskate.on(event, handler) end

---Sends a chat message from the server to everyone. "\n" splits it into lines (up to 12).
---@param text string
function reskate.broadcast(text) end

---Sends a chat message to one player.
---@param player ReskatePlayerRef
---@param text string
---@return boolean sent False if no connected player matches
function reskate.tell(player, text) end

---All connected players.
---@return ReskatePlayer[]
function reskate.players() end

---A connected player by SteamID64 or the start of their name; nil if none or several match.
---@param query ReskatePlayerRef
---@return ReskatePlayer?
function reskate.find(query) end

---@return ReskateServerInfo
function reskate.server() end

---Fills in {server}, {map}, {players} and {max_players}.
---@param text string
---@return string
function reskate.format(text) end

---Runs a server console command with console rights, e.g. "tod night". Logged with its answer.
---@param command string
function reskate.run(command) end

---Writes "[plugin] text" to the server log.
---@param text string
function reskate.log(text) end

---This plugin's saved data (plugins/data/<plugin>.json), or an empty table if there is none.
---@return table
function reskate.load() end

---Saves a table as this plugin's data (up to 1 MiB, no functions; keys are stored as text).
---@param data table
---@return boolean saved
function reskate.save(data) end
