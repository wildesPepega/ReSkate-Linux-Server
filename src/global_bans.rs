// The ReSkate backend's ban list (Server/global_bans.{h,cpp}, Extension/Multiplayer/
// developer_identity.h), which a dedicated server enforces beside its own: Host::tick turns those
// players away, also when they are already on. The same answer says whether the server browser
// shows only servers with a steam_token. Read at startup and every ten minutes (a minute
// after a failure) on a thread of its own; until the first answer, and while the backend cannot be
// reached, the bans already read hold. "global_bans": false leaves it unread.
use std::io::Read;
use std::time::Duration;

pub const LISTS_URL: &str = "https://api.reskate.dev/api/v1/steam-ids";
pub const BANNED_NOTICE: &str = "You are banned from ReSkate multiplayer.";
const MAX_BYTES: u64 = 256 * 1024;
// The categories the answer carries besides the bans. Not used here, but an answer whose lists
// are not lists of players is not the lists, and lifts no ban.
const CATEGORIES: [&str; 5] = ["dev", "staff", "homie", "content_creator", "centrix"];

// A player's SteamID64 is 76561197960265728 plus a 32-bit account number, and nobody has
// account 0. The API sends them as strings.
fn steam_id(entry: &serde_json::Value) -> Option<u64> {
    const BASE: u64 = 76561197960265728;
    let id: u64 = entry.as_str()?.parse().ok()?;
    (id > BASE && id <= BASE + 0xffff_ffff && id.to_string() == entry.as_str()?).then_some(id)
}

fn players(list: &serde_json::Value) -> Result<Vec<u64>, String> {
    let entries = list.as_array().ok_or("a list of players is not a list")?;
    let mut ids = entries.iter().map(steam_id).collect::<Option<Vec<u64>>>().ok_or("an entry is not a player's SteamID64")?;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

// The servers the answer lists (official ones by the ID their login token gives them, blocked
// ones by any game server ID). Not used here either, but checked the same way.
fn servers(list: &serde_json::Value, valid: fn(u64) -> bool) -> Result<(), String> {
    let entries = list.as_array().ok_or("a list of servers is not a list")?;
    let good = |entry: &serde_json::Value| {
        let text = entry.as_str()?;
        let id: u64 = text.parse().ok()?;
        (id.to_string() == text && valid(id)).then_some(())
    };
    entries.iter().map(good).collect::<Option<Vec<()>>>().map(|_| ()).ok_or_else(|| "an entry is not a server's SteamID64".into())
}

// Whether the server browser shows only servers signed in with a login token (steam_token), as
// the backend switches it on and off beside the lists ("server_tokens_required"). False when the
// answer says nothing.
pub fn parse_tokens_required(answer: &str) -> bool {
    let root: serde_json::Value = serde_json::from_str(answer).unwrap_or_default();
    root.get("server_tokens_required").and_then(serde_json::Value::as_bool).unwrap_or(false)
}

// The API's answer, {"categories":{"dev":["7656119..."],"homie":[],...},"banned":[]}: the banned
// players, sorted. An answer without them bans nobody; anything that is not the lists is an error.
pub fn parse_ban_list(answer: &str) -> Result<Vec<u64>, String> {
    let root: serde_json::Value = serde_json::from_str(answer).map_err(|_| "the answer is not JSON".to_string())?;
    let categories = root.get("categories").filter(|c| c.is_object()).ok_or("the answer has no categories")?;
    for name in CATEGORIES {
        if let Some(list) = categories.get(name) {
            players(list)?;
        }
    }
    if let Some(list) = root.get("official_servers") {
        servers(list, crate::protocol::persistent_server_steam_id)?;
    }
    if let Some(list) = root.get("blocked_servers") {
        servers(list, crate::protocol::game_server_steam_id)?;
    }
    match root.get("banned") {
        Some(list) => players(list),
        None => Ok(Vec::new()),
    }
}

// Asks the backend; blocks for 15 s at most, so it runs off the main loop. The bans, and
// whether the server browser wants a steam_token (parse_tokens_required).
pub type BanCheck = Result<(Vec<u64>, bool), String>;
pub fn read_ban_list() -> BanCheck {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("ReSkateServer/{} (Linux)", crate::update::VERSION))
        .build();
    let response = match agent.get(LISTS_URL).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(status, _)) => return Err(format!("api.reskate.dev answered HTTP {status}")),
        Err(e) => return Err(format!("api.reskate.dev could not be reached: {}", e.kind())),
    };
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("the answer was cut off: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("the answer is larger than a ban list can be".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "the answer is not text".to_string())?;
    let bans = parse_ban_list(&text).map_err(|e| format!("the answer was not the lists: {e}"))?;
    Ok((bans, parse_tokens_required(&text)))
}
