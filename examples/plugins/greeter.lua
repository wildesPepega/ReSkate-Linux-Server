-- greeter.lua: events, timers and players.

reskate.on("join", function(player)
    reskate.tell(player, "Welcome, " .. player.name .. "! Type /online to see who is here.")
    if #reskate.players() == 10 then
        reskate.broadcast("10 skaters online. Let's go!")
    end
end)

reskate.on("leave", function(player, reason)
    reskate.log(player.name .. " left: " .. reason)
end)

reskate.command("online", { description = "Who is online" }, function(player)
    local names = {}
    for _, p in ipairs(reskate.players()) do
        names[#names + 1] = p.name
    end
    return #names .. " online: " .. table.concat(names, ", ")
end)

-- Admin-only command with arguments: /announce <text>
reskate.command("announce", { description = "Announce something to everyone", usage = "<text>", admin = true },
    function(player, args, line)
        if line == "" then
            return "Usage: /announce <text>"
        end
        reskate.broadcast("[Announcement] " .. line)
    end)

-- Every 30 minutes, write the player count to the log.
reskate.every(1800, function()
    reskate.log(#reskate.players() .. " players online")
end)
