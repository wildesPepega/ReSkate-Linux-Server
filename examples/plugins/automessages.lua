-- automessages.lua: chat messages the server sends on its own.
-- Placeholders: {server}, {map}, {players}, {max_players}.

reskate.automessage({
    interval = 600,          -- seconds between messages
    order = "sequence",      -- or "random"
    min_players = 2,         -- stay quiet while fewer players are on
    messages = {
        "Welcome to {server}! Type /help for the server's commands.",
        "Join our Discord: type /discord",
        "{players} skaters online on {map}. Have fun!",
    },
})
