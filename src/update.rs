// Release checks (Server/server_update.cpp, Launcher/updater.cpp). The latest GitHub release's
// launcher.json names the server version and the supported game build. The Windows server
// installs its zip; this Linux build cannot run that zip, so it reports the new release and
// keeps running until it is replaced with a matching Linux build.
use crate::protocol::GAME_SHA256;
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

const RELEASE_REPO: &str = "Dingo-Shenanigans/ReSkate";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct UpdateCheck {
    pub available: bool,
    pub version: String,
    pub problem: String,
    // The release supports a different skate. build: players on it cannot join this server.
    pub new_game_build: bool,
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<String, String> {
    let mut request = agent.get(url).set("Cache-Control", "no-cache");
    let github = url.starts_with("https://api.github.com/");
    if github {
        request = request.set("X-GitHub-Api-Version", "2022-11-28").set("Accept", "application/vnd.github+json");
    }
    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(status, _)) if github && (status == 403 || status == 429) => {
            return Err(format!("GitHub is limiting release checks from this network (HTTP {status}); try again later"))
        }
        Err(ureq::Error::Status(status, _)) => return Err(format!("{url} returned HTTP {status}")),
        Err(_) => return Err("GitHub could not be reached".into()),
    };
    let mut text = String::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_string(&mut text)
        .map_err(|_| "GitHub could not be reached".to_string())?;
    if text.len() as u64 > limit {
        return Err(format!("Download is larger than expected: {url}"));
    }
    Ok(text)
}

fn fetch() -> Result<(String, String), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .redirects(5)
        .user_agent(&format!("ReSkateServer/{VERSION} (Linux)"))
        .build();
    let latest = format!("https://api.github.com/repos/{RELEASE_REPO}/releases/latest");
    let release: Value = serde_json::from_str(&get(&agent, &latest, 4 * 1024 * 1024)?).map_err(|e| e.to_string())?;
    let manifest = release
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|a| a.get("name").and_then(Value::as_str) == Some("launcher.json"))
        })
        .and_then(|a| a.get("browser_download_url").and_then(Value::as_str))
        .ok_or("The latest release has no launcher.json asset")?
        .to_string();
    let config: Value = serde_json::from_str(&get(&agent, &manifest, 64 * 1024)?).map_err(|e| e.to_string())?;
    let schema = config.get("schema").and_then(Value::as_i64).unwrap_or(0);
    if schema != 1 {
        return Err(format!("Launcher config schema {schema} is not supported"));
    }
    let server = config.get("server").filter(|s| !s.is_null()).ok_or("the latest release has no server")?;
    let version = server.get("version").and_then(Value::as_str).unwrap_or("").to_string();
    let game = config
        .get("game")
        .and_then(|g| g.get("skate_sha256"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok((version, game))
}

// Asks GitHub for the latest release; blocks for a few seconds at most.
pub fn check_for_update() -> UpdateCheck {
    match fetch() {
        Ok((version, game)) => UpdateCheck {
            available: !version.is_empty() && version != VERSION,
            new_game_build: !game.is_empty() && game != GAME_SHA256,
            version,
            problem: String::new(),
        },
        Err(problem) => UpdateCheck { available: false, version: String::new(), problem, new_game_build: false },
    }
}
