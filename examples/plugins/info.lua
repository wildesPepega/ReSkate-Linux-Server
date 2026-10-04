-- info.lua: simple /commands that answer with text.
-- Copy this file into the server's plugins/ folder and edit the texts.

reskate.command("discord", { description = "Our Discord server" }, function(player)
    return "Join us on Discord: https://discord.gg/your-invite"
end)

reskate.command("rules", { description = "The server rules", aliases = { "r" } }, function(player)
    return table.concat({
        "1. Be nice to each other.",
        "2. No speed hacks or cheats.",
        "3. Don't spam objects in busy spots.",
    }, "\n")
end)

-- {server}, {map}, {players} and {max_players} are filled in by reskate.format.
reskate.command("info", { description = "About this server" }, function(player)
    return reskate.format("Hi " .. player.name .. "! You're on {server}, map {map}, {players}/{max_players} players.")
end)
