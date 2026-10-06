// ReSkate dedicated server for Linux: a headless session host that players find in the in-game
// server browser (a Rust port of Server/main.cpp). Runs from its own folder, next to
// libsteam_api.so; Steam's steamclient.so comes from SteamCMD (~/.steam/sdk64 or this folder).
//
// Console: commands are read line by line from stdin and every line of output is written to
// stdout as it happens, so hosting panels (Pterodactyl and the like) can drive it. SIGINT,
// SIGTERM and SIGHUP shut it down cleanly, signing out of Steam first.
mod activity;
mod buffers;
mod config;
mod discord;
mod global_bans;
mod host;
mod objects;
mod party;
mod plugins;
mod password;
mod protocol;
mod speed;
mod status;
mod steam;
mod text;
mod throwdown;
mod update;
mod wire;
mod words;
mod world;

#[cfg(test)]
mod tests;

use config::{config_error, load_config, load_levels, map_setting, save_config};
use host::Host;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use steam::{Advertisement, SteamServer, SteamTransport};

static STOPPING: AtomicBool = AtomicBool::new(false);
static LOG: Mutex<Option<LogFile>> = Mutex::new(None);
static HERE: OnceLock<PathBuf> = OnceLock::new();
// run()'s answer when the server installed an update and starts again in place.
const RESTART: i32 = -2;
// Days of rotated logs kept in logs/.
const LOG_DAYS: usize = 14;

// ReSkateServer.log holds today; each day before moves to logs/ReSkateServer-<date>.log.
struct LogFile {
    file: Option<File>,
    date: String,
    folder: PathBuf,
}

// Microseconds on the monotonic clock. Session timings share it.
pub fn now_us() -> u64 {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) };
    time.tv_sec as u64 * 1_000_000 + time.tv_nsec as u64 / 1000
}

fn local_time(seconds: libc::time_t) -> (String, String) {
    unsafe {
        let mut local: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut local);
        let date = format!("{:04}-{:02}-{:02}", local.tm_year + 1900, local.tm_mon + 1, local.tm_mday);
        let time = format!("{:02}:{:02}:{:02}", local.tm_hour, local.tm_min, local.tm_sec);
        (date, time)
    }
}

fn stamp() -> (String, String) {
    local_time(unsafe { libc::time(std::ptr::null_mut()) })
}

impl LogFile {
    fn current(&self) -> PathBuf {
        self.folder.join("ReSkateServer.log")
    }
    fn open(&mut self) {
        self.file = OpenOptions::new().create(true).append(true).open(self.current()).ok();
    }
    // Moves ReSkateServer.log (holding `date`) to logs/ and drops the oldest beyond LOG_DAYS.
    fn rotate(&mut self, date: &str) {
        self.file = None;
        let logs = self.folder.join("logs");
        if std::fs::create_dir_all(&logs).is_ok() {
            let target = logs.join(format!("ReSkateServer-{date}.log"));
            if target.exists() {
                // A second rotation the same day (a restart after midnight): add to it.
                if let (Ok(mut old), Ok(mut out)) = (File::open(self.current()), OpenOptions::new().append(true).open(&target)) {
                    if std::io::copy(&mut old, &mut out).is_ok() {
                        let _ = std::fs::remove_file(self.current());
                    }
                }
            } else {
                let _ = std::fs::rename(self.current(), &target);
            }
            if let Ok(entries) = std::fs::read_dir(&logs) {
                let mut old: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("ReSkateServer-") && n.ends_with(".log")))
                    .collect();
                old.sort();
                while old.len() > LOG_DAYS {
                    let _ = std::fs::remove_file(old.remove(0));
                }
            }
        }
        self.open();
    }
}

// Opens the log; one left from an earlier day is rotated first.
fn open_log(folder: &Path) {
    let mut log = LogFile { file: None, date: stamp().0, folder: folder.to_path_buf() };
    let modified = std::fs::metadata(log.current()).and_then(|m| m.modified()).ok();
    if let Some(seconds) = modified.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()) {
        let (date, _) = local_time(seconds.as_secs() as libc::time_t);
        if date != log.date {
            log.rotate(&date);
        }
    }
    log.open();
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(log);
}

fn write_log(text: &str) {
    let time = write_local(text);
    discord::post(text, &time);
}

// The console and the log file; returns the time stamp used.
fn write_local(text: &str) -> String {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let (date, time) = stamp();
    // One write per line, flushed at once, so a hosting panel reading the pipe sees it now.
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "[{time}] {text}");
    let _ = out.flush();
    if let Some(log) = log.as_mut() {
        if log.date != date {
            let previous = std::mem::replace(&mut log.date, date.clone());
            log.rotate(&previous);
        }
        if let Some(file) = log.file.as_mut() {
            let _ = writeln!(file, "[{date} {time}] {text}");
            let _ = file.flush();
        }
    }
    time
}

// How long the main loop sleeps between ticks, with players and on an empty server.
const LOOP_MS: u64 = 2;
const IDLE_LOOP_MS: u64 = 20;

extern "C" fn on_signal(_: libc::c_int) {
    STOPPING.store(true, Ordering::SeqCst);
}

// The server's folder, read once: after an update replaced the binary, /proc/self/exe no longer
// resolves.
fn folder() -> PathBuf {
    HERE.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.canonicalize().ok())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    })
    .clone()
}

// Steam's client library looks in ~/.steam/sdk64. A steamclient.so put next to the server is
// linked there when nothing is there yet.
fn link_steam_client(here: &Path) {
    let client = here.join("steamclient.so");
    let Some(home) = std::env::var_os("HOME") else { return };
    let sdk = PathBuf::from(home).join(".steam").join("sdk64");
    let target = sdk.join("steamclient.so");
    if !client.exists() || target.exists() {
        return;
    }
    if std::fs::create_dir_all(&sdk).is_ok() {
        let _ = std::os::unix::fs::symlink(&client, &target);
    }
}

// Console lines, read on their own thread so the network loop never waits for typing.
fn console_input() -> Receiver<String> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => return, // no console (EOF): the server keeps running
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buffer);
                    let line = line.trim_end_matches(['\n', '\r']).to_string();
                    if sender.send(line).is_err() {
                        return;
                    }
                }
            }
        }
    });
    receiver
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = payload.downcast_ref::<&str>() {
        text.to_string()
    } else {
        "unknown error".into()
    }
}

fn update_message(check: &update::UpdateCheck) -> String {
    let mut text = format!(
        "ReSkate {} is out; this Linux server is {}. It updates itself once a Linux build of it is released (github.com/{}).",
        check.version,
        update::VERSION,
        update::LINUX_REPO
    );
    if check.new_game_build {
        text += " The new release supports a newer skate. build: players on it cannot join this server until then.";
    }
    text
}

struct Options {
    config: Option<PathBuf>,
    no_update: bool,
    port: Option<u16>,
    query_port: Option<u16>,
}

fn usage() -> &'static str {
    "ReSkateServer [--config <file>] [--port <port>] [--query-port <port>] [--no-update] [--version]\n\
     Commands are read from the console (stdin); type help once it runs."
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options { config: None, no_update: false, port: None, query_port: None };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value.", args[i]));
        let port = |text: String, name: &str| text.parse::<u16>().ok().filter(|&p| p != 0).ok_or_else(|| format!("{name} must be 1 to 65535."));
        match args[i].as_str() {
            "--config" => {
                options.config = Some(PathBuf::from(value(i)?));
                i += 1;
            }
            "--port" => {
                options.port = Some(port(value(i)?, "--port")?);
                i += 1;
            }
            "--query-port" | "--query_port" => {
                options.query_port = Some(port(value(i)?, "--query-port")?);
                i += 1;
            }
            "--no-update" => options.no_update = true,
            "--export-world-layers" => {
                return Err("--export-world-layers reads the game's own files and runs on Windows only; copy \
                            world-layers.json from a player's %LOCALAPPDATA%\\ReSkate\\cache folder instead."
                    .into())
            }
            "--help" | "-h" => return Err(usage().into()),
            other => return Err(format!("Unknown option {other}.\n{}", usage())),
        }
        i += 1;
    }
    Ok(options)
}

// Starts the new binary in this process: same PID, console and arguments, so a hosting panel
// sees the server keep running.
fn restart() -> i32 {
    use std::os::unix::process::CommandExt;
    let error = std::process::Command::new(folder().join("ReSkateServer")).args(std::env::args_os().skip(1)).exec();
    write_log(&format!("Could not restart after the update: {error}. Start the server again."));
    1
}

fn stage_in_background(release: update::LinuxRelease, here: PathBuf) -> Receiver<(update::LinuxRelease, Result<PathBuf, String>)> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let staged = update::stage(&release, &here);
        let _ = sender.send((release, staged));
    });
    receiver
}

fn run() -> i32 {
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("ReSkateServer {}", update::VERSION);
        return 0;
    }
    let options = match parse_options() {
        Ok(options) => options,
        Err(text) => {
            println!("{text}");
            return 1;
        }
    };
    let here = folder();
    let config_file = options.config.clone().unwrap_or_else(|| here.join("ReSkateServer.json"));
    open_log(&here);
    write_log(&format!("ReSkate Linux Server {}", update::VERSION));
    let _ = std::fs::remove_dir_all(here.join(update::STAGING_DIR)); // left by an interrupted update

    let fresh = !config_file.exists();
    let mut added = Vec::new();
    let mut config = match load_config(&config_file, &mut added) {
        Ok(config) => config,
        Err(e) => {
            write_log(&format!("Cannot read {}: {e}", config_file.display()));
            return 1;
        }
    };
    let file_name = config_file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if fresh {
        write_log(&format!("Wrote a default {file_name}. Edit it to name the server and add admins."));
    }
    if !added.is_empty() {
        write_log(&format!("Added new settings to {file_name} with their defaults: {}.", added.join(", ")));
    }
    // Maps: the retail ones and custom maps from Mods/<mod>/reskate-levels.json.
    for problem in load_levels(&here.join("Mods")) {
        write_log(&format!("Mods: skipped {problem}"));
    }
    let level_count = config::levels().len();
    if level_count > 6 {
        write_log(&format!("Mods: {} custom map(s).", level_count - 6));
    }
    // Older configs name the map by its full destination; keep the plain name instead.
    let mut renamed = false;
    let setting = map_setting(&config.map);
    if setting != config.map && !setting.is_empty() {
        config.map = setting;
        renamed = true;
    }
    // Short pool names ("isle") are saved in full.
    for map in &mut config.map_pool {
        if let Some(level) = config::find_level(map) {
            if level.name != *map {
                *map = level.name;
                renamed = true;
            }
        }
    }
    if renamed {
        let _ = save_config(&config);
    }
    // A hosting panel hands out the ports; they win over the file.
    if let Some(port) = options.port {
        config.port = port;
    }
    if let Some(port) = options.query_port {
        config.query_port = port;
    }
    let error = config_error(&config);
    if !error.is_empty() {
        write_log(&format!("Config problem: {error}"));
        return 1;
    }
    if !config.dropped.is_empty() {
        write_log(&format!(
            "Removed {} from {file_name}: the ports come from --port and --query-port (default 27015 and 27016).",
            config.dropped.join(", ")
        ));
    }
    if !config.discord_webhook.is_empty() {
        match discord::start(&config.discord_webhook, &config.discord_events, &config.discord_style, &config.name, write_log) {
            Ok(summary) => write_log(&summary),
            Err(e) => write_log(&format!("Discord is off: {e}.")),
        }
    }
    // Release checks: at startup, then every half hour. --no-update or "auto_update": false turns them off.
    let auto_update = config.auto_update && !options.no_update;
    let mut announced_version = String::new();
    if auto_update {
        write_log("Checking for updates...");
        let check = update::check_for_update();
        if let Some(release) = &check.linux {
            // Nobody is on yet: install before signing in to Steam.
            write_log(&format!("ReSkate Linux Server {} is available (this is {}); installing it.", release.version, update::VERSION));
            match update::stage(release, &here).and_then(|package| update::install(&package, &here)) {
                Ok(()) => {
                    write_log(&format!("Installed {}; restarting.", release.version));
                    discord::flush(Duration::from_secs(5));
                    return RESTART;
                }
                Err(e) => write_log(&format!("The update failed: {e}. Starting {} instead.", update::VERSION)),
            }
        } else if check.available {
            write_log(&update_message(&check));
            announced_version = check.version.clone();
        } else if check.problem.is_empty() && check.linux_problem.is_empty() {
            write_log(&format!("The server is up to date ({}).", update::VERSION));
        }
        if !check.problem.is_empty() || !check.linux_problem.is_empty() {
            let problem = if check.linux_problem.is_empty() { &check.problem } else { &check.linux_problem };
            write_log(&format!("Update check skipped: {problem}."));
        }
    }
    // World layers are optional: without the players' catalog every player keeps their own.
    let catalog = here.join("world-layers.json");
    if catalog.exists() {
        match world::read_world_layers(&catalog) {
            Ok(catalog) => {
                world::install_world_layer_catalog(catalog);
                write_log(&format!("World layers: {} from world-layers.json.", world::world_layers().len()));
            }
            Err(e) => write_log(&format!("world-layers.json is unreadable; world layer sync is off: {e}")),
        }
    }

    link_steam_client(&here);
    let mut steam = match SteamServer::start(&here, config.port, config.query_port) {
        Ok(steam) => steam,
        Err(e) => {
            write_log(&e);
            return 1;
        }
    };
    write_log("Signing in to Steam...");
    let login_started = Instant::now();
    while !steam.logged_on() && !STOPPING.load(Ordering::SeqCst) {
        steam.run_callbacks();
        if login_started.elapsed() > Duration::from_secs(60) {
            write_log("Steam sign-in timed out after 60 s. Check the internet connection and try again.");
            return 1;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if STOPPING.load(Ordering::SeqCst) {
        write_log("Shutting down.");
        return 0;
    }

    let mut transport = SteamTransport::new();
    if !transport.open_game_server(steam.library()) {
        write_log(&format!("Steam networking failed: {}", transport.detail));
        return 1;
    }
    let mut host = Host::new(config, transport, Box::new(write_log));
    if let Err(e) = catch_unwind(AssertUnwindSafe(|| host.start())).unwrap_or_else(|p| Err(panic_text(p))) {
        write_log(&format!("Could not open the server: {e}"));
        return 1;
    }
    host.load_plugins(&here.join("plugins"));
    write_log(&format!("{} is up on {} for {} players.", host.config.name, host.map_name(), host.config.max_players));
    write_log(&format!("Steam ID {}, public IP {}.", steam.steam_id(), steam.public_ip()));
    write_log(&format!(
        "Join code: {}{}",
        host.invite(),
        if host.config.password.is_empty() { "" } else { " (password required)" }
    ));
    write_log(&if host.config.admins.is_empty() {
        "No admins yet: type \"admin add <SteamID64>\" to add one.".to_string()
    } else {
        format!("{} admin(s). Type help for commands.", host.config.admins.len())
    });

    let status = if !host.config.status_enabled {
        None
    } else {
        match status::StatusServer::start(host.config.port) {
            Ok(server) => {
                write_log(&format!("Status: http://{}:{}/status (TCP).", steam.public_ip(), host.config.port));
                Some(server)
            }
            Err(e) => {
                write_log(&format!("Status page is off: TCP port {} cannot be opened ({e}).", host.config.port));
                None
            }
        }
    };
    let mut next_status = Instant::now();
    // The backend's ban list (src/global_bans.rs): read now and every ten minutes, a minute after
    // a failure. Said when it changes, not every ten minutes.
    let mut ban_check: Option<Receiver<Result<Vec<u64>, String>>> = None;
    let mut next_ban_check = Instant::now();
    let mut last_bans: Option<Vec<u64>> = None;
    let mut bans_unread = false;
    if !host.config.global_bans {
        write_log("Global bans are off (\"global_bans\": false): only this server's own bans apply.");
    }

    let input = console_input();
    let mut next_advertise = Instant::now();
    let mut name_allowed: Option<bool> = None;
    let update_interval = Duration::from_secs(30 * 60);
    let mut next_update_check = Instant::now() + update_interval;
    let mut update_check: Option<Receiver<update::UpdateCheck>> = None;
    let mut update_now = false;
    // A newer Linux build: being downloaded, or downloaded and waiting for an empty server.
    let mut staging: Option<Receiver<(update::LinuxRelease, Result<PathBuf, String>)>> = None;
    let mut staged: Option<(update::LinuxRelease, PathBuf)> = None;
    let mut install_now = false;
    let mut restarting = false;
    // An empty server looks for an update soon after the last player left, then every few minutes.
    let mut empty_since: Option<Instant> = None;
    let mut quick_check: Option<Receiver<Result<bool, String>>> = None;
    let mut last_quick_check = Instant::now();
    let empty_delay = Duration::from_secs(10);
    let empty_interval = Duration::from_secs(5 * 60);
    let quick_spacing = Duration::from_secs(60);
    let start_check = || {
        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            let _ = sender.send(update::check_for_update());
        });
        receiver
    };
    while !STOPPING.load(Ordering::SeqCst) {
        steam.run_callbacks();
        if let Err(p) = catch_unwind(AssertUnwindSafe(|| host.tick(now_us()))) {
            write_log(&format!("Server error: {}", panic_text(p)));
        }
        loop {
            let line = match input.try_recv() {
                Ok(line) => line,
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            };
            let line = text::trim(&line).to_string();
            if line == "quit" || line == "exit" || line == "stop" {
                STOPPING.store(true, Ordering::SeqCst);
                break;
            }
            if line.is_empty() {
                continue;
            }
            // "update": check now and install straight away (players are told to rejoin).
            if line == "update" {
                update_now = true;
                install_now = true;
                if staged.is_none() && staging.is_none() && update_check.is_none() {
                    update_check = Some(start_check());
                    write_log("Checking for updates...");
                }
                continue;
            }
            if line == "discord" || line == "discord test" {
                if line == "discord test" && !discord::test(&format!("Test message from {}.", host.config.name)) {
                    write_log("Discord is off: set discord.webhook in the config and restart.");
                } else {
                    write_log(&discord::status());
                }
                continue;
            }
            match catch_unwind(AssertUnwindSafe(|| host.command(&line, 0))) {
                Ok(answer) => write_log(&answer),
                Err(p) => write_log(&format!("Command failed: {}", panic_text(p))),
            }
        }
        let now = Instant::now();
        if auto_update && update_check.is_none() && now >= next_update_check {
            update_check = Some(start_check());
        }
        if let Some(receiver) = &update_check {
            match receiver.try_recv() {
                Ok(check) => {
                    next_update_check = now + update_interval;
                    if let Some(release) = check.linux.clone() {
                        if staging.is_none() && !matches!(&staged, Some((r, _)) if r.version == release.version) {
                            write_log(&format!("ReSkate Linux Server {} is available (this is {}); downloading it.", release.version, update::VERSION));
                            staging = Some(stage_in_background(release, here.clone()));
                        }
                    } else if check.available {
                        if update_now || check.version != announced_version {
                            write_log(&update_message(&check));
                            announced_version = check.version.clone();
                        }
                    } else if update_now {
                        let problem = if check.linux_problem.is_empty() { &check.problem } else { &check.linux_problem };
                        write_log(&if problem.is_empty() {
                            format!("The server is up to date ({}).", update::VERSION)
                        } else {
                            format!("Update check failed: {problem}.")
                        });
                    }
                    if check.linux.is_none() {
                        install_now = false;
                    }
                    update_now = false;
                    update_check = None;
                }
                Err(TryRecvError::Disconnected) => update_check = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if auto_update {
            if host.connected() > 0 {
                empty_since = None;
            } else {
                let since = *empty_since.get_or_insert(now);
                // Ten seconds empty (not a rejoin or a map change), a minute since the last look,
                // and then every five minutes while it stays empty.
                let due = now.duration_since(since) >= empty_delay
                    && now.duration_since(last_quick_check) >= quick_spacing
                    && (now.duration_since(since) < empty_delay + quick_spacing || now.duration_since(last_quick_check) >= empty_interval);
                if due && quick_check.is_none() && update_check.is_none() && staging.is_none() && staged.is_none() {
                    last_quick_check = now;
                    let (sender, receiver) = channel();
                    std::thread::spawn(move || {
                        let _ = sender.send(update::quick_check());
                    });
                    quick_check = Some(receiver);
                }
            }
        }
        if let Some(receiver) = &quick_check {
            match receiver.try_recv() {
                // A newer release: the full check reads its checksum, then it is staged and
                // installed below, since the server is empty.
                Ok(Ok(true)) => {
                    if update_check.is_none() {
                        update_check = Some(start_check());
                    }
                    quick_check = None;
                }
                Ok(_) | Err(TryRecvError::Disconnected) => quick_check = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = &staging {
            match receiver.try_recv() {
                Ok((release, Ok(package))) => {
                    if host.connected() > 0 && !install_now {
                        write_log(&format!("ReSkate Linux Server {} is ready; it installs when the server is empty.", release.version));
                    }
                    staged = Some((release, package));
                    staging = None;
                }
                Ok((release, Err(e))) => {
                    write_log(&format!("Could not download ReSkate Linux Server {}: {e}. Trying again at the next check.", release.version));
                    install_now = false;
                    staging = None;
                }
                Err(TryRecvError::Disconnected) => staging = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some((release, package)) = &staged {
            if host.connected() == 0 || install_now {
                match update::install(package, &here) {
                    Ok(()) => {
                        write_log(&format!("Installed ReSkate Linux Server {}; restarting.", release.version));
                        restarting = true;
                        STOPPING.store(true, Ordering::SeqCst);
                    }
                    Err(e) => write_log(&format!("Could not install ReSkate Linux Server {}: {e}.", release.version)),
                }
                staged = None;
                install_now = false;
            }
        }
        if host.config.global_bans && ban_check.is_none() && now >= next_ban_check {
            let (sender, receiver) = channel();
            std::thread::spawn(move || {
                let _ = sender.send(global_bans::read_ban_list());
            });
            ban_check = Some(receiver);
        }
        if let Some(receiver) = &ban_check {
            match receiver.try_recv() {
                Ok(Ok(ids)) => {
                    next_ban_check = now + Duration::from_secs(10 * 60);
                    if last_bans.as_ref() != Some(&ids) || bans_unread {
                        write_log(&format!("Global bans: {} player(s) banned from ReSkate multiplayer cannot join.", ids.len()));
                    }
                    last_bans = Some(ids.clone());
                    host.set_global_bans(ids);
                    bans_unread = false;
                    ban_check = None;
                }
                Ok(Err(problem)) => {
                    next_ban_check = now + Duration::from_secs(60);
                    if !bans_unread {
                        write_log(&format!(
                            "The global ban list could not be read ({problem}). Trying again every minute; until then the bans already read hold."
                        ));
                    }
                    bans_unread = true;
                    ban_check = None;
                }
                Err(TryRecvError::Disconnected) => {
                    next_ban_check = now + Duration::from_secs(60);
                    ban_check = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(server) = &status {
            if now >= next_status {
                next_status = now + Duration::from_secs(1);
                server.set(host.status().to_string());
            }
        }
        if now >= next_advertise {
            next_advertise = now + Duration::from_secs(2);
            discord::set_name(&host.config.name);
            // A name with a bad word in it is never listed (clients hide one too); the server
            // still runs and players can join with its code.
            let allowed = !words::contains_bad_words(&host.config.name);
            if name_allowed != Some(allowed) {
                if !allowed {
                    write_log(&format!(
                        "The server name \"{}\" contains blocked words, so the server is not listed in the server browser. Rename it with: name <new name>",
                        host.config.name
                    ));
                } else if name_allowed.is_some() {
                    write_log(if host.config.listed {
                        "The server name is allowed again; the server is listed."
                    } else {
                        "The server name is allowed again (the server is still set to unlisted)."
                    });
                }
                name_allowed = Some(allowed);
            }
            steam.advertise(&Advertisement {
                name: host.config.name.clone(),
                map: host.map_name(),
                players: host.players(),
                max_players: host.config.max_players,
                password: !host.config.password.is_empty(),
                listed: host.config.listed && allowed,
                secret: host.secret(),
            });
        }
        // With nobody connected nothing needs a fast loop: a new connection is noticed within
        // 20 ms and the loop is back at full speed from the next tick on.
        std::thread::sleep(Duration::from_millis(if host.connected() == 0 { IDLE_LOOP_MS } else { LOOP_MS }));
    }
    write_log(if restarting { "Restarting for the update." } else { "Shutting down." });
    let reason = if restarting { "The server is restarting for an update. Join again in a minute." } else { "The server is shutting down." };
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| host.stop(reason))) {
        write_log(&format!("Server error: {}", panic_text(p)));
    }
    // The networking closes before Steam itself shuts down.
    drop(host);
    steam.stop();
    discord::flush(Duration::from_secs(5));
    if restarting {
        RESTART
    } else {
        0
    }
}

fn main() {
    // C++ exceptions became panics; the loop reports them as the C++ server reports exceptions.
    std::panic::set_hook(Box::new(|_| {}));
    unsafe {
        libc::signal(libc::SIGINT, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    folder(); // before an update can replace the binary
    let mut code = run();
    if code == RESTART {
        code = restart();
    }
    std::process::exit(code);
}
