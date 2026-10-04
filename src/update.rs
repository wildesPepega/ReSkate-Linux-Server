// Release checks and self-update (Server/server_update.cpp, Launcher/updater.cpp).
// Two sources: ReSkate's own latest release names the server version and the supported game
// build in its launcher.json; this port's releases carry the Linux build. A newer Linux build is
// downloaded, checked and staged in the background and installed once the server is empty
// (docs/installation.md#updates).
use crate::protocol::GAME_SHA256;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RELEASE_REPO: &str = "Dingo-Shenanigans/ReSkate";
pub const LINUX_REPO: &str = "wildesPepega/ReSkate-Linux-Server";
const LINUX_ASSET: &str = "ReSkateServer-linux-x64.tar.gz";
const PACKAGE_DIR: &str = "ReSkateServer-linux-x64";
pub const STAGING_DIR: &str = ".update";
// "1.0.8" or "1.0.8-1": the ReSkate version this build matches, then this port's revision.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
// What an update must never replace, though a release does not contain these anyway.
const KEEP: &[&str] = &["ReSkateServer.json", "ReSkateServer.log", "Mods", "plugins", "logs", STAGING_DIR];

pub struct UpdateCheck {
    // ReSkate itself
    pub available: bool,
    pub version: String,
    pub problem: String,
    // The release supports a different skate. build: players on it cannot join this server.
    pub new_game_build: bool,
    // This port: a newer Linux build, if there is one.
    pub linux: Option<LinuxRelease>,
    pub linux_problem: String,
}

#[derive(Clone)]
pub struct LinuxRelease {
    pub version: String,
    url: String,
    size: u64,
    sha256: Option<String>,
}

// (1.0.8, revision 1) from "v1.0.8-1"; None for anything else.
pub fn parse_version(text: &str) -> Option<(Vec<u32>, u32)> {
    let text = text.trim().trim_start_matches('v');
    let (base, revision) = match text.split_once('-') {
        Some((base, revision)) => (base, revision.parse().ok()?),
        None => (text, 0),
    };
    let parts: Option<Vec<u32>> = base.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty()).map(|p| (p, revision))
}

pub fn newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).redirects(5).user_agent(&format!("ReSkateServer/{VERSION} (Linux)")).build()
}

fn call(agent: &ureq::Agent, url: &str) -> Result<ureq::Response, String> {
    let mut request = agent.get(url).set("Cache-Control", "no-cache");
    let github = url.starts_with("https://api.github.com/");
    if github {
        request = request.set("X-GitHub-Api-Version", "2022-11-28").set("Accept", "application/vnd.github+json");
    }
    match request.call() {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(status, _)) if github && (status == 403 || status == 429) => {
            Err(format!("GitHub is limiting release checks from this network (HTTP {status}); try again later"))
        }
        Err(ureq::Error::Status(status, _)) => Err(format!("{url} returned HTTP {status}")),
        Err(_) => Err("GitHub could not be reached".into()),
    }
}

fn get_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    call(agent, url)?.into_reader().take(limit + 1).read_to_end(&mut bytes).map_err(|_| "GitHub could not be reached".to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("Download is larger than expected: {url}"));
    }
    Ok(bytes)
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<String, String> {
    String::from_utf8(get_bytes(agent, url, limit)?).map_err(|_| format!("{url} is not text"))
}

fn latest_release(agent: &ureq::Agent, repo: &str) -> Result<Value, String> {
    let latest = format!("https://api.github.com/repos/{repo}/releases/latest");
    serde_json::from_str(&get(agent, &latest, 4 * 1024 * 1024)?).map_err(|e| e.to_string())
}

fn asset<'a>(release: &'a Value, name: &str) -> Option<&'a Value> {
    release.get("assets")?.as_array()?.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(name))
}

fn fetch() -> Result<(String, String), String> {
    let agent = agent(Duration::from_secs(8));
    let release = latest_release(&agent, RELEASE_REPO)?;
    let manifest = asset(&release, "launcher.json")
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

// This port's latest release, when it is newer than this build.
fn fetch_linux() -> Result<Option<LinuxRelease>, String> {
    let agent = agent(Duration::from_secs(8));
    let release = latest_release(&agent, LINUX_REPO)?;
    let tag = release.get("tag_name").and_then(Value::as_str).unwrap_or("");
    if !newer(tag, VERSION) {
        return Ok(None);
    }
    let file = asset(&release, LINUX_ASSET).ok_or_else(|| format!("release {tag} has no {LINUX_ASSET}"))?;
    let url = file.get("browser_download_url").and_then(Value::as_str).ok_or("the release asset has no download link")?;
    let sha256 = file.get("digest").and_then(Value::as_str).and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase);
    Ok(Some(LinuxRelease {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
        size: file.get("size").and_then(Value::as_u64).unwrap_or(0),
        sha256,
    }))
}

// Asks GitHub for both latest releases; blocks for a few seconds at most.
pub fn check_for_update() -> UpdateCheck {
    let (linux, linux_problem) = match fetch_linux() {
        Ok(linux) => (linux, String::new()),
        Err(problem) => (None, problem),
    };
    match fetch() {
        Ok((version, game)) => UpdateCheck {
            available: newer(&version, VERSION.split('-').next().unwrap_or(VERSION)),
            new_game_build: !game.is_empty() && game != GAME_SHA256,
            version,
            problem: String::new(),
            linux,
            linux_problem,
        },
        Err(problem) => UpdateCheck { available: false, version: String::new(), problem, new_game_build: false, linux, linux_problem },
    }
}

// Downloads the release into <here>/.update, checks it and returns the unpacked package folder.
pub fn stage(release: &LinuxRelease, here: &Path) -> Result<PathBuf, String> {
    let agent = agent(Duration::from_secs(120));
    let limit = if release.size > 0 { release.size } else { 128 * 1024 * 1024 };
    let archive = get_bytes(&agent, &release.url, limit)?;
    if let Some(expected) = &release.sha256 {
        let actual: String = Sha256::digest(&archive).iter().map(|b| format!("{b:02x}")).collect();
        if &actual != expected {
            return Err("the download does not match its SHA-256 checksum".into());
        }
    }
    let staging = here.join(STAGING_DIR);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
    // unpack_in refuses entries that would land outside the staging folder.
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive.as_slice()));
    let entries = tar.entries().map_err(|e| format!("the download is not a valid archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("the download is not a valid archive: {e}"))?;
        entry.unpack_in(&staging).map_err(|e| format!("cannot unpack the update: {e}"))?;
    }
    let package = staging.join(PACKAGE_DIR);
    let binary = package.join("ReSkateServer");
    if !binary.is_file() || !package.join("libsteam_api.so").is_file() {
        return Err("the download does not contain the server".into());
    }
    check_runs(&binary)?;
    Ok(package)
}

// The new binary must at least start on this system (a build for a newer glibc would not).
pub(crate) fn check_runs(binary: &Path) -> Result<(), String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("the new server cannot be started: {e}"))?;
    let started = Instant::now();
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if started.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            return Err("the new server did not answer --version".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    // The server itself prints to stdout: its version, or (builds before --version) its usage.
    // The loader's errors go to stderr and name the binary too ("required by ./ReSkateServer").
    let printed = String::from_utf8_lossy(&output.stdout);
    if printed.starts_with("ReSkateServer") || printed.contains("ReSkateServer [") {
        return Ok(());
    }
    let errors = String::from_utf8_lossy(&output.stderr);
    let first = errors.lines().chain(printed.lines()).next().unwrap_or("no output");
    Err(format!("the new server does not run on this system: {first}"))
}

// Moves the staged package over the installed files. The running binary can be replaced on
// Linux; the new one runs after the restart.
pub fn install(package: &Path, here: &Path) -> Result<(), String> {
    fn copy_over(from: &Path, to: &Path, top: bool) -> Result<(), String> {
        for entry in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name();
            if top && KEEP.iter().any(|k| name.to_str() == Some(k)) {
                continue;
            }
            let source = entry.path();
            let target = to.join(&name);
            if source.is_dir() {
                std::fs::create_dir_all(&target).map_err(|e| format!("cannot create {}: {e}", target.display()))?;
                copy_over(&source, &target, false)?;
            } else {
                // rename is atomic and works on a running executable; copy across file systems.
                if std::fs::rename(&source, &target).is_err() {
                    let temporary = to.join(format!(".{}.new", name.to_string_lossy()));
                    std::fs::copy(&source, &temporary).map_err(|e| format!("cannot write {}: {e}", target.display()))?;
                    std::fs::rename(&temporary, &target).map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
                }
            }
        }
        Ok(())
    }
    copy_over(package, here, true)?;
    let _ = std::fs::remove_dir_all(here.join(STAGING_DIR));
    Ok(())
}
