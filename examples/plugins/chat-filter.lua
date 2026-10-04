-- chat-filter.lua: returning false from a "chat" handler keeps the message from the others.

local blocked = { "badword1", "badword2" }

reskate.on("chat", function(player, text)
    local lower = text:lower()
    for _, word in ipairs(blocked) do
        if lower:find(word, 1, true) then
            reskate.tell(player, "Please keep the chat friendly.")
            return false
        end
    end
end)
