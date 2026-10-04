// Console lines to a Discord webhook (docs/configuration.md#discord). Each line gets a category
// from its shape; the chosen categories are sent, several lines per message, from a thread of
// their own so the server never waits for Discord.
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const CATEGORIES: &[&str] = &[
    "start", "stop", "update", "join", "leave", "chat", "party", "command", "admin", "anticheat", "throwdown", "objects",
    "vote", "map", "plugins", "server",
];
const WEBHOOK_PREFIXES: &[&str] = &[
    "https://discord.com/api/webhooks/",
    "https://discordapp.com/api/webhooks/",
    "https://ptb.discord.com/api/webhooks/",
    "https://canary.discord.com/api/webhooks/",
];
// Lines collected for at most this long, or until a message is nearly full (2000 characters).
const BATCH: Duration = Duration::from_millis(1500);
const MESSAGE_LIMIT: usize = 1900;

enum Message {
    Line(String),
    Name(String),
    Flush(Sender<()>),
}

struct Discord {
    sender: Sender<Message>,
    events: Vec<&'static str>,
    name: String,
}

static DISCORD: Mutex<Option<Discord>> = Mutex::new(None);

// The category of one console line, from the prefixes and shapes the server writes.
pub fn category(text: &str) -> &'static str {
    let bracket = |prefix: &str| text.starts_with(prefix);
    if bracket("[chat] ") {
        return "chat";
    }
    if bracket("[party chat] ") || bracket("[party] ") {
        return "party";
    }
    if bracket("[command] ") {
        return "command";
    }
    if bracket("[admin] ") {
        return "admin";
    }
    if bracket("[anticheat] ") {
        return "anticheat";
    }
    if bracket("[throwdown] ") {
        return "throwdown";
    }
    if bracket("[objects] ") {
        return "objects";
    }
    if bracket("[vote] ") {
        return "vote";
    }
    if bracket("[map] ") || text.starts_with("Everyone has loaded ") {
        return "map";
    }
    if bracket("[plugins] ") {
        return "plugins";
    }
    if text.starts_with('[') && text.find("] ").is_some_and(|end| !text[1..end].contains(' ')) {
        return "plugins"; // "[name] ..." from a plugin
    }
    if text.contains(" joined (") && text.contains(" players") {
        return "join";
    }
    if text.contains(" left (") {
        return "leave";
    }
    if text.contains(" is up on ") && text.ends_with(" players.") {
        return "start";
    }
    if text == "Shutting down." || text == "Restarting for the update." {
        return "stop";
    }
    if text.starts_with("ReSkate Linux Server ") && text.contains(" is ")
        || text.starts_with("Installed ")
        || text.starts_with("Could not download ")
        || text.starts_with("Could not install ")
        || text.starts_with("The update failed")
        || text.starts_with("ReSkate ") && text.contains(" is out;")
    {
        return "update";
    }
    "server"
}

fn emoji(category: &str) -> &'static str {
    match category {
        "start" => "✅",
        "stop" => "⛔",
        "update" => "⬆️",
        "join" => "🟢",
        "leave" => "🔴",
        "chat" => "💬",
        "party" => "👥",
        "command" => "⌨️",
        "admin" => "🔧",
        "anticheat" => "🛡️",
        "throwdown" => "🏁",
        "objects" => "🧱",
        "vote" => "🗳️",
        "map" => "🗺️",
        "plugins" => "🧩",
        _ => "ℹ️",
    }
}

// Discord would format these (player names and chat are free text).
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '~' | '`' | '|' | '>' | '#' | '-' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn valid_webhook(url: &str) -> bool {
    WEBHOOK_PREFIXES.iter().any(|p| url.starts_with(p)) && url.len() < 512 && !url.contains(char::is_whitespace)
}

// Starts sending. `events` are category names, or "all".
pub fn start(webhook: &str, events: &[String], name: &str, log: fn(&str)) -> Result<String, String> {
    start_checked(webhook, events, name, log, true)
}

pub(crate) fn start_checked(webhook: &str, events: &[String], name: &str, log: fn(&str), check: bool) -> Result<String, String> {
    if check && !valid_webhook(webhook) {
        return Err("discord.webhook is not a Discord webhook URL (https://discord.com/api/webhooks/...)".into());
    }
    let mut chosen: Vec<&'static str> = Vec::new();
    for event in events {
        if event == "all" {
            chosen = CATEGORIES.to_vec();
            break;
        }
        match CATEGORIES.iter().find(|c| **c == event) {
            Some(c) => chosen.push(c),
            None => return Err(format!("discord.events: unknown event \"{event}\" (one of {}, or all)", CATEGORIES.join(", "))),
        }
    }
    let (sender, receiver) = channel();
    let url = webhook.to_string();
    let username = name.to_string();
    std::thread::spawn(move || run(url, username, receiver, log));
    let summary = format!("Discord: sending {} to the webhook.", if chosen.is_empty() { "nothing".into() } else { chosen.join(", ") });
    *DISCORD.lock().unwrap_or_else(|e| e.into_inner()) = Some(Discord { sender, events: chosen, name: name.to_string() });
    Ok(summary)
}

// One console line (as written, without the time stamp).
pub fn post(text: &str, time: &str) {
    let discord = DISCORD.lock().unwrap_or_else(|e| e.into_inner());
    let Some(discord) = discord.as_ref() else { return };
    let category = category(text);
    if discord.events.contains(&category) {
        let line = format!("{} `{time}` {}", emoji(category), escape(text));
        let _ = discord.sender.send(Message::Line(line));
    }
}

// The webhook posts under the server's name; it follows `name` changes.
pub fn set_name(name: &str) {
    let mut discord = DISCORD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(discord) = discord.as_mut().filter(|d| d.name != name) {
        discord.name = name.to_string();
        let _ = discord.sender.send(Message::Name(name.to_string()));
    }
}

// Sends what is queued, waiting up to `timeout` (before the server stops or restarts).
pub fn flush(timeout: Duration) {
    let sender = DISCORD.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|d| d.sender.clone());
    if let Some(sender) = sender {
        let (done, wait) = channel();
        if sender.send(Message::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(timeout);
        }
    }
}

pub fn status() -> String {
    match DISCORD.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(d) => format!("Discord: on, sending {}.", if d.events.is_empty() { "nothing".into() } else { d.events.join(", ") }),
        None => "Discord: off (set discord.webhook in the config and restart).".into(),
    }
}

// A line sent whatever the event filter says, to try the webhook.
pub fn test(text: &str) -> bool {
    let discord = DISCORD.lock().unwrap_or_else(|e| e.into_inner());
    let Some(discord) = discord.as_ref() else { return false };
    discord.sender.send(Message::Line(format!("🧪 {}", escape(text)))).is_ok()
}

fn send(agent: &ureq::Agent, url: &str, username: &str, content: &str) -> Result<(), (String, Option<Duration>)> {
    let body: Value = json!({
        "username": username.chars().take(80).collect::<String>(),
        "content": content,
        "allowed_mentions": { "parse": [] }, // no @everyone or pings from player text
        "flags": 4,                            // no link previews for URLs in chat
    });
    match agent.post(url).set("Content-Type", "application/json").send_string(&body.to_string()) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(429, response)) => {
            let wait = response.into_string().ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["retry_after"].as_f64());
            Err(("rate limited".into(), Some(Duration::from_secs_f64(wait.unwrap_or(2.0).clamp(0.5, 60.0)))))
        }
        Err(ureq::Error::Status(status, _)) => Err((format!("HTTP {status}"), None)),
        Err(_) => Err(("Discord could not be reached".into(), None)),
    }
}

fn run(url: String, mut username: String, receiver: Receiver<Message>, log: fn(&str)) {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).user_agent("ReSkateServer (Linux)").build();
    let mut pending: Vec<String> = Vec::new();
    let mut flushes: Vec<Sender<()>> = Vec::new();
    let mut failures = 0u32;
    loop {
        // Wait for a line, then gather more for a moment.
        let first = if pending.is_empty() && flushes.is_empty() { receiver.recv().ok() } else { None };
        let mut take = |message: Message, pending: &mut Vec<String>, flushes: &mut Vec<Sender<()>>| match message {
            Message::Line(line) => pending.push(line),
            Message::Name(name) => username = name,
            Message::Flush(done) => flushes.push(done),
        };
        match first {
            Some(message) => take(message, &mut pending, &mut flushes),
            None if pending.is_empty() && flushes.is_empty() => return, // the server is gone
            None => {}
        }
        let deadline = Instant::now() + BATCH;
        while flushes.is_empty() && pending.iter().map(|l| l.len() + 1).sum::<usize>() < MESSAGE_LIMIT {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(message) => take(message, &mut pending, &mut flushes),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        while let Ok(message) = receiver.try_recv() {
            take(message, &mut pending, &mut flushes);
        }
        // As many lines as fit in one message; the rest go next round.
        while !pending.is_empty() {
            let mut content = String::new();
            let mut used = 0;
            for line in &pending {
                let line: String = line.chars().take(MESSAGE_LIMIT).collect();
                if !content.is_empty() && content.len() + line.len() + 1 > MESSAGE_LIMIT {
                    break;
                }
                content.push_str(&line);
                content.push('\n');
                used += 1;
            }
            match send(&agent, &url, &username, &content) {
                Ok(()) => {
                    pending.drain(..used);
                    failures = 0;
                }
                Err((_, Some(wait))) => std::thread::sleep(wait),
                Err((why, None)) => {
                    failures += 1;
                    if failures == 3 {
                        log(&format!("Discord: the webhook is not accepting messages ({why}); dropping them until it works again."));
                    }
                    pending.drain(..used);
                    if failures >= 3 {
                        std::thread::sleep(Duration::from_secs(5));
                    }
                }
            }
            if flushes.is_empty() {
                break; // keep collecting; the next round sends the rest
            }
        }
        for done in flushes.drain(..) {
            let _ = done.send(());
        }
    }
}
