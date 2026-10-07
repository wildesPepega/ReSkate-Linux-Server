// Lua plugins (docs/plugins.md): plugins/<name>.lua or plugins/<name>/main.lua, each in its own
// sandboxed Lua state. Plugins add chat commands, automatic messages, timers and event handlers.
// They never touch the host directly: what they do is queued as actions, which the host applies
// once the plugin's code has returned.
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, Variadic, VmState};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

// How long one call into a plugin may run, and how much memory one plugin may use.
const CALL_LIMIT: Duration = Duration::from_millis(250);
const LOAD_LIMIT: Duration = Duration::from_secs(2);
const MEMORY_LIMIT: usize = 32 * 1024 * 1024;
const MIN_INTERVAL_US: u64 = 1_000_000;
const MIN_AUTOMESSAGE_US: u64 = 10_000_000;
// reskate.save(): the largest file one plugin may keep, and how deep its tables may nest.
const DATA_LIMIT: usize = 1024 * 1024;
const DATA_DEPTH: usize = 32;

// Commands the server answers itself; plugins cannot take these names.
const RESERVED: &[&str] = &[
    "help", "?", "party", "p", "yes", "y", "no", "n", "vote", "status", "players", "say", "chat", "kick", "ban", "unban",
    "w", "whisper", "tell", "msg", "msg-party", "msg-admins", "map-pool", "rotation", "map-mods",
    "bans", "map", "maps", "name", "password", "welcome", "listed", "tps", "voice", "voice-range", "voice-allow",
    "distances", "placement", "object-placement", "clear-objects", "noclip", "noclip-allow", "nobail", "nobail-allow",
    "boosts", "boosts-allow", "tuning", "tuning-enforce", "tpall", "tphere", "park", "layer", "layers", "layer-sync",
    "world-layer-sync", "tod", "time", "votes", "vote-cancel", "activity-log", "announce-throwdowns", "parties",
    "party-size", "speed-check", "score-check", "score-allow", "admin", "admins", "update", "quit", "plugins", "version",
];

#[derive(Clone, Default)]
pub struct PlayerInfo {
    pub id: u64,
    pub name: String,
    pub admin: bool,
    pub online_seconds: u64, // since the player's connection was made
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub players: Vec<PlayerInfo>,
    pub server: String,
    pub map: String,
    pub max_players: u32,
    pub password: bool,
    pub listed: bool,
    pub uptime_seconds: u64,
}

pub enum Action {
    Broadcast(String),
    Tell(u64, String),
    Log(String),
    Run(String, String), // plugin name, command
}

struct Command {
    plugin: usize,
    name: String,
    description: String,
    usage: String,
    admin: bool,
    handler: Function,
}

struct Timer {
    id: u64,
    plugin: usize,
    due: u64,
    every: Option<u64>,
    handler: Function,
}

struct AutoMessage {
    id: u64,
    plugin: usize,
    due: u64,
    every: u64,
    messages: Vec<String>,
    random: bool,
    min_players: u32,
    next: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Event {
    Join,
    Leave,
    Chat,
    Map,
    Stop,
}

// What all plugins share with the API functions they call.
#[derive(Default)]
struct Core {
    names: Vec<String>,
    actions: Vec<Action>,
    snapshot: Snapshot,
    now: u64,
    commands: BTreeMap<String, Rc<Command>>, // every name and alias
    timers: Vec<Timer>,
    automessages: Vec<AutoMessage>,
    events: Vec<(Event, usize, Function)>,
    next_id: u64,
    random: u64,
}

impl Core {
    fn forget(&mut self, plugin: usize) {
        self.commands.retain(|_, c| c.plugin != plugin);
        self.timers.retain(|t| t.plugin != plugin);
        self.automessages.retain(|a| a.plugin != plugin);
        self.events.retain(|e| e.1 != plugin);
    }
    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    fn next_random(&mut self) -> u64 {
        // xorshift64; seeded once, good enough to shuffle chat lines.
        let mut x = self.random;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random = x;
        x
    }
    fn format(&self, text: &str) -> String {
        text.replace("{players}", &self.snapshot.players.len().to_string())
            .replace("{max_players}", &self.snapshot.max_players.to_string())
            .replace("{server}", &self.snapshot.server)
            .replace("{map}", &self.snapshot.map)
    }
}

struct Plugin {
    name: String,
    lua: Lua,
}

pub struct PluginManager {
    dir: PathBuf,
    plugins: Vec<Plugin>,
    core: Rc<RefCell<Core>>,
    deadline: Rc<Cell<Option<Instant>>>,
}

impl Default for PluginManager {
    fn default() -> Self {
        PluginManager { dir: PathBuf::new(), plugins: Vec::new(), core: Rc::default(), deadline: Rc::default() }
    }
}

fn player_table(lua: &Lua, player: &PlayerInfo) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("id", player.id.to_string())?;
    table.set("name", player.name.as_str())?;
    table.set("admin", player.admin)?;
    table.set("online_seconds", player.online_seconds)?;
    Ok(table)
}

// reskate.save(): a Lua value as JSON. A table whose keys are exactly 1..n is a list; any other
// table an object, its keys written as text. Functions and the like cannot be saved.
fn lua_to_json(value: &Value, depth: usize) -> Result<serde_json::Value, String> {
    if depth > DATA_DEPTH {
        return Err(format!("tables nest deeper than {DATA_DEPTH} levels"));
    }
    Ok(match value {
        Value::Nil => serde_json::Value::Null,
        Value::Boolean(b) => (*b).into(),
        Value::Integer(i) => (*i).into(),
        Value::Number(n) => serde_json::Number::from_f64(*n).map(serde_json::Value::Number).ok_or("numbers must be finite")?,
        Value::String(s) => s.to_str().map_err(|_| "text must be UTF-8")?.to_string().into(),
        Value::Table(table) => {
            let length = table.raw_len();
            let mut entries = Vec::new();
            for pair in table.pairs::<Value, Value>() {
                entries.push(pair.map_err(|e| e.to_string())?);
            }
            let list = length > 0
                && entries.len() == length
                && entries.iter().all(|(k, _)| matches!(k, Value::Integer(i) if *i >= 1 && *i as usize <= length));
            if list {
                let mut items = vec![serde_json::Value::Null; length];
                for (key, item) in &entries {
                    if let Value::Integer(i) = key {
                        items[*i as usize - 1] = lua_to_json(item, depth + 1)?;
                    }
                }
                serde_json::Value::Array(items)
            } else {
                let mut object = serde_json::Map::new();
                for (key, item) in &entries {
                    let key = match key {
                        Value::String(s) => s.to_str().map_err(|_| "keys must be UTF-8")?.to_string(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) if n.is_finite() => n.to_string(),
                        Value::Boolean(b) => b.to_string(),
                        other => return Err(format!("a {} cannot be a key", other.type_name())),
                    };
                    object.insert(key, lua_to_json(item, depth + 1)?);
                }
                serde_json::Value::Object(object)
            }
        }
        other => return Err(format!("a {} cannot be saved", other.type_name())),
    })
}

fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    Ok(match value {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(s) => Value::String(lua.create_string(s)?),
        serde_json::Value::Array(items) => {
            let table = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                table.raw_set(i + 1, json_to_lua(lua, item)?)?;
            }
            Value::Table(table)
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (key, item) in map {
                table.raw_set(key.as_str(), json_to_lua(lua, item)?)?;
            }
            Value::Table(table)
        }
    })
}

// Writes a plugin's data: whole or not at all (a crash mid-write leaves the old file).
fn write_data(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let partial = file.with_extension("json.tmp");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, file)
}

// A player table, a SteamID64 string or number, or the start of a connected player's name.
fn resolve_player(core: &Core, value: &Value) -> Option<u64> {
    let text = match value {
        Value::Table(t) => t.get::<String>("id").ok()?,
        Value::Integer(i) => i.to_string(),
        Value::String(s) => s.to_str().ok()?.to_string(),
        _ => return None,
    };
    if let Ok(id) = text.parse::<u64>() {
        if core.snapshot.players.iter().any(|p| p.id == id) {
            return Some(id);
        }
    }
    let wanted = text.to_lowercase();
    let mut found = core.snapshot.players.iter().filter(|p| p.name.to_lowercase().starts_with(&wanted));
    match (found.next(), found.next()) {
        (Some(p), None) => Some(p.id),
        _ => None,
    }
}

fn seconds_to_us(seconds: f64, minimum: u64, what: &str) -> mlua::Result<u64> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(mlua::Error::runtime(format!("{what} must be a positive number of seconds")));
    }
    Ok(((seconds * 1_000_000.0) as u64).max(minimum))
}

fn valid_command_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

// The first lines of a Lua error: enough to find the problem without flooding the log.
fn error_text(error: &mlua::Error) -> String {
    let text = error.to_string();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(4).collect();
    lines.join(" | ")
}

impl PluginManager {
    pub fn load(&mut self, dir: &Path, now: u64) -> Vec<String> {
        self.dir = dir.to_path_buf();
        self.core.borrow_mut().now = now;
        self.reload()
    }

    // (Re)loads every plugin in the folder. Returns lines for the log.
    pub fn reload(&mut self) -> Vec<String> {
        self.plugins.clear();
        {
            let mut core = self.core.borrow_mut();
            let (random, now) = (core.random, core.now);
            *core = Core::default();
            core.now = now;
            core.random = if random != 0 {
                random
            } else {
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1
            };
        }
        let mut lines = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return lines;
        };
        let mut found: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name.starts_with('.') || file_name.starts_with('_') {
                continue; // a leading "_" turns a plugin off
            }
            if path.is_dir() {
                let main = path.join("main.lua");
                if main.is_file() {
                    found.push((file_name, main));
                }
            } else if let Some(name) = file_name.strip_suffix(".lua") {
                found.push((name.to_string(), path));
            }
        }
        found.sort();
        for (name, path) in found {
            match self.load_one(&name, &path) {
                Ok(()) => {}
                Err(e) => lines.push(format!("[plugins] {name} failed to load: {e}")),
            }
        }
        lines.push(self.summary());
        lines
    }

    fn load_one(&mut self, name: &str, path: &Path) -> Result<(), String> {
        let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let index = self.plugins.len();
        let lua = self.sandbox(name, index).map_err(|e| error_text(&e))?;
        self.core.borrow_mut().names.truncate(index);
        self.core.borrow_mut().names.push(name.to_string());
        self.deadline.set(Some(Instant::now() + LOAD_LIMIT));
        let result = lua.load(source).set_name(format!("@{name}")).exec();
        self.deadline.set(None);
        if let Err(e) = result {
            self.core.borrow_mut().forget(index);
            self.core.borrow_mut().names.truncate(index);
            return Err(error_text(&e));
        }
        self.plugins.push(Plugin { name: name.to_string(), lua });
        Ok(())
    }

    // A Lua state with only the safe standard libraries and the reskate API.
    fn sandbox(&self, name: &str, index: usize) -> mlua::Result<Lua> {
        let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE | StdLib::OS, LuaOptions::default())?;
        lua.set_memory_limit(MEMORY_LIMIT)?;
        let deadline = self.deadline.clone();
        lua.set_global_hook(HookTriggers::new().every_nth_instruction(1000), move |_, _| match deadline.get() {
            Some(limit) if Instant::now() > limit => Err(mlua::Error::runtime("the plugin took too long and was stopped")),
            _ => Ok(VmState::Continue),
        })?;
        let globals = lua.globals();
        for unsafe_global in ["dofile", "loadfile", "collectgarbage"] {
            globals.set(unsafe_global, Value::Nil)?;
        }
        // os: only the clock functions.
        let os: Table = globals.get("os")?;
        let safe_os = lua.create_table()?;
        for key in ["time", "clock", "date", "difftime"] {
            safe_os.set(key, os.get::<Value>(key)?)?;
        }
        globals.set("os", safe_os)?;

        let api = lua.create_table()?;
        let core = self.core.clone();
        let plugin_name = name.to_string();
        let log_name = plugin_name.clone();
        globals.set(
            "print",
            lua.create_function(move |_, values: Variadic<Value>| {
                let text: Vec<String> = values.iter().map(|v| v.to_string().unwrap_or_default()).collect();
                core.borrow_mut().actions.push(Action::Log(format!("[{log_name}] {}", text.join(" "))));
                Ok(())
            })?,
        )?;

        let info = lua.create_table()?;
        info.set("name", plugin_name.as_str())?;
        api.set("plugin", info)?;
        api.set("version", crate::update::VERSION)?;

        let core = self.core.clone();
        api.set(
            "command",
            lua.create_function(move |_, (name, second, third): (String, Value, Value)| {
                let (options, handler) = match (second, third) {
                    (Value::Function(f), Value::Nil) => (None, f),
                    (Value::Table(t), Value::Function(f)) => (Some(t), f),
                    _ => return Err(mlua::Error::runtime("usage: reskate.command(name, [options,] function(player, args, line) ... end)")),
                };
                let name = name.trim_start_matches('/').to_lowercase();
                let mut aliases = Vec::new();
                let (mut description, mut usage, mut admin) = (String::new(), String::new(), false);
                if let Some(options) = &options {
                    description = options.get::<Option<String>>("description")?.unwrap_or_default();
                    usage = options.get::<Option<String>>("usage")?.unwrap_or_default();
                    admin = options.get::<Option<bool>>("admin")?.unwrap_or(false);
                    if let Some(list) = options.get::<Option<Vec<String>>>("aliases")? {
                        aliases = list.into_iter().map(|a| a.trim_start_matches('/').to_lowercase()).collect();
                    }
                }
                let mut core = core.borrow_mut();
                for word in std::iter::once(&name).chain(aliases.iter()) {
                    if !valid_command_name(word) {
                        return Err(mlua::Error::runtime(format!("\"{word}\" is not a valid command name (a-z, 0-9, - and _, up to 32)")));
                    }
                    if RESERVED.contains(&word.as_str()) {
                        return Err(mlua::Error::runtime(format!("/{word} is a server command and cannot be replaced")));
                    }
                    if let Some(other) = core.commands.get(word) {
                        let owner = core.names.get(other.plugin).cloned().unwrap_or_default();
                        return Err(mlua::Error::runtime(format!("/{word} is already registered by plugin {owner}")));
                    }
                }
                let command = Rc::new(Command { plugin: index, name: name.clone(), description, usage, admin, handler });
                for word in std::iter::once(name).chain(aliases) {
                    core.commands.insert(word, command.clone());
                }
                Ok(())
            })?,
        )?;

        let core = self.core.clone();
        api.set(
            "automessage",
            lua.create_function(move |_, options: Table| {
                let messages: Vec<String> = options.get::<Option<Vec<String>>>("messages")?.unwrap_or_default();
                if messages.is_empty() {
                    return Err(mlua::Error::runtime("reskate.automessage needs messages = { \"...\", ... }"));
                }
                let every = seconds_to_us(options.get::<Option<f64>>("interval")?.unwrap_or(300.0), MIN_AUTOMESSAGE_US, "interval")?;
                let delay = match options.get::<Option<f64>>("delay")? {
                    Some(d) => seconds_to_us(d, MIN_INTERVAL_US, "delay")?,
                    None => every,
                };
                let order = options.get::<Option<String>>("order")?.unwrap_or_else(|| "sequence".into());
                let random = match order.as_str() {
                    "sequence" => false,
                    "random" => true,
                    _ => return Err(mlua::Error::runtime("order must be \"sequence\" or \"random\"")),
                };
                let min_players = options.get::<Option<u32>>("min_players")?.unwrap_or(1);
                let mut core = core.borrow_mut();
                let id = core.new_id();
                let due = core.now + delay;
                core.automessages.push(AutoMessage { id, plugin: index, due, every, messages, random, min_players, next: 0 });
                Ok(id)
            })?,
        )?;

        for (key, repeat) in [("every", true), ("after", false)] {
            let core = self.core.clone();
            api.set(
                key,
                lua.create_function(move |_, (seconds, handler): (f64, Function)| {
                    let us = seconds_to_us(seconds, MIN_INTERVAL_US, "the time")?;
                    let mut core = core.borrow_mut();
                    let id = core.new_id();
                    let due = core.now + us;
                    core.timers.push(Timer { id, plugin: index, due, every: repeat.then_some(us), handler });
                    Ok(id)
                })?,
            )?;
        }

        let core = self.core.clone();
        api.set(
            "cancel",
            lua.create_function(move |_, id: u64| {
                let mut core = core.borrow_mut();
                let before = core.timers.len() + core.automessages.len();
                core.timers.retain(|t| !(t.id == id && t.plugin == index));
                core.automessages.retain(|a| !(a.id == id && a.plugin == index));
                Ok(before != core.timers.len() + core.automessages.len())
            })?,
        )?;

        let core = self.core.clone();
        api.set(
            "on",
            lua.create_function(move |_, (event, handler): (String, Function)| {
                let event = match event.as_str() {
                    "join" => Event::Join,
                    "leave" => Event::Leave,
                    "chat" => Event::Chat,
                    "map" => Event::Map,
                    "stop" => Event::Stop,
                    _ => return Err(mlua::Error::runtime(format!("unknown event \"{event}\" (join, leave, chat, map, stop)"))),
                };
                core.borrow_mut().events.push((event, index, handler));
                Ok(())
            })?,
        )?;

        let core = self.core.clone();
        api.set(
            "broadcast",
            lua.create_function(move |_, text: String| {
                core.borrow_mut().actions.push(Action::Broadcast(text));
                Ok(())
            })?,
        )?;

        let core = self.core.clone();
        api.set(
            "tell",
            lua.create_function(move |_, (target, text): (Value, String)| {
                let mut core = core.borrow_mut();
                let Some(id) = resolve_player(&core, &target) else { return Ok(false) };
                core.actions.push(Action::Tell(id, text));
                Ok(true)
            })?,
        )?;

        let core = self.core.clone();
        let log_name = plugin_name.clone();
        api.set(
            "log",
            lua.create_function(move |_, text: String| {
                core.borrow_mut().actions.push(Action::Log(format!("[{log_name}] {text}")));
                Ok(())
            })?,
        )?;

        let core = self.core.clone();
        let run_name = plugin_name.clone();
        api.set(
            "run",
            lua.create_function(move |_, command: String| {
                core.borrow_mut().actions.push(Action::Run(run_name.clone(), command));
                Ok(())
            })?,
        )?;

        let core = self.core.clone();
        api.set(
            "players",
            lua.create_function(move |lua, ()| {
                let core = core.borrow();
                let list = lua.create_table()?;
                for (i, player) in core.snapshot.players.iter().enumerate() {
                    list.set(i + 1, player_table(lua, player)?)?;
                }
                Ok(list)
            })?,
        )?;

        let core = self.core.clone();
        api.set(
            "find",
            lua.create_function(move |lua, query: Value| {
                let core = core.borrow();
                match resolve_player(&core, &query).and_then(|id| core.snapshot.players.iter().find(|p| p.id == id)) {
                    Some(player) => Ok(Value::Table(player_table(lua, player)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;

        let core = self.core.clone();
        api.set(
            "server",
            lua.create_function(move |lua, ()| {
                let core = core.borrow();
                let table = lua.create_table()?;
                table.set("name", core.snapshot.server.as_str())?;
                table.set("map", core.snapshot.map.as_str())?;
                table.set("players", core.snapshot.players.len())?;
                table.set("max_players", core.snapshot.max_players)?;
                table.set("password", core.snapshot.password)?;
                table.set("listed", core.snapshot.listed)?;
                table.set("uptime_seconds", core.snapshot.uptime_seconds)?;
                Ok(table)
            })?,
        )?;

        let core = self.core.clone();
        api.set("format", lua.create_function(move |_, text: String| Ok(core.borrow().format(&text)))?)?;

        // The plugin's own data, kept across restarts in plugins/data/<name>.json.
        let file = self.dir.join("data").join(format!("{plugin_name}.json"));
        let read = file.clone();
        api.set(
            "load",
            lua.create_function(move |lua, ()| {
                let text = match std::fs::read_to_string(&read) {
                    Ok(text) => text,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Table(lua.create_table()?)),
                    Err(e) => return Err(mlua::Error::runtime(format!("cannot read {}: {e}", read.display()))),
                };
                let value: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| mlua::Error::runtime(format!("{} is not valid JSON: {e}", read.display())))?;
                match json_to_lua(lua, &value)? {
                    Value::Table(table) => Ok(Value::Table(table)),
                    _ => Ok(Value::Table(lua.create_table()?)),
                }
            })?,
        )?;
        api.set(
            "save",
            lua.create_function(move |_, data: Table| {
                let json = lua_to_json(&Value::Table(data), 0).map_err(|e| mlua::Error::runtime(format!("cannot save: {e}")))?;
                let text = json.to_string();
                if text.len() > DATA_LIMIT {
                    return Err(mlua::Error::runtime(format!("cannot save: more than {} KiB", DATA_LIMIT / 1024)));
                }
                write_data(&file, &text).map_err(|e| mlua::Error::runtime(format!("cannot write {}: {e}", file.display())))?;
                Ok(true)
            })?,
        )?;

        globals.set("reskate", api)?;
        Ok(lua)
    }

    // Runs one plugin function under the time limit. Errors are logged, never raised.
    fn call<R: mlua::FromLuaMulti>(&self, plugin: usize, handler: &Function, args: impl mlua::IntoLuaMulti) -> Option<R> {
        self.deadline.set(Some(Instant::now() + CALL_LIMIT));
        let result = handler.call::<R>(args);
        self.deadline.set(None);
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                let name = self.plugins.get(plugin).map(|p| p.name.clone()).unwrap_or_default();
                self.core.borrow_mut().actions.push(Action::Log(format!("[{name}] error: {}", error_text(&e))));
                None
            }
        }
    }

    fn begin(&self, snapshot: Snapshot) {
        self.core.borrow_mut().snapshot = snapshot;
    }
    fn finish(&self) -> Vec<Action> {
        std::mem::take(&mut self.core.borrow_mut().actions)
    }
    fn lua(&self, plugin: usize) -> Option<&Lua> {
        self.plugins.get(plugin).map(|p| &p.lua)
    }

    // A player's /command. None when no plugin has it; otherwise the actions and the reply.
    pub fn command(&self, snapshot: Snapshot, player: &PlayerInfo, verb: &str, line: &str) -> Option<(Vec<Action>, String)> {
        let command = self.core.borrow().commands.get(verb).cloned()?;
        if command.admin && !player.admin {
            return Some((Vec::new(), format!("Only admins can use /{verb}.")));
        }
        self.begin(snapshot);
        let lua = self.lua(command.plugin)?;
        let args = lua.create_sequence_from(line.split_whitespace().map(str::to_string)).ok()?;
        let table = player_table(lua, player).ok()?;
        let reply = match self.call::<Value>(command.plugin, &command.handler, (table, args, line.to_string())) {
            Some(Value::Nil) => String::new(),
            Some(value) => value.to_string().unwrap_or_default(),
            None => "That command failed. The server log has the details.".to_string(),
        };
        Some((self.finish(), reply))
    }

    // One line per command for /help, admin-only ones for admins.
    pub fn help(&self, admin: bool) -> String {
        let core = self.core.borrow();
        let names: Vec<String> = core
            .commands
            .iter()
            .filter(|(word, c)| **word == c.name && (admin || !c.admin))
            .map(|(word, _)| format!("/{word}"))
            .collect();
        if names.is_empty() {
            return String::new();
        }
        format!("Server commands: {} (/help <command> for more)", names.join(" "))
    }

    pub fn describe(&self, verb: &str, admin: bool) -> Option<String> {
        let core = self.core.borrow();
        let command = core.commands.get(verb)?;
        if command.admin && !admin {
            return None;
        }
        let usage = if command.usage.is_empty() { format!("/{}", command.name) } else { format!("/{} {}", command.name, command.usage) };
        Some(if command.description.is_empty() { usage } else { format!("{usage}: {}", command.description) })
    }

    fn dispatch(&mut self, event: Event, snapshot: Snapshot, player: &PlayerInfo, extra: Option<&str>) -> (Vec<Action>, bool) {
        let handlers: Vec<(usize, Function)> =
            self.core.borrow().events.iter().filter(|(e, _, _)| *e == event).map(|(_, p, f)| (*p, f.clone())).collect();
        if handlers.is_empty() {
            return (Vec::new(), true);
        }
        self.begin(snapshot);
        let mut allowed = true;
        for (plugin, handler) in handlers {
            let Some(lua) = self.lua(plugin) else { continue };
            let Ok(table) = player_table(lua, player) else { continue };
            let result = match extra {
                Some(text) => self.call::<Value>(plugin, &handler, (table, text.to_string())),
                None => self.call::<Value>(plugin, &handler, table),
            };
            if matches!(result, Some(Value::Boolean(false))) {
                allowed = false;
            }
        }
        (self.finish(), allowed)
    }

    pub fn join(&mut self, snapshot: Snapshot, player: &PlayerInfo) -> Vec<Action> {
        self.dispatch(Event::Join, snapshot, player, None).0
    }
    pub fn leave(&mut self, snapshot: Snapshot, player: &PlayerInfo, reason: &str) -> Vec<Action> {
        self.dispatch(Event::Leave, snapshot, player, Some(reason)).0
    }
    // False when a plugin keeps the message from the other players.
    pub fn chat(&mut self, snapshot: Snapshot, player: &PlayerInfo, text: &str) -> (Vec<Action>, bool) {
        self.dispatch(Event::Chat, snapshot, player, Some(text))
    }

    // Events about the server rather than a player: the handlers get `text`.
    fn fire(&mut self, event: Event, snapshot: Snapshot, text: &str) -> Vec<Action> {
        let handlers: Vec<(usize, Function)> =
            self.core.borrow().events.iter().filter(|(e, _, _)| *e == event).map(|(_, p, f)| (*p, f.clone())).collect();
        if handlers.is_empty() {
            return Vec::new();
        }
        self.begin(snapshot);
        for (plugin, handler) in handlers {
            self.call::<()>(plugin, &handler, text.to_string());
        }
        self.finish()
    }
    // The server changed to `map` (its name as players see it).
    pub fn map(&mut self, snapshot: Snapshot, map: &str) -> Vec<Action> {
        self.fire(Event::Map, snapshot, map)
    }
    // The server is shutting down or restarting; `reason` as players are told.
    pub fn stop(&mut self, snapshot: Snapshot, reason: &str) -> Vec<Action> {
        self.fire(Event::Stop, snapshot, reason)
    }

    // Advances the plugins' clock; true when a timer or automatic message is due.
    pub fn due(&self, now: u64) -> bool {
        let mut core = self.core.borrow_mut();
        core.now = now;
        core.timers.iter().any(|t| now >= t.due) || core.automessages.iter().any(|a| now >= a.due)
    }

    // Runs what is due (after due() said so).
    pub fn tick(&mut self, now: u64, snapshot: Snapshot) -> Vec<Action> {
        self.begin(snapshot);
        let mut lines = Vec::new();
        {
            let mut core = self.core.borrow_mut();
            let players = core.snapshot.players.len() as u32;
            for i in 0..core.automessages.len() {
                if now < core.automessages[i].due {
                    continue;
                }
                core.automessages[i].due = now + core.automessages[i].every;
                if players == 0 || players < core.automessages[i].min_players {
                    continue;
                }
                let count = core.automessages[i].messages.len();
                let pick = if core.automessages[i].random { (core.next_random() % count as u64) as usize } else { core.automessages[i].next % count };
                core.automessages[i].next = pick + 1;
                let text = core.format(&core.automessages[i].messages[pick]);
                lines.push(text);
            }
        }
        let fired: Vec<(u64, usize, Function)> = {
            let mut core = self.core.borrow_mut();
            let mut fired = Vec::new();
            core.timers.retain_mut(|t| {
                if now < t.due {
                    return true;
                }
                fired.push((t.id, t.plugin, t.handler.clone()));
                match t.every {
                    Some(every) => {
                        t.due = now + every;
                        true
                    }
                    None => false,
                }
            });
            fired
        };
        for (_, plugin, handler) in fired {
            self.call::<()>(plugin, &handler, ());
        }
        let mut actions: Vec<Action> = lines.into_iter().map(Action::Broadcast).collect();
        actions.extend(self.finish());
        actions
    }

    pub fn summary(&self) -> String {
        if self.plugins.is_empty() {
            return format!("[plugins] No plugins loaded (put .lua files in {}).", self.dir.display());
        }
        let core = self.core.borrow();
        let rows: Vec<String> = self
            .plugins
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let commands: Vec<String> =
                    core.commands.iter().filter(|(w, c)| c.plugin == i && **w == c.name).map(|(w, _)| format!("/{w}")).collect();
                let messages = core.automessages.iter().filter(|a| a.plugin == i).count();
                let timers = core.timers.iter().filter(|t| t.plugin == i).count();
                let mut parts = Vec::new();
                if !commands.is_empty() {
                    parts.push(commands.join(" "));
                }
                if messages > 0 {
                    parts.push(format!("{messages} automessage(s)"));
                }
                if timers > 0 {
                    parts.push(format!("{timers} timer(s)"));
                }
                if parts.is_empty() {
                    p.name.clone()
                } else {
                    format!("{} ({})", p.name, parts.join(", "))
                }
            })
            .collect();
        format!("[plugins] {} loaded: {}", self.plugins.len(), rows.join("; "))
    }
}
