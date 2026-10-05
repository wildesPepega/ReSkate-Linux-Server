-- playtime.lua: saved data. Counts each player's time on the server across restarts,
-- with /playtime [player] and /top.

local data = reskate.load()          -- { players = { [steam id] = { name = ..., seconds = ... } } }
data.players = data.players or {}
local counted = {}                   -- steam id -> online_seconds already added this connection

local function record(p)
    -- Someone online before this plugin was (re)loaded: their time so far was counted then.
    if counted[p.id] == nil then
        counted[p.id] = p.online_seconds
    end
    local entry = data.players[p.id] or { name = p.name, seconds = 0 }
    entry.seconds = entry.seconds + math.max(0, p.online_seconds - counted[p.id])
    entry.name = p.name
    data.players[p.id] = entry
    counted[p.id] = p.online_seconds
end

local function save()
    for _, p in ipairs(reskate.players()) do
        record(p)
    end
    reskate.save(data)
end

local function duration(seconds)
    local hours, minutes = seconds // 3600, seconds % 3600 // 60
    if hours > 0 then
        return string.format("%dh %02dm", hours, minutes)
    end
    return string.format("%dm", minutes)
end

reskate.on("join", function(player)
    counted[player.id] = player.online_seconds -- loading the map does not count
end)
reskate.on("leave", function(player)
    record(player)
    counted[player.id] = nil
    reskate.save(data)
end)
reskate.on("stop", save)
reskate.every(60, save)

reskate.command("playtime", { description = "Time on this server", usage = "[player]" }, function(player, args)
    save()
    local wanted = args[1] and args[1]:lower()
    for id, entry in pairs(data.players) do
        if (wanted == nil and id == player.id) or (wanted and entry.name:lower():sub(1, #wanted) == wanted) then
            return entry.name .. ": " .. duration(entry.seconds)
        end
    end
    return wanted and "Nobody called \"" .. args[1] .. "\" has played here." or "No time counted yet."
end)

reskate.command("top", { description = "Most time on this server" }, function()
    save()
    local list = {}
    for _, entry in pairs(data.players) do
        list[#list + 1] = entry
    end
    table.sort(list, function(a, b) return a.seconds > b.seconds end)
    local lines = {}
    for i = 1, math.min(5, #list) do
        lines[#lines + 1] = i .. ". " .. list[i].name .. " " .. duration(list[i].seconds)
    end
    return #lines > 0 and table.concat(lines, "\n") or "No time counted yet."
end)
