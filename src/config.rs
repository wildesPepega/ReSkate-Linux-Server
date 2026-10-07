// ReSkateServer.json and the server's maps (Server/server_config.{h,cpp}). Every setting an
// admin or the console changes is saved back, so a restart keeps it.
use crate::protocol::{individual_steam_id, valid_chat_text, valid_map_destination, valid_member_name, valid_multiplayer_tps};
use crate::protocol::{placement, valid_voice_range, Ban, Distances, DEFAULT_TPS, DEFAULT_VOICE_RANGE, MAX_MAP_ROTATION, MAX_PLAYERS};
use crate::world::{valid_park, world_destination_asset, world_level_name, world_level_short_name, ParkChoices, PARK_LOTS};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Clone, Copy)]
pub struct VoteSetting {
    pub enabled: bool,
    pub percent: u32,
}

#[derive(Clone)]
pub struct VoteSettings {
    pub map: VoteSetting,
    pub kick: VoteSetting,
    pub time: VoteSetting,
    pub seconds: u32,
    pub cooldown: u32,
}
impl Default for VoteSettings {
    fn default() -> Self {
        VoteSettings {
            map: VoteSetting { enabled: false, percent: 60 },
            kick: VoteSetting { enabled: false, percent: 60 },
            time: VoteSetting { enabled: false, percent: 50 },
            seconds: 30,
            cooldown: 60,
        }
    }
}

#[derive(Clone)]
pub struct ServerConfig {
    pub file: PathBuf,
    pub name: String,
    pub map: String,
    pub map_pool: Vec<String>, // maps for votes and the rotation, in order; empty: every map
    pub map_rotation: u32,     // minutes per map before the next pool map (0: off)
    pub map_mods: Vec<String>, // Thunderstore links of custom maps fetched at startup (src/thunderstore.rs)
    pub max_players: u32,
    pub password: String,
    pub welcome: String,
    pub listed: bool,
    pub auto_update: bool,
    pub global_bans: bool, // turn away players the ReSkate team has banned (src/global_bans.rs)
    pub activity_log: bool,
    pub announce_throwdowns: bool,
    pub parties: bool,
    pub party_size: u32,
    pub speed_check: String,
    pub score_check: String,
    pub score_allow: Vec<u64>,
    // From --port / --query-port (a hosting panel's allocations), not from the file.
    pub port: u16,
    pub query_port: u16,
    pub tps: u32,
    pub voice_chat: bool,
    pub voice_range: f32,
    pub distances: Distances,
    pub object_placement: u8,
    pub noclip: bool,
    pub no_bail: bool,
    pub boosts: bool,
    pub enforce_tuning: bool,
    pub votes: VoteSettings,
    pub parks: ParkChoices,
    pub world_layer_sync: bool,
    pub layers: BTreeMap<String, String>,
    pub admins: Vec<u64>,
    pub bans: Vec<Ban>,
    pub discord_webhook: String,
    pub discord_events: Vec<String>,
    pub discord_style: String,
    // GET /status over TCP on the game port (src/status.rs), and whether it names the players.
    pub status_enabled: bool,
    pub status_players: bool,
    // Settings an older file had that are no longer read, as "key = value", for the log.
    pub dropped: Vec<String>,
}

// What a new config sends to a Discord webhook (docs/configuration.md#discord).
pub const DEFAULT_DISCORD_EVENTS: &[&str] = &["start", "stop", "update", "join", "leave", "throwdown", "vote", "anticheat", "admin"];

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            file: PathBuf::new(),
            name: "ReSkate server".into(),
            map: "San Vansterdam".into(),
            map_pool: Vec::new(),
            map_rotation: 0,
            map_mods: Vec::new(),
            max_players: 16,
            password: String::new(),
            welcome: String::new(),
            listed: true,
            auto_update: true,
            global_bans: true,
            activity_log: true,
            announce_throwdowns: true,
            parties: true,
            party_size: 8,
            speed_check: "warn".into(),
            score_check: "warn".into(),
            score_allow: Vec::new(),
            port: 27015,
            query_port: 27016,
            tps: DEFAULT_TPS,
            voice_chat: true,
            voice_range: DEFAULT_VOICE_RANGE,
            distances: Distances::default(),
            object_placement: placement::EVERYONE,
            noclip: true,
            no_bail: true,
            boosts: true,
            enforce_tuning: true,
            votes: VoteSettings::default(),
            parks: ["skatepark_01".into(), "megapark_05".into(), "flumppark_08".into()],
            world_layer_sync: false,
            layers: BTreeMap::new(),
            admins: Vec::new(),
            bans: Vec::new(),
            discord_webhook: String::new(),
            discord_events: DEFAULT_DISCORD_EVENTS.iter().map(|e| e.to_string()).collect(),
            discord_style: "embed".into(),
            status_enabled: true,
            status_players: true,
            dropped: Vec::new(),
        }
    }
}

fn placement_text(policy: u8) -> &'static str {
    match policy {
        placement::EVERYONE => "everyone",
        placement::HOST_ONLY => "admins",
        _ => "nobody",
    }
}

fn to_json(c: &ServerConfig) -> Value {
    let mut root = Map::new();
    root.insert("name".into(), c.name.clone().into());
    root.insert("map".into(), c.map.clone().into());
    root.insert("map_pool".into(), Value::Array(c.map_pool.iter().map(|m| m.clone().into()).collect()));
    root.insert("map_rotation_minutes".into(), c.map_rotation.into());
    root.insert("map_mods".into(), Value::Array(c.map_mods.iter().map(|m| m.clone().into()).collect()));
    root.insert("max_players".into(), c.max_players.into());
    root.insert("password".into(), c.password.clone().into());
    root.insert("welcome".into(), c.welcome.clone().into());
    root.insert("listed".into(), c.listed.into());
    root.insert("auto_update".into(), c.auto_update.into());
    root.insert("global_bans".into(), c.global_bans.into());
    root.insert("activity_log".into(), c.activity_log.into());
    root.insert("announce_throwdowns".into(), c.announce_throwdowns.into());
    root.insert("parties".into(), c.parties.into());
    root.insert("party_size".into(), c.party_size.into());
    root.insert("speed_check".into(), c.speed_check.clone().into());
    root.insert("score_check".into(), c.score_check.clone().into());
    root.insert("score_allow".into(), Value::Array(c.score_allow.iter().map(|&f| scoring_text(f).into()).collect()));
    root.insert("tps".into(), c.tps.into());
    root.insert("voice_chat".into(), c.voice_chat.into());
    root.insert("voice_range".into(), f64::from(c.voice_range).into());
    let mut distances = Map::new();
    distances.insert("full_rate_return".into(), c.distances.full_rate_return.into());
    distances.insert("half_rate_start".into(), c.distances.half_rate_start.into());
    distances.insert("half_rate_return".into(), c.distances.half_rate_return.into());
    distances.insert("low_rate_start".into(), c.distances.low_rate_start.into());
    root.insert("distances".into(), Value::Object(distances));
    root.insert("object_placement".into(), placement_text(c.object_placement).into());
    root.insert("noclip".into(), c.noclip.into());
    root.insert("no_bail".into(), c.no_bail.into());
    root.insert("boosts".into(), c.boosts.into());
    root.insert("enforce_tuning".into(), c.enforce_tuning.into());
    let vote = |v: &VoteSetting| {
        let mut item = Map::new();
        item.insert("enabled".into(), v.enabled.into());
        item.insert("percent".into(), v.percent.into());
        Value::Object(item)
    };
    let mut votes = Map::new();
    votes.insert("map".into(), vote(&c.votes.map));
    votes.insert("kick".into(), vote(&c.votes.kick));
    votes.insert("time_of_day".into(), vote(&c.votes.time));
    votes.insert("seconds".into(), c.votes.seconds.into());
    votes.insert("cooldown_seconds".into(), c.votes.cooldown.into());
    root.insert("votes".into(), Value::Object(votes));
    let mut parks = Map::new();
    for (lot, park) in PARK_LOTS.iter().enumerate() {
        parks.insert(park.key.into(), c.parks[lot].clone().into());
    }
    root.insert("parks".into(), Value::Object(parks));
    root.insert("world_layer_sync".into(), c.world_layer_sync.into());
    let mut layers = Map::new();
    for (key, mode) in &c.layers {
        layers.insert(key.clone(), mode.clone().into());
    }
    root.insert("layers".into(), Value::Object(layers));
    // SteamID64s are written as strings: JSON readers often lose 64-bit precision.
    root.insert("admins".into(), Value::Array(c.admins.iter().map(|id| id.to_string().into()).collect()));
    let bans = c
        .bans
        .iter()
        .map(|ban| {
            let mut row = Map::new();
            row.insert("id".into(), ban.id.to_string().into());
            row.insert("name".into(), ban.name.clone().into());
            row.insert("added".into(), ban.added.into());
            Value::Object(row)
        })
        .collect();
    root.insert("bans".into(), Value::Array(bans));
    let mut discord = Map::new();
    discord.insert("webhook".into(), c.discord_webhook.clone().into());
    discord.insert("events".into(), Value::Array(c.discord_events.iter().map(|e| e.clone().into()).collect()));
    discord.insert("style".into(), c.discord_style.clone().into());
    root.insert("discord".into(), Value::Object(discord));
    let mut status = Map::new();
    status.insert("enabled".into(), c.status_enabled.into());
    status.insert("players".into(), c.status_players.into());
    root.insert("status".into(), Value::Object(status));
    Value::Object(root)
}

// ---- Typed reads, with the C++ Json class's rules -------------------------------------------
fn number_u64(value: &Value) -> Result<u64, String> {
    if let Some(v) = value.as_u64() {
        Ok(v)
    } else if let Some(v) = value.as_i64() {
        Ok(v as u64)
    } else if let Some(v) = value.as_f64() {
        Ok(v as u64)
    } else {
        Err("JSON value must be numeric".into())
    }
}
fn number_f64(value: &Value) -> Result<f64, String> {
    value.as_f64().ok_or_else(|| "JSON value must be numeric".to_string())
}
fn number_i64(value: &Value) -> Result<i64, String> {
    if let Some(v) = value.as_i64() {
        Ok(v)
    } else if let Some(v) = value.as_u64() {
        Ok(v as i64)
    } else if let Some(v) = value.as_f64() {
        Ok(v as i64)
    } else {
        Err("JSON value must be numeric".into())
    }
}
fn read_u32(root: &Value, key: &str, fallback: u32) -> Result<u32, String> {
    match root.get(key) {
        Some(v) => Ok(number_u64(v)? as u32),
        None => Ok(fallback),
    }
}
fn read_i32(root: &Value, key: &str, fallback: i32) -> Result<i32, String> {
    match root.get(key) {
        Some(v) => Ok(number_i64(v)? as i32),
        None => Ok(fallback),
    }
}
fn read_bool(root: &Value, key: &str, fallback: bool) -> Result<bool, String> {
    match root.get(key) {
        Some(v) => v.as_bool().ok_or_else(|| format!("\"{key}\" must be true or false")),
        None => Ok(fallback),
    }
}
fn read_string(root: &Value, key: &str, fallback: &str) -> Result<String, String> {
    match root.get(key) {
        Some(v) => v.as_str().map(str::to_string).ok_or_else(|| format!("\"{key}\" must be text")),
        None => Ok(fallback.to_string()),
    }
}
// std::stoull: leading blanks, then digits.
fn steam_id(value: &Value) -> Result<u64, String> {
    if let Some(text) = value.as_str() {
        let digits: String = text.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return Err("invalid stoull argument".into());
        }
        return digits.parse::<u64>().map_err(|_| "stoull argument out of range".to_string());
    }
    number_u64(value)
}

// The settings `expected` has that `file` lacks, nested objects included.
fn missing_settings(file: &Value, expected: &Value, prefix: &str, out: &mut Vec<String>) {
    let Some(expected) = expected.as_object() else { return };
    for (key, value) in expected {
        match file.get(key) {
            None => out.push(format!("{prefix}{key}")),
            Some(found) if value.is_object() && found.is_object() => {
                missing_settings(found, value, &format!("{prefix}{key}."), out)
            }
            _ => {}
        }
    }
}

// Reads `file`, writing a default one first when it does not exist. Settings this version has
// that the file lacks are written back with their defaults, and their names go to `added`.
pub fn load_config(file: &Path, added: &mut Vec<String>) -> Result<ServerConfig, String> {
    let mut c = ServerConfig { file: file.to_path_buf(), ..Default::default() };
    if !file.exists() {
        save_config(&c)?;
        return Ok(c);
    }
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let root: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if !root.is_object() {
        return Err("The server config must be a JSON object.".into());
    }
    c.name = read_string(&root, "name", &c.name)?;
    c.map = read_string(&root, "map", &c.map)?;
    if let Some(Value::Array(list)) = root.get("map_pool") {
        c.map_pool = list.iter().filter_map(|m| m.as_str()).filter(|m| !m.is_empty()).map(str::to_string).collect();
    }
    c.map_rotation = read_u32(&root, "map_rotation_minutes", c.map_rotation)?.min(MAX_MAP_ROTATION);
    if let Some(Value::Array(list)) = root.get("map_mods") {
        c.map_mods = list.iter().filter_map(|m| m.as_str()).map(str::trim).filter(|m| !m.is_empty()).map(str::to_string).collect();
    }
    c.max_players = read_u32(&root, "max_players", c.max_players)?;
    c.password = read_string(&root, "password", &c.password)?;
    c.welcome = read_string(&root, "welcome", &c.welcome)?;
    c.listed = read_bool(&root, "listed", c.listed)?;
    c.auto_update = read_bool(&root, "auto_update", c.auto_update)?;
    c.global_bans = read_bool(&root, "global_bans", c.global_bans)?;
    c.activity_log = read_bool(&root, "activity_log", c.activity_log)?;
    c.announce_throwdowns = read_bool(&root, "announce_throwdowns", c.announce_throwdowns)?;
    c.parties = read_bool(&root, "parties", c.parties)?;
    c.party_size = read_u32(&root, "party_size", c.party_size)?.clamp(2, 8);
    c.speed_check = read_string(&root, "speed_check", &c.speed_check)?;
    if !matches!(c.speed_check.as_str(), "off" | "warn" | "kick") {
        c.speed_check = "warn".into();
    }
    c.score_check = read_string(&root, "score_check", &c.score_check)?;
    if !matches!(c.score_check.as_str(), "off" | "warn" | "kick") {
        c.score_check = "warn".into();
    }
    if let Some(Value::Array(list)) = root.get("score_allow") {
        for value in list {
            if let Some(fingerprint) = value.as_str().and_then(parse_scoring) {
                c.score_allow.push(fingerprint);
            }
        }
    }
    // The ports come from --port / --query-port now; older files had them.
    for key in ["port", "query_port"] {
        if let Some(value) = root.get(key) {
            c.dropped.push(format!("{key} = {value}"));
        }
    }
    if let Some(status) = root.get("status").filter(|s| s.is_object()) {
        c.status_enabled = read_bool(status, "enabled", c.status_enabled)?;
        c.status_players = read_bool(status, "players", c.status_players)?;
    }
    if let Some(discord) = root.get("discord").filter(|d| d.is_object()) {
        c.discord_webhook = read_string(discord, "webhook", "")?.trim().to_string();
        c.discord_style = read_string(discord, "style", &c.discord_style)?.trim().to_lowercase();
        if let Some(events) = discord.get("events") {
            let list = events.as_array().ok_or("\"discord.events\" must be a list")?;
            c.discord_events = list.iter().filter_map(|e| e.as_str()).map(|e| e.trim().to_lowercase()).collect();
        }
    }
    c.tps = read_u32(&root, "tps", c.tps)?;
    c.voice_chat = read_bool(&root, "voice_chat", c.voice_chat)?;
    if let Some(v) = root.get("voice_range") {
        c.voice_range = number_f64(v)? as f32;
    }
    if let Some(d) = root.get("distances") {
        if !d.is_object() {
            return Err("JSON value must be an object".into());
        }
        c.distances.full_rate_return = read_i32(d, "full_rate_return", c.distances.full_rate_return)?;
        c.distances.half_rate_start = read_i32(d, "half_rate_start", c.distances.half_rate_start)?;
        c.distances.half_rate_return = read_i32(d, "half_rate_return", c.distances.half_rate_return)?;
        c.distances.low_rate_start = read_i32(d, "low_rate_start", c.distances.low_rate_start)?;
    }
    c.noclip = read_bool(&root, "noclip", c.noclip)?;
    c.no_bail = read_bool(&root, "no_bail", c.no_bail)?;
    c.boosts = read_bool(&root, "boosts", c.boosts)?;
    c.enforce_tuning = read_bool(&root, "enforce_tuning", c.enforce_tuning)?;
    if let Some(votes) = root.get("votes").filter(|v| v.is_object()) {
        let read_vote = |key: &str, v: &mut VoteSetting| -> Result<(), String> {
            let Some(item) = votes.get(key).filter(|i| i.is_object()) else { return Ok(()) };
            v.enabled = read_bool(item, "enabled", v.enabled)?;
            v.percent = read_u32(item, "percent", v.percent)?.clamp(1, 100);
            Ok(())
        };
        read_vote("map", &mut c.votes.map)?;
        read_vote("kick", &mut c.votes.kick)?;
        read_vote("time_of_day", &mut c.votes.time)?;
        c.votes.seconds = read_u32(votes, "seconds", c.votes.seconds)?.clamp(10, 300);
        c.votes.cooldown = read_u32(votes, "cooldown_seconds", c.votes.cooldown)?.clamp(0, 3600);
    }
    let placement_name = read_string(&root, "object_placement", placement_text(c.object_placement))?;
    // On a dedicated server the protocol's "host only" means its admins.
    c.object_placement = match placement_name.as_str() {
        "nobody" => placement::NOBODY,
        "admins" | "host" => placement::HOST_ONLY,
        _ => placement::EVERYONE,
    };
    if let Some(parks) = root.get("parks") {
        if !parks.is_object() {
            return Err("JSON value must be an object".into());
        }
        for (lot, park) in PARK_LOTS.iter().enumerate() {
            c.parks[lot] = read_string(parks, park.key, &c.parks[lot])?;
        }
    }
    c.world_layer_sync = read_bool(&root, "world_layer_sync", c.world_layer_sync)?;
    if let Some(Value::Object(layers)) = root.get("layers") {
        for (key, mode) in layers {
            if let Some(mode) = mode.as_str() {
                c.layers.insert(key.clone(), mode.to_string());
            }
        }
    }
    if let Some(admins) = root.get("admins") {
        for id in admins.as_array().ok_or("\"admins\" must be a list")? {
            c.admins.push(steam_id(id)?);
        }
    }
    if let Some(bans) = root.get("bans") {
        for row in bans.as_array().ok_or("\"bans\" must be a list")? {
            let id = steam_id(row.get("id").ok_or("Missing JSON field: id")?)?;
            let name = read_string(row, "name", "")?;
            let added = match row.get("added") {
                Some(v) => number_i64(v)?,
                None => 0,
            };
            c.bans.push(Ban { id, name, added });
        }
    }
    // A config from an older version: write the new settings into it so owners can see them.
    let mut missing = Vec::new();
    missing_settings(&root, &to_json(&c), "", &mut missing);
    if !missing.is_empty() || !c.dropped.is_empty() {
        save_config(&c)?;
        *added = missing;
    }
    Ok(c)
}

pub fn save_config(c: &ServerConfig) -> Result<(), String> {
    if let Some(parent) = c.file.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let mut temporary = c.file.clone().into_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let mut text = serde_json::to_string_pretty(&to_json(c)).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(&temporary, text).map_err(|_| format!("Cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, &c.file).map_err(|e| e.to_string())
}

// A scoring fingerprint as the config and console write it (16 hex digits).
pub fn scoring_text(fingerprint: u64) -> String {
    format!("{fingerprint:016x}")
}
pub fn parse_scoring(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 16 || !text.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u64::from_str_radix(text, 16).ok()?;
    (value != 0).then_some(value)
}

// Why `config` cannot run, or empty.
pub fn config_error(c: &ServerConfig) -> String {
    if c.name.is_empty() || c.name.len() > 64 || !valid_member_name(c.name.as_bytes()) {
        return "name must be 1 to 64 characters.".into();
    }
    if c.map.is_empty() || !valid_map_destination(&map_destination(&c.map)) {
        return format!(
            "map \"{}\" is not a known map. Use a name like \"San Vansterdam\", or put the map's mod folder in Mods next to the server.",
            c.map
        );
    }
    for map in &c.map_pool {
        if find_level(map).is_none() || !valid_map_destination(&map_destination(map)) {
            return format!(
                "map_pool: \"{map}\" is not a single known map. Use names like \"Isle of Grom\", or put the map's mod folder in Mods next to the server."
            );
        }
    }
    if c.max_players < 1 || c.max_players as usize + 1 > MAX_PLAYERS {
        return format!("max_players must be 1 to {}.", MAX_PLAYERS - 1);
    }
    if c.password.len() > 64 {
        return "password must be at most 64 characters.".into();
    }
    if !c.welcome.is_empty() && !valid_chat_text(c.welcome.as_bytes()) {
        return "welcome must be one chat line (at most 200 bytes).".into();
    }
    if !valid_multiplayer_tps(c.tps) {
        return "tps must be 20, 30, 60 or 120.".into();
    }
    if !valid_voice_range(c.voice_range) {
        return "voice_range must be 50 to 1000.".into();
    }
    if !c.distances.valid() {
        return "distances must be ordered: full_rate_return < half_rate_start <= half_rate_return < low_rate_start <= 10000.".into();
    }
    for (lot, park) in PARK_LOTS.iter().enumerate() {
        if c.parks[lot].is_empty() || !valid_park(lot, &c.parks[lot]) {
            return format!("parks.{} is not a park layout (e.g. skatepark_01, or empty).", park.key);
        }
    }
    if c.port == 0 || c.query_port == 0 || c.port == c.query_port {
        return "--port and --query-port must differ.".into();
    }
    if c.admins.iter().any(|&id| !individual_steam_id(id)) {
        return "admins must be SteamID64s (17 digits starting 7656119).".into();
    }
    String::new()
}

// ---- Maps ------------------------------------------------------------------------------------
// The game always has its root level loaded, and a map is the level loaded into it, named as
// the game's own `load` command names it.
#[derive(Clone)]
pub struct ServerLevel {
    pub asset: String,
    pub name: String,
}

const ROOT_LEVEL: &str = "Levels/Game/DingoLevel_Root/DingoLevel_Root";

static LEVELS: RwLock<Vec<ServerLevel>> = RwLock::new(Vec::new());

fn folded(text: &str) -> String {
    text.chars().map(|c| if c == '\\' { '/' } else { c.to_ascii_lowercase() }).collect()
}
fn same(a: &str, b: &str) -> bool {
    folded(a) == folded(b)
}
fn starts(text: &str, prefix: &str) -> bool {
    folded(text).starts_with(&folded(prefix))
}

// The retail maps, and the custom maps in `mods`/<mod>/reskate-levels.json. Returns why a mod
// was skipped.
pub fn load_levels(mods: &Path) -> Vec<String> {
    let mut list: Vec<ServerLevel> = [
        "Levels/Game/BAM_LevelRoot/BAM_LevelRoot",
        "Levels/Game/DingoLevel_Isle_of_Grom/DingoLevel_Isle_of_Grom",
        "Levels/Game/DingoLevel_MPR/DingoLevel_MPR",
        "Levels/Game/DingoLevel_FTUE_Island/DingoLevel_FTUE_Island",
        "Levels/Game/DingoLevel_SDM/DingoLevel_SDM_Int_001/DingoLevel_SDM_Int_001",
        "Levels/Game/DingoLevel_SDM/DingoLevel_SDM_Int_002/DingoLevel_SDM_Int_002",
    ]
    .iter()
    .map(|asset| ServerLevel { asset: asset.to_string(), name: world_level_name(asset) })
    .collect();
    let mut problems = Vec::new();
    if let Ok(entries) = std::fs::read_dir(mods) {
        let mut folders: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            let manifest = folder.join("reskate-levels.json");
            if !manifest.exists() {
                continue;
            }
            let result = (|| -> Result<(), String> {
                let text = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
                let root: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let levels = root.get("levels").ok_or("Missing JSON field: levels")?;
                let levels = levels.as_array().ok_or("\"levels\" must be a list")?;
                for level in levels {
                    let asset = level
                        .get("asset")
                        .ok_or("Missing JSON field: asset")?
                        .as_str()
                        .ok_or("\"asset\" must be text")?
                        .to_string();
                    let name = read_string(level, "displayName", &world_level_name(&asset))?;
                    if !list.iter().any(|l| same(&l.asset, &asset)) {
                        let name = if name.is_empty() { world_level_name(&asset) } else { name };
                        list.push(ServerLevel { asset, name });
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                problems.push(format!("{name}: {e}"));
            }
        }
    }
    *LEVELS.write().unwrap() = list;
    problems
}

pub fn levels() -> Vec<ServerLevel> {
    LEVELS.read().unwrap().clone()
}

// Like the game's `load`: a level path, a name or short name, or the unique start of one.
pub fn find_level(map: &str) -> Option<ServerLevel> {
    if map.is_empty() {
        return None;
    }
    let list = LEVELS.read().unwrap();
    if let Some(level) = list.iter().find(|l| same(&l.asset, map)) {
        return Some(level.clone());
    }
    for exact in [true, false] {
        let mut result: Option<&ServerLevel> = None;
        for level in list.iter() {
            let alias = world_level_short_name(&level.asset);
            let hit = if exact {
                same(&level.name, map) || same(&alias, map)
            } else {
                starts(&level.name, map) || starts(&alias, map) || starts(&level.asset, map)
            };
            if hit {
                if result.is_some() {
                    return None; // ambiguous
                }
                result = Some(level);
            }
        }
        if let Some(level) = result {
            return Some(level.clone());
        }
    }
    None
}

// What players load for a map, as the protocol carries it ("<root>|<level>").
pub fn map_destination(map: &str) -> String {
    if map.contains('|') {
        return map.to_string();
    }
    if let Some(level) = find_level(map) {
        return format!("{ROOT_LEVEL}|{}", level.asset);
    }
    // A level path the server has no manifest for: players with that map can still load it.
    if map.contains('/') {
        return format!("{ROOT_LEVEL}|{map}");
    }
    String::new()
}

// The name to store for a map, a level path or a destination.
pub fn map_setting(map: &str) -> String {
    let mut level = map;
    if let Some(split) = map.find('|') {
        let root = &map[..split];
        let detached = &map[split + 1..];
        // Anything not loaded into the usual root level keeps its full destination.
        if !same(root, ROOT_LEVEL) || detached.is_empty() {
            return map.to_string();
        }
        level = detached;
    }
    if let Some(known) = find_level(level) {
        return known.name;
    }
    level.to_string()
}

// A map's name for people: "San Vansterdam".
pub fn map_label(map: &str) -> String {
    if let Some(level) = find_level(map) {
        return level.name;
    }
    world_level_name(world_destination_asset(&map_destination(map)))
}

// The maps players vote between and the rotation goes through, each once, in the pool's order;
// every known map when the pool is empty.
pub fn pool_levels(c: &ServerConfig) -> Vec<ServerLevel> {
    let mut pool: Vec<ServerLevel> = Vec::new();
    let mut add = |level: Option<ServerLevel>| {
        if let Some(level) = level {
            if valid_map_destination(&map_destination(&level.asset)) && !pool.iter().any(|p| same(&p.asset, &level.asset)) {
                pool.push(level);
            }
        }
    };
    if c.map_pool.is_empty() {
        for level in levels() {
            add(Some(level));
        }
    }
    for map in &c.map_pool {
        add(find_level(map));
    }
    pool
}
// Maps that just came from Thunderstore (their assets) join a pool that names maps, or nobody
// could vote for them; taken out again later, they stay out. The names added.
pub fn pool_new_maps(c: &mut ServerConfig, assets: &[String]) -> Vec<String> {
    let mut added = Vec::new();
    if c.map_pool.is_empty() {
        return added;
    }
    for asset in assets {
        if let Some(level) = find_level(asset) {
            if !c.map_pool.iter().any(|m| m.eq_ignore_ascii_case(&level.name)) {
                c.map_pool.push(level.name.clone());
                added.push(level.name);
            }
        }
    }
    added
}

// Maps whose packages were removed (names and assets) leave the pool, and the server's map falls
// back to San Vansterdam, or the config would name maps the server cannot find and it could not
// start again. What changed, for the log.
pub fn drop_removed_maps(c: &mut ServerConfig, removed: &[String]) -> Vec<String> {
    let gone = |map: &str| removed.iter().any(|r| r.eq_ignore_ascii_case(map));
    let mut lines = Vec::new();
    let before = c.map_pool.len();
    c.map_pool.retain(|m| !gone(m));
    if c.map_pool.len() != before {
        lines.push("Map mods: removed maps were taken out of map_pool.".to_string());
    }
    if gone(&c.map) {
        lines.push(format!("Map mods: {} was removed, so the server's map is San Vansterdam again.", c.map));
        c.map = ServerConfig::default().map;
    }
    lines
}

pub fn in_map_pool(c: &ServerConfig, map: &str) -> bool {
    if c.map_pool.is_empty() {
        return true;
    }
    find_level(map).is_some_and(|level| pool_levels(c).iter().any(|p| same(&p.asset, &level.asset)))
}
// The pool map after `map` (the first one when `map` is not in the pool); None when there is
// no other.
pub fn next_pool_map(c: &ServerConfig, map: &str) -> Option<ServerLevel> {
    let pool = pool_levels(c);
    let current = find_level(map);
    let is_current = |level: &ServerLevel| current.as_ref().is_some_and(|c| same(&c.asset, &level.asset));
    let at = pool.iter().position(|l| is_current(l)).unwrap_or(pool.len());
    (1..=pool.len())
        .map(|step| if at == pool.len() { &pool[step - 1] } else { &pool[(at + step) % pool.len()] })
        .find(|l| !is_current(l))
        .cloned()
}
