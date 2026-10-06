// The host side of a ReSkate session without a game (Server/server_host.{h,cpp},
// server_votes.cpp, server_party.cpp): admission, the roster, map changes and relaying every
// player's poses, outfits, voice, chat and objects. It follows the protocol the in-game host
// speaks, minus the host's own skater.
use crate::activity::{Activity, ActivityLog};
use crate::buffers::{AppearanceBuffer, AudioBuffer, PoseBuffer};
use crate::config::{find_level, in_map_pool, levels, map_destination, map_label, map_setting, next_pool_map, parse_scoring, pool_levels};
use crate::config::{save_config, scoring_text};
use crate::config::{ServerConfig, VoteSetting};
use crate::objects::{ObjectResult, ObjectState};
use crate::party::{PartyBook, PartyResult};
use crate::password::{password_key, password_proof, proof_matches, PasswordKey};
use crate::plugins::{Action, PlayerInfo, PluginManager, Snapshot};
use crate::protocol::*;
use crate::speed::{SpeedCheck, LIMIT as SPEED_LIMIT};
use crate::steam::{SteamTransport, TransportMessage, TransportPeer};
use crate::text::{decimal, dm_line, integer, lower, number, on_off, prefix, split, trim};
use crate::wire::{decode_wire, encode_wire, encode_wire_bytes, DeltaReceiver, DeltaSender};
use crate::words::{contains_bad_words, mask_bad_words};
use crate::world::{default_world_layers, pack_world_layers, park_label, valid_park, valid_world_layer_mode, world_layers, PARK_LOTS};
use std::collections::{BTreeMap, BTreeSet};

const HELP_TEXT: &str = "status | players | say <text> | msg <player> <text> | msg-party <player> <text> | msg-admins <text> | kick <player> | ban <player or SteamID64> [name] | unban <SteamID64> | bans\n\
map <name, e.g. San Vansterdam> | maps | name <text> | password <text|off> | welcome <text|off> | listed on|off\n\
tps 20|30|60|120 | voice on|off | voice-range <50-1000> | distances <full> <half> <half-return> <low>\n\
placement everyone|admins|nobody | clear-objects | noclip on|off | nobail on|off | boosts on|off | tuning on|off\n\
tpall [player] | tphere <player> | votes [map|kick|tod on|off|<percent>] | vote-cancel\n\
map-pool [add|remove <map>|clear] | rotation [<minutes>|off]\n\
park <lot> <layout> | layer-sync on|off | layer <key> default|on|off | tod <time|default>\n\
activity-log on|off | announce-throwdowns on|off | parties [on|off] | party-size <2-8> | speed-check off|warn|kick\n\
score-check [off|warn|kick] | score-allow [<fingerprint>|remove <fingerprint>]\n\
admin add|remove <SteamID64> | admins | plugins [reload] | discord [test] | version | update | quit";

// How long a connected player's game may send nothing before it is dropped. The C++ server uses
// 10 s, but games freeze for longer while loading what a new throwdown drop needs (Spot Battles
// especially), and those players were dropped. Steam ends dead connections on its own.
const GAMEPLAY_TIMEOUT_US: u64 = 30_000_000;

const TIMES: [&str; 8] = ["default", "morning", "noon", "afternoon", "evening", "night", "weatherday", "weathernight"];

fn nonce() -> u64 {
    let mut value = 0u64;
    let filled = unsafe { libc::getrandom(&mut value as *mut u64 as *mut libc::c_void, 8, 0) };
    if filled != 8 || value == 0 {
        panic!("Cannot generate a session code.");
    }
    value
}

// A command as the log shows it: the log is plain text, so a new password is left out.
fn loggable(command: &str) -> String {
    let (verb, argument) = split(command);
    if lower(verb) != "password" || argument.is_empty() || argument == "off" {
        return command.to_string();
    }
    format!("{verb} <hidden>")
}

#[derive(Clone, Copy, Default)]
struct ChatBudget {
    since: u64,
    messages: u32,
}
impl ChatBudget {
    // A few messages at once, then they recover every 5 s.
    fn accept(&mut self, now: u64, burst: u32) -> bool {
        if now < self.since || now - self.since >= 5_000_000 {
            self.since = now;
            self.messages = 0;
        }
        self.messages += 1;
        self.messages <= burst
    }
}

#[derive(Default)]
struct ObjectDelivery {
    sent: BTreeMap<u64, (u64, u64)>,
    chunks: Vec<ObjectChunk>,
    source: u64,
    epoch: u64,
    next: usize,
    cursor: usize,
}

struct PendingCosmetics {
    packet: Packet,
    received: u64,
}

struct Guest {
    member: Member,
    password_challenge: u64,
    handshaken: bool,
    map_authorized: bool,
    world_ready: bool,
    last_map_offer: u64,
    travel_since: u64,
    connected_at: u64,
    last_packet: u64,
    loading_since: u64,
    ready_sequence: u32,
    budget: ReceiveBudget,
    sender: DeltaSender,
    receiver: DeltaReceiver,
    poses: PoseBuffer,
    audio: AudioBuffer,
    appearance: AppearanceBuffer,
    cosmetic_packet: Vec<u8>,
    pending_cosmetics: Vec<PendingCosmetics>,
    latest_root: Option<Transform>,
    pose_arrival: u64,
    pose_delivery: Vec<PoseDelivery>,
    direct_routes: Vec<Member>,
    route_reported: u64,
    route_sequence: u32,
    received_voice: bool,
    voice_sequence: u32,
    voice_budget: VoiceBudget,
    outfit_budget: OutfitBudget,
    sound_budget: SoundBudget,
    chat_rate: ChatRate,
    admin_budget: ChatBudget,
    throwdown_budget: ChatBudget,
    party_budget: ChatBudget,
    speed: SpeedCheck,
    speeding: bool,
    speed_normal_since: u64,
    scoring: Option<u64>,
    scoring_mods: String,
    scoring_flagged: bool,
    scoring_budget: ChatBudget,
    bans_sent: u64,
    maps_sent: bool,
    objects: ObjectState,
    shared: ObjectState,
    shared_from: u64,
    cleared: BTreeSet<u64>,
    object_delivery: ObjectDelivery,
}

impl Default for Guest {
    fn default() -> Self {
        Guest {
            member: Member::default(),
            password_challenge: 0,
            handshaken: false,
            map_authorized: false,
            world_ready: true,
            last_map_offer: 0,
            travel_since: 0,
            connected_at: 0,
            last_packet: 0,
            loading_since: 0,
            ready_sequence: 0,
            budget: ReceiveBudget::default(),
            sender: DeltaSender::default(),
            receiver: DeltaReceiver::default(),
            poses: PoseBuffer::default(),
            audio: AudioBuffer::default(),
            appearance: AppearanceBuffer::default(),
            cosmetic_packet: Vec::new(),
            pending_cosmetics: Vec::new(),
            latest_root: None,
            pose_arrival: 0,
            pose_delivery: vec![PoseDelivery::default(); MAX_PLAYERS],
            direct_routes: Vec::new(),
            route_reported: 0,
            route_sequence: 0,
            received_voice: false,
            voice_sequence: 0,
            voice_budget: VoiceBudget::default(),
            outfit_budget: OutfitBudget::default(),
            sound_budget: SoundBudget::default(),
            chat_rate: ChatRate::default(),
            admin_budget: ChatBudget::default(),
            throwdown_budget: ChatBudget::default(),
            party_budget: ChatBudget::default(),
            speed: SpeedCheck::default(),
            speeding: false,
            speed_normal_since: 0,
            scoring: None,
            scoring_mods: String::new(),
            scoring_flagged: false,
            scoring_budget: ChatBudget::default(),
            bans_sent: 0,
            maps_sent: false,
            objects: ObjectState::default(),
            shared: ObjectState::default(),
            shared_from: 0,
            cleared: BTreeSet::new(),
            object_delivery: ObjectDelivery::default(),
        }
    }
}

fn guest_name(g: &Guest) -> String {
    if g.member.name.is_empty() {
        g.member.id.to_string()
    } else {
        g.member.name.clone()
    }
}

// A connected player's name for the activity log, or empty.
fn current_name(guests: &BTreeMap<u64, Box<Guest>>, id: u64) -> String {
    guests.get(&id).filter(|g| g.handshaken).map(|g| guest_name(g)).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VoteKind {
    Map,
    Kick,
    Time,
}
fn vote_name(kind: VoteKind) -> &'static str {
    match kind {
        VoteKind::Map => "Map",
        VoteKind::Kick => "Kick",
        VoteKind::Time => "Time of day",
    }
}

struct Vote {
    kind: VoteKind,
    target: u64,
    value: String,
    label: String,
    yes: BTreeSet<u64>,
    no: BTreeSet<u64>,
    ends: u64,
    shown_yes: u32,
    shown_no: u32,
}

pub type Log = Box<dyn Fn(&str)>;

pub struct Host {
    pub config: ServerConfig,
    pub transport: SteamTransport,
    log: Log,
    activity: ActivityLog,
    vote: Option<Vote>,
    vote_recount: bool,
    vote_cooldowns: BTreeMap<u64, u64>,
    parties: PartyBook,
    party_revision: u64,
    guests: BTreeMap<u64, Box<Guest>>,
    spare_ids: Vec<u64>, // reused by guest_ids(): the loops below run hundreds of times a second
    spare_links: Vec<TransportPeer>,
    spare_messages: Vec<TransportMessage>,
    kicked: BTreeSet<u64>,
    global_bans: Vec<u64>, // the ReSkate backend's ban list, sorted (src/global_bans.rs)
    join_backoff: JoinBackoff, // Steam IDs whose attempts to join keep failing
    password: Option<PasswordKey>,
    id: u64,
    secret: u64,
    epoch: u64,
    map: u64,
    world: u64,
    sequence: u32,
    voice_policy: VoicePolicy,
    object_clears: u32,
    layers: Vec<String>,
    roster_dirty: bool,
    running: bool,
    started: u64, // when start() opened the server (crate::now_us())
    map_since: u64,        // rotation clock start: the last map change, or while nobody is on
    rotation_warned: bool, // players were told the next map is a minute away
    // Set while plugins handle a map change: a map they change to from there gets no event of
    // its own, so two plugins (or one) cannot send the server from map to map without end.
    in_map_event: bool,
    bans_revision: u64,
    now: u64,
    last_roster: u64,
    last_world_state: u64,
    next_object_update: u64,
    travel_started: u64,
    plugins: PluginManager,
}

macro_rules! guest {
    ($self:ident, $id:expr) => {
        $self.guests.get_mut(&$id).unwrap()
    };
}

impl Host {
    pub fn new(config: ServerConfig, transport: SteamTransport, log: Log) -> Host {
        let parties = PartyBook::new(config.party_size as usize);
        Host {
            config,
            transport,
            log,
            activity: ActivityLog::default(),
            vote: None,
            vote_recount: false,
            vote_cooldowns: BTreeMap::new(),
            parties,
            party_revision: 0,
            guests: BTreeMap::new(),
            spare_ids: Vec::new(),
            spare_links: Vec::new(),
            spare_messages: Vec::new(),
            kicked: BTreeSet::new(),
            global_bans: Vec::new(),
            join_backoff: JoinBackoff::default(),
            password: None,
            id: 0,
            secret: 0,
            epoch: 0,
            map: 0,
            world: 1,
            sequence: 0,
            voice_policy: VoicePolicy::default(),
            object_clears: 0,
            layers: Vec::new(),
            roster_dirty: true,
            running: false,
            started: 0,
            map_since: 0,
            rotation_warned: false,
            in_map_event: false,
            bans_revision: 1,
            now: 0,
            last_roster: 0,
            last_world_state: 0,
            next_object_update: 0,
            travel_started: 0,
            plugins: PluginManager::default(),
        }
    }

    fn log(&self, text: &str) {
        (self.log)(text)
    }

    fn apply_activity(&mut self, events: Vec<Activity>) {
        for event in events {
            match event {
                Activity::Log(text) => {
                    if self.config.activity_log {
                        self.log(&text)
                    }
                }
                Activity::Announce(text) => {
                    if self.config.announce_throwdowns {
                        self.send_chat(&text, None)
                    }
                }
            }
        }
    }

    fn packet(&mut self, kind: u16, now: u64) -> Packet {
        self.sequence = self.sequence.wrapping_add(1);
        Packet {
            kind,
            sequence: self.sequence,
            session: self.secret,
            epoch: self.epoch,
            map: self.map,
            time_us: now,
            source: self.id,
            world: self.world,
            tps: self.config.tps,
            pose_interval_us: multiplayer_pose_interval(self.config.tps),
            build: game_sha256_bytes(),
            ..Default::default()
        }
    }

    fn capacity(&self) -> u32 {
        self.config.max_players + 1
    }
    fn is_admin(&self, id: u64) -> bool {
        self.config.admins.contains(&id)
    }
    fn is_banned(&self, id: u64) -> bool {
        self.config.bans.iter().any(|b| b.id == id)
    }
    // Banned by the ReSkate team, unless this server lets them in ("global_bans": false).
    pub fn globally_banned(&self, id: u64) -> bool {
        self.config.global_bans && self.global_bans.binary_search(&id).is_ok()
    }
    // A new list from the backend (sorted): from the next tick on, it turns those players away,
    // also when they are already on.
    pub fn set_global_bans(&mut self, ids: Vec<u64>) {
        self.global_bans = ids;
    }
    fn save(&self) {
        if let Err(e) = save_config(&self.config) {
            self.log(&format!("Could not save the config: {e}"));
        }
    }
    fn name_of(&self, id: u64) -> String {
        self.guests.get(&id).map(|g| guest_name(g)).unwrap_or_default()
    }
    // The name a joining player is known by: the one their game sent, which the server cannot
    // check against Steam, made safe to show and to type. Never blank, never the server's or
    // ReSkate's own, never read as a SteamID64 by kick or ban, never the same as another player's.
    fn player_name(&self, wanted: &str, id: u64) -> String {
        let fallback = format!("Player {}", id % 10000);
        let mut name = prefix(&clean_roster_name(wanted), MAX_MEMBER_NAME).to_string();
        // Names show in every player's roster, nametags and party UI.
        if contains_bad_words(&name) {
            name = mask_bad_words(&name);
        }
        let folded = lower(&name);
        let (first, _) = split(&name);
        let digits = !first.is_empty() && first.bytes().all(|c| c.is_ascii_digit());
        if name.is_empty() || folded == "server" || folded == "reskate" || folded == lower(&self.config.name) {
            name = fallback;
        } else if digits {
            name = format!("Player {name}");
        }
        let taken = |candidate: &str| {
            self.guests.iter().any(|(&other, g)| other != id && g.handshaken && lower(&g.member.name) == lower(candidate))
        };
        let mut unique = name.clone();
        let mut copy = 2;
        while taken(&unique) && copy < 1000 {
            unique = format!("{name} ({copy})");
            copy += 1;
        }
        prefix(&clean_roster_name(&unique), 128).to_string()
    }

    // Every guest's ID, for loops that may drop guests while they walk. The list's memory is
    // reused: hand it back with spare_ids() when done (a loop left early just loses it).
    fn guest_ids(&mut self) -> Vec<u64> {
        let mut ids = std::mem::take(&mut self.spare_ids);
        ids.clear();
        ids.extend(self.guests.keys().copied());
        ids
    }
    fn spare_ids(&mut self, ids: Vec<u64>) {
        if ids.capacity() > self.spare_ids.capacity() {
            self.spare_ids = ids;
        }
    }

    fn handshaken(&self, id: u64) -> bool {
        self.guests.get(&id).is_some_and(|g| g.handshaken)
    }

    // ---- Plugins ---------------------------------------------------------------------------
    pub fn load_plugins(&mut self, dir: &std::path::Path) {
        for line in self.plugins.load(dir, crate::now_us()) {
            self.log(&line);
        }
    }
    fn player_info(&self, id: u64) -> PlayerInfo {
        let online_seconds = self.guests.get(&id).map_or(0, |g| self.online_seconds(g));
        PlayerInfo { id, name: self.name_of(id), admin: self.is_admin(id), online_seconds }
    }
    // How long a player has been connected, in whole seconds.
    fn online_seconds(&self, g: &Guest) -> u64 {
        if g.connected_at != 0 && self.now > g.connected_at {
            (self.now - g.connected_at) / 1_000_000
        } else {
            0
        }
    }
    fn uptime_seconds(&self) -> u64 {
        if self.started != 0 && self.now > self.started {
            (self.now - self.started) / 1_000_000
        } else {
            0
        }
    }
    // Listed in the server browser: set so, and with a name clients show.
    fn listed(&self) -> bool {
        self.config.listed && valid_server_name(&self.config.name) && !contains_bad_words(&self.config.name)
    }
    fn plugin_snapshot(&self) -> Snapshot {
        Snapshot {
            players: self.guests.iter().filter(|(_, g)| g.handshaken).map(|(&id, _)| self.player_info(id)).collect(),
            server: self.config.name.clone(),
            map: self.map_name(),
            max_players: self.config.max_players,
            password: !self.config.password.is_empty(),
            listed: self.listed(),
            uptime_seconds: self.uptime_seconds(),
        }
    }
    fn apply_plugin_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Broadcast(text) => {
                    for line in text.split('\n').filter(|l| !trim(l).is_empty()).take(12) {
                        self.send_chat(line, None);
                    }
                }
                Action::Tell(id, text) => {
                    if self.handshaken(id) {
                        self.reply(id, &text);
                    }
                }
                Action::Log(text) => self.log(&text),
                Action::Run(plugin, line) => {
                    let answer = self.command(&line, 0);
                    self.log(&format!("[{plugin}] {}: {answer}", loggable(&line)));
                }
            }
        }
    }

    pub fn invite(&self) -> String {
        format_invite(self.id, self.secret)
    }
    pub fn map_name(&self) -> String {
        map_label(&self.config.map)
    }
    // What GET /status answers (src/status.rs): public facts only. The join code only for a
    // listed server, whose code the server browser shows anyway; player names only if allowed.
    pub fn status(&self) -> serde_json::Value {
        let listed = self.listed();
        let mut players: Vec<serde_json::Value> = Vec::new();
        if self.config.status_players {
            for g in self.guests.values().filter(|g| g.handshaken) {
                players.push(serde_json::json!({
                    "name": guest_name(g),
                    "admin": self.is_admin(g.member.id),
                    "online_seconds": self.online_seconds(g),
                }));
            }
        }
        let updated = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        serde_json::json!({
            "name": self.config.name,
            "version": crate::update::VERSION,
            "protocol": PROTOCOL_VERSION,
            "map": self.map_name(),
            "players": self.players(),
            "max_players": self.config.max_players,
            "password": !self.config.password.is_empty(),
            "listed": listed,
            "join_code": if listed && self.id != 0 { serde_json::Value::from(self.invite()) } else { serde_json::Value::Null },
            "uptime_seconds": self.uptime_seconds(),
            "player_list": if self.config.status_players { serde_json::Value::from(players) } else { serde_json::Value::Null },
            "updated": updated,
        })
    }
    // Everyone connected, including players still joining.
    pub fn connected(&self) -> usize {
        self.guests.len()
    }
    pub fn players(&self) -> u32 {
        self.guests.values().filter(|g| g.handshaken).count() as u32
    }
    pub fn secret(&self) -> u64 {
        self.secret
    }

    // Opens the listener and starts a session under the given server identity.
    pub fn start(&mut self) -> Result<(), String> {
        if !self.transport.host(self.capacity() as usize) {
            return Err(self.transport.detail.clone());
        }
        self.id = self.transport.local_id;
        self.started = crate::now_us();
        self.secret = nonce();
        self.epoch = nonce();
        self.world = 1;
        self.map = map_hash(&map_destination(&self.config.map));
        self.password = if self.config.password.is_empty() { None } else { password_key(&self.config.password, self.secret) };
        self.voice_policy = VoicePolicy { allowed: self.config.voice_chat, revision: 1 };
        self.apply_layers();
        self.running = true;
        self.roster_dirty = true;
        Ok(())
    }

    pub fn stop(&mut self, reason: &str) {
        if !self.running {
            return;
        }
        // Plugins save what they keep, and may say goodbye while everyone is still there.
        self.now = crate::now_us();
        let actions = self.plugins.stop(self.plugin_snapshot(), reason);
        self.apply_plugin_actions(actions);
        let away = self.packet(kind::AWAY, crate::now_us());
        let ids: Vec<u64> = self.guests.iter().filter(|(_, g)| g.handshaken).map(|(&id, _)| id).collect();
        for id in ids {
            self.send_packet(id, &away, true, false);
        }
        let ids: Vec<u64> = self.guests.keys().copied().collect();
        for id in ids {
            self.transport.disconnect(id, reason);
        }
        self.guests.clear();
        self.transport.stop();
        self.running = false;
    }

    fn apply_layers(&mut self) {
        self.layers = default_world_layers();
        if !self.config.world_layer_sync {
            return;
        }
        for (i, layer) in world_layers().iter().enumerate() {
            if let Some(mode) = self.config.layers.get(&layer.key) {
                if valid_world_layer_mode(mode) {
                    self.layers[i] = mode.clone();
                }
            }
        }
    }

    // ---- Sending ---------------------------------------------------------------------------
    fn send_to(transport: &mut SteamTransport, g: &mut Guest, p: &Packet, reliable: bool, fresh: bool, encoded: Option<(&[u8], &[u8])>) -> bool {
        let update = match encoded {
            None => g.sender.prepare(p),
            Some((raw, wire)) => g.sender.prepare_with(p, raw, wire),
        };
        // A packet that cannot be built for anyone (an outfit too big to relay) is its source's
        // fault, never this recipient's: nothing is sent, and the recipient is not dropped.
        if update.bytes.is_empty() {
            return true;
        }
        // A stream and its reliable delta references must always use the same lane.
        if !transport.send(g.member.id, &update.bytes, reliable || update.establishes_baseline(), fresh, traffic_lane(p.kind)) {
            return false;
        }
        g.sender.sent(p, update);
        true
    }

    fn send_packet(&mut self, id: u64, p: &Packet, reliable: bool, fresh: bool) -> bool {
        let Some(g) = self.guests.get_mut(&id) else { return false };
        Self::send_to(&mut self.transport, g, p, reliable, fresh, None)
    }

    fn send_required(&mut self, id: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let sent = match decode_wire(bytes) {
            Some(p) => self.send_packet(id, &p, true, false),
            None => false,
        };
        if !sent {
            self.transport.disconnect(id, "Cannot deliver required session data. Join again.");
        }
    }

    fn broadcast(&mut self, packet: &Packet, reliable: bool, fresh: bool, except: u64) {
        let now = self.now;
        let source = self.guests.get(&packet.source).map(|s| (s.latest_root, s.pose_arrival, s.member.id, s.member.epoch));
        let mut encoded: [Option<(Packet, Vec<u8>, Vec<u8>)>; 3] = [None, None, None];
        let gameplay = matches!(packet.kind, kind::POSE | kind::AUDIO | kind::VOICE | kind::COSMETICS);
        let ids = self.guest_ids();
        for &id in &ids {
            let p = self.guests.get_mut(&id).unwrap();
            if !p.handshaken || id == except || (gameplay && !p.world_ready) {
                continue;
            }
            if packet.kind == kind::VOICE {
                if !self.voice_policy.accepts(&packet.voice) {
                    continue;
                }
                // The server forwards proximity voice within its range. Unknown positions go through.
                if packet.voice.distance > 0.0 {
                    if let Some((source_root, source_arrival, _, _)) = source {
                        let known = source_root.is_some()
                            && p.latest_root.is_some()
                            && p.pose_arrival != 0
                            && now.wrapping_sub(p.pose_arrival) <= 3_000_000
                            && source_arrival != 0
                            && now.wrapping_sub(source_arrival) <= 3_000_000;
                        if known
                            && voice_gain(&source_root.unwrap().position, &p.latest_root.unwrap().position, self.config.voice_range) <= 0.0
                        {
                            continue;
                        }
                    }
                }
            }
            // Frequent gameplay streams skip the server once the receiver confirms a direct route.
            if let Some((_, _, source_id, source_epoch)) = source {
                if (packet.kind == kind::POSE || packet.kind == kind::AUDIO)
                    && !needs_relay(&p.direct_routes, p.route_reported, source_id, source_epoch, now)
                {
                    continue;
                }
            }
            let mut delivery: Option<usize> = None;
            let mut interval = multiplayer_pose_interval(self.config.tps);
            if packet.kind == kind::POSE {
                let index = match p.pose_delivery.iter().position(|v| v.source == packet.source) {
                    Some(index) => index,
                    None => {
                        let mut best = 0;
                        for (i, v) in p.pose_delivery.iter().enumerate() {
                            if v.last_sent < p.pose_delivery[best].last_sent {
                                best = i;
                            }
                        }
                        best
                    }
                };
                let d = &mut p.pose_delivery[index];
                if d.source != packet.source || d.epoch != packet.epoch {
                    *d = PoseDelivery { source: packet.source, epoch: packet.epoch, last_sent: 0, interval_us: interval, next_source_time: 0 };
                }
                if let Some(root) = p.latest_root {
                    if p.pose_arrival != 0 && now >= p.pose_arrival && now - p.pose_arrival <= 1_000_000 {
                        let mut distance = 0.0f32;
                        for i in 0..3 {
                            let v = root.position[i] - packet.pose.root.position[i];
                            distance += v * v;
                        }
                        interval = pose_interval(distance, p.pose_delivery[index].interval_us, &self.config.distances, self.config.tps);
                    }
                }
                let d = &mut p.pose_delivery[index];
                if d.interval_us != interval {
                    d.next_source_time = 0;
                }
                d.interval_us = interval;
                if interval > multiplayer_pose_interval(self.config.tps) && packet.time_us < d.next_source_time {
                    continue;
                }
                delivery = Some(index);
            }
            let variant = if interval == 200_000 {
                2
            } else if interval == 100_000 {
                1
            } else {
                0
            };
            if encoded[variant].is_none() {
                let mut copy = packet.clone();
                copy.pose_interval_us = interval;
                let raw = encode(&copy, true);
                let wire = encode_wire_bytes(&raw);
                encoded[variant] = Some((copy, raw, wire));
            }
            let (data, raw, wire) = encoded[variant].as_ref().unwrap();
            let sent = Self::send_to(&mut self.transport, p, data, reliable, fresh, Some((raw, wire)));
            if sent {
                if let Some(index) = delivery {
                    let d = &mut p.pose_delivery[index];
                    d.last_sent = now;
                    advance_pose_deadline(&mut d.next_source_time, packet.time_us, u64::from(interval));
                }
            }
            if !sent && reliable && !fresh {
                self.transport.disconnect(id, "Cannot deliver required session data. Join again.");
            }
        }
        self.spare_ids(ids);
    }

    fn send_roster(&mut self) {
        let mut p = self.packet(kind::ROSTER, self.now);
        p.voice_policy = self.voice_policy;
        p.voice_range = self.config.voice_range;
        p.distances = self.config.distances;
        p.object_placement = self.config.object_placement;
        p.guest_noclip = self.config.noclip;
        p.guest_no_bail = self.config.no_bail;
        p.guest_boosts = self.config.boosts;
        p.enforce_tuning = self.config.enforce_tuning;
        p.server_votes = self.enabled_votes();
        p.object_clears = self.object_clears;
        // Without the catalog (or with sync off) no layers are sent: every player keeps their own.
        p.force_world_layers = self.config.world_layer_sync && !world_layers().is_empty();
        p.layers = if p.force_world_layers { pack_world_layers(&self.layers) } else { Vec::new() };
        p.parks = self.config.parks.clone();
        p.capacity = self.capacity();
        p.members.push(Member { id: self.id, epoch: self.epoch, name: self.config.name.clone(), ..Default::default() });
        let ids: Vec<u64> = self.guests.keys().copied().collect();
        for id in ids {
            if !self.guests[&id].handshaken {
                continue;
            }
            let admin = self.is_admin(id);
            let party = self.parties.party_of(id);
            let details = self.parties.party(party).cloned();
            let g = guest!(self, id);
            g.member.admin = admin;
            g.member.party = party;
            g.member.party_leader = details.as_ref().is_some_and(|d| d.leader == id);
            g.member.party_open = g.member.party_leader && details.as_ref().is_some_and(|d| d.open);
            g.member.speeding = g.speeding;
            g.member.scoring = g.scoring_flagged;
            p.members.push(g.member.clone());
        }
        // Parties hold only handshaken players; this only guards the roster's rules: every listed
        // party has two members and one leader.
        let parties: Vec<u32> = p.members.iter().map(|m| m.party).collect();
        for m in p.members.iter_mut() {
            if m.party != 0 && parties.iter().filter(|&&o| o == m.party).count() < 2 {
                m.party = 0;
                m.party_leader = false;
                m.party_open = false;
            }
        }
        for i in 0..p.members.len() {
            let party = p.members[i].party;
            if party != 0 && !p.members.iter().any(|o| o.party == party && o.party_leader) {
                p.members[i].party_leader = true; // the first listed member of a party missing its leader
            }
        }
        self.broadcast(&p, true, false, 0);
        self.roster_dirty = false;
        self.last_roster = self.now;
    }

    fn send_world_state(&mut self) {
        let mut state = self.packet(kind::WORLD_STATE, self.now);
        state.destination = map_destination(&self.config.map);
        state.map_label = self.wire_map_label();
        state.world_ready = true; // the server has nothing to load
        let bytes = encode_wire(&state);
        let ids: Vec<u64> = self.guests.iter().filter(|(_, g)| g.handshaken).map(|(&id, _)| id).collect();
        for id in ids {
            self.send_required(id, &bytes);
        }
        self.last_world_state = self.now;
    }

    fn send_chat(&mut self, text: &str, only: Option<u64>) {
        let mut message = self.packet(kind::CHAT, self.now);
        message.text = clean_chat_text(text);
        if message.text.is_empty() {
            return;
        }
        match only {
            Some(id) => {
                self.send_packet(id, &message, true, false);
            }
            None => self.broadcast(&message, true, false, 0),
        }
    }

    fn send_bans(&mut self, admin: u64) {
        let mut list = self.packet(kind::BANS, self.now);
        list.ban_total = self.config.bans.len() as u32;
        // Newest first; a very long list sends only its newest rows.
        for ban in self.config.bans.iter().rev() {
            if list.bans.len() >= MAX_BAN_ROWS {
                break;
            }
            let mut row = ban.clone();
            let mut name = row.name.clone().into_bytes();
            while !name.is_empty() && !valid_member_name(&name) {
                name.pop();
            }
            row.name = String::from_utf8(name).unwrap_or_default();
            if individual_steam_id(row.id) {
                list.bans.push(row);
            }
        }
        if self.send_packet(admin, &list, true, false) {
            guest!(self, admin).bans_sent = self.bans_revision;
        }
    }

    // The map's name as map offers and world states carry it, so players without the map's mod
    // still see which map it is.
    fn wire_map_label(&self) -> String {
        let label = prefix(&self.map_name(), MAX_MEMBER_NAME).to_string();
        if valid_map_label(&label) {
            label
        } else {
            String::new()
        }
    }

    // Admins: every map the server knows, and the pool as indices into them. Players: the pool.
    fn send_maps(&mut self, id: u64) {
        let mut list = self.packet(kind::MAPS, self.now);
        let add = |asset: String, maps: &mut Vec<String>| {
            if valid_map_asset(&asset) && maps.len() < MAX_SERVER_MAPS {
                maps.push(asset);
            }
        };
        if self.is_admin(id) {
            for level in levels() {
                add(level.asset, &mut list.maps);
            }
            if !self.config.map_pool.is_empty() {
                for level in pool_levels(&self.config) {
                    if let Some(at) = list.maps.iter().position(|asset| *asset == level.asset) {
                        list.map_pool.push(at as u16);
                    }
                }
            }
        } else {
            for level in pool_levels(&self.config) {
                add(level.asset, &mut list.maps);
            }
        }
        list.map_rotation = self.config.map_rotation.min(MAX_MAP_ROTATION) as u16;
        if self.send_packet(id, &list, true, false) {
            guest!(self, id).maps_sent = true;
        }
    }
    // After the pool, the rotation or the admins change: everyone gets their list again.
    fn resend_maps(&mut self) {
        for g in self.guests.values_mut() {
            g.maps_sent = false;
        }
    }

    fn change_map(&mut self, map: &str) {
        // Everything tied to the old world goes; admission and player slots stay.
        let now = self.now;
        for g in self.guests.values_mut() {
            let old = std::mem::take(g.as_mut());
            g.member = old.member;
            g.handshaken = old.handshaken;
            g.map_authorized = old.map_authorized;
            g.password_challenge = old.password_challenge;
            g.connected_at = old.connected_at;
            // Rate limits and anti-cheat flags belong to the player, not the world.
            g.budget = old.budget;
            g.chat_rate = old.chat_rate;
            g.admin_budget = old.admin_budget;
            g.throwdown_budget = old.throwdown_budget;
            g.party_budget = old.party_budget;
            g.scoring_budget = old.scoring_budget;
            g.speeding = old.speeding;
            g.scoring = old.scoring;
            g.scoring_mods = old.scoring_mods;
            g.scoring_flagged = old.scoring_flagged;
            g.world_ready = false;
            g.travel_since = if g.handshaken { now } else { 0 };
            g.last_packet = now;
        }
        self.world = self.world.wrapping_add(1);
        if self.world == 0 {
            panic!("Map transition counter exhausted.");
        }
        self.activity.clear(); // throwdowns end with the world
        self.config.map = map_setting(map);
        self.map = map_hash(&map_destination(&self.config.map));
        self.travel_started = now;
        self.map_since = now;
        self.rotation_warned = false;
        self.last_world_state = 0;
        self.send_world_state();
        self.roster_dirty = true;
        if !self.in_map_event {
            self.in_map_event = true;
            let actions = self.plugins.map(self.plugin_snapshot(), &self.map_name());
            self.apply_plugin_actions(actions);
            self.in_map_event = false;
        }
    }

    fn drop_guest(&mut self, id: u64, reason: &str) {
        self.transport.disconnect(id, reason);
        let Some(guest) = self.guests.remove(&id) else { return };
        let name = guest_name(&guest);
        if guest.handshaken {
            self.roster_dirty = true;
            self.log(&format!("{name} left ({reason})"));
            let guests = &self.guests;
            self.activity.left(id, &|player| current_name(guests, player));
            let player = PlayerInfo { id, name: name.clone(), admin: self.is_admin(id), online_seconds: self.online_seconds(&guest) };
            let actions = self.plugins.leave(self.plugin_snapshot(), &player, reason);
            self.apply_plugin_actions(actions);
        } else {
            // Never admitted: it held a player slot meanwhile, so it shows in the log, and an ID
            // that keeps failing waits longer each time before its connection is taken again.
            let failures = self.join_backoff.failed(id, self.now);
            let attempt = if failures > 1 { format!(", attempt {failures}") } else { String::new() };
            self.log(&format!("[join] {id} did not finish joining ({reason}){attempt}"));
        }
        self.party_left(id, &name);
        // Their vote cooldown stays (tick() drops it once it has run out): leaving and coming
        // back does not let a player start another vote sooner.
        // One voter fewer, or the player a kick vote was about: counted in tick().
        self.vote_recount = true;
    }

    // ---- Receiving -------------------------------------------------------------------------
    fn accept_data(&mut self, id: u64, p: &Packet) -> bool {
        let now = self.now;
        let source = guest!(self, id);
        let accepted = match p.kind {
            kind::COSMETICS => {
                let accepted = source.outfit_budget.accept(now) && source.appearance.push(p);
                if accepted {
                    source.cosmetic_packet = encode_wire(p);
                }
                accepted
            }
            kind::AUDIO => source.sound_budget.accept(now, p.audio.len()) && source.audio.push(p, now),
            kind::POSE => source.poses.push_validated(p, now),
            _ => false,
        };
        // The server never plays anything back: keep only what ordering needs.
        if source.poses.size() > 4 || source.audio.size() > 16 {
            source.poses.clear();
            source.audio.clear();
        }
        if !accepted {
            return false;
        }
        source.last_packet = now;
        if p.kind == kind::POSE {
            source.pose_arrival = now;
            source.latest_root = Some(p.pose.root);
            return self.check_speed(id, p.time_us); // last: a speed-check kick frees the guest
        }
        true
    }

    // `arrived`: when Steam received the message (TransportMessage::arrived), 0 if unknown.
    fn receive(&mut self, peer: u64, bytes: &[u8], arrived: u64) {
        let now = self.now;
        let world = self.world;
        let Some(link) = self.guests.get_mut(&peer) else { return };
        // Counted by arrival: after the server was held up, everyone's packets are read at once.
        if !link.budget.accept(if arrived != 0 { arrived } else { now }, bytes.len(), 1) {
            return self.drop_guest(peer, "Peer exceeded the multiplayer packet limit.");
        }
        let mut missing = false;
        let decoded = link.receiver.receive(bytes, &mut missing, world);
        if missing {
            return;
        }
        let p = match decoded {
            Some(p) if p.session == self.secret && p.source == peer => p,
            _ => return self.drop_guest(peer, "Join code, sender identity, or multiplayer protocol did not match."),
        };
        match p.kind {
            kind::WORLD_STATE => return self.drop_guest(peer, "Only the server may change the room's map."),
            // Sent once the player's game knows, and again if a mod enabled later changes it.
            kind::SCORING => {
                let link = guest!(self, peer);
                if !link.handshaken || p.epoch != link.member.epoch || !link.scoring_budget.accept(now, 4) {
                    return;
                }
                link.last_packet = now;
                if link.scoring == Some(p.scoring) && link.scoring_mods == p.text {
                    return;
                }
                link.scoring = Some(p.scoring);
                link.scoring_mods = p.text.clone();
                return self.check_scoring(peer);
            }
            kind::WORLD_READY => {
                let map = self.map;
                let link = guest!(self, peer);
                if !link.handshaken || p.epoch != link.member.epoch {
                    return self.drop_guest(peer, "Invalid map readiness message.");
                }
                if p.world != world || p.map != map || (link.ready_sequence != 0 && !newer_sequence(p.sequence, link.ready_sequence)) {
                    return;
                }
                link.ready_sequence = p.sequence;
                link.last_packet = now;
                let arrived = p.world_ready && !link.world_ready;
                if !p.world_ready && link.world_ready {
                    link.loading_since = now;
                }
                let since = if link.travel_since != 0 { link.travel_since } else { link.loading_since };
                let name = guest_name(link);
                link.world_ready = p.world_ready;
                if p.world_ready {
                    link.travel_since = 0;
                    link.loading_since = 0;
                }
                if arrived && self.config.activity_log && since != 0 && now > since {
                    self.log(&format!("[map] {name} finished loading ({} s)", (now - since) / 1_000_000));
                }
                if arrived {
                    self.send_others_cosmetics(peer);
                }
                return;
            }
            kind::MAP_REQUEST => {
                let link = guest!(self, peer);
                if p.build != game_sha256_bytes() || (link.member.epoch != 0 && link.member.epoch != p.epoch) {
                    return self.drop_guest(peer, "Invalid map request. Update ReSkate to the server's version and join again.");
                }
                if link.handshaken {
                    return;
                }
                if p.map != 0 && p.map != self.map {
                    return self.drop_guest(peer, "The server changed maps while you were joining. Join again.");
                }
                link.member.epoch = p.epoch;
                let mut authorized = self.password.is_none();
                if let Some(key) = &self.password {
                    if link.password_challenge == 0 {
                        link.password_challenge = nonce();
                    }
                    if p.challenge == link.password_challenge {
                        let proof = password_proof(key, self.secret, self.map, self.id, peer, self.epoch, p.epoch, link.password_challenge);
                        if !proof_matches(&proof, &p.proof) {
                            return self.drop_guest(peer, "Incorrect server password.");
                        }
                        authorized = true;
                    }
                }
                let newly_authorized = authorized && !link.map_authorized;
                link.map_authorized |= authorized;
                if newly_authorized || link.last_map_offer == 0 || now.wrapping_sub(link.last_map_offer) >= 1_000_000 {
                    let challenge = link.password_challenge;
                    let mut offer = self.packet(kind::MAP_OFFER, now);
                    offer.destination = map_destination(&self.config.map);
                    offer.map_label = self.wire_map_label();
                    offer.challenge = challenge;
                    offer.map_authorized = authorized;
                    self.send_required(peer, &encode_wire(&offer));
                    if let Some(link) = self.guests.get_mut(&peer) {
                        link.last_map_offer = now;
                    }
                }
                return;
            }
            kind::MAP_OFFER => {
                if self.guests[&peer].handshaken || p.world < world {
                    return;
                }
                return self.drop_guest(peer, "Only the server offers maps.");
            }
            _ => {}
        }
        // A map reload can return to the same asset. The generation keeps late poses, audio,
        // cosmetics and control from reviving the old world.
        if p.world != world {
            return;
        }
        match p.kind {
            kind::PEER_HELLO | kind::PEER_WELCOME => return self.drop_guest(peer, "Direct peer is not admitted to this session."),
            kind::CHALLENGE => return self.drop_guest(peer, "Invalid lobby password challenge."),
            kind::WELCOME => return self.drop_guest(peer, "Unexpected multiplayer handshake direction."),
            kind::HELLO => return self.hello(peer, &p),
            kind::COSMETICS => {
                let link = guest!(self, peer);
                link.pending_cosmetics.retain(|item| now.wrapping_sub(item.received) <= 10_000_000);
                if let Some(found) =
                    link.pending_cosmetics.iter_mut().find(|item| item.packet.source == p.source && item.packet.epoch == p.epoch)
                {
                    if newer_sequence(p.sequence, found.packet.sequence) {
                        *found = PendingCosmetics { packet: p, received: now };
                    }
                } else {
                    if link.pending_cosmetics.len() >= 4 {
                        link.pending_cosmetics.remove(0);
                    }
                    link.pending_cosmetics.push(PendingCosmetics { packet: p, received: now });
                }
                return;
            }
            _ => {}
        }
        let id = self.id;
        let link = guest!(self, peer);
        if !link.handshaken || p.map != self.map {
            return;
        }
        if p.kind == kind::ROUTES {
            if p.epoch != link.member.epoch {
                return self.drop_guest(peer, "Invalid direct route report.");
            }
            if link.route_reported == 0 || newer_sequence(p.sequence, link.route_sequence) {
                let routes: Vec<Member> = p
                    .members
                    .iter()
                    .filter(|m| {
                        m.id != peer && self.guests.get(&m.id).is_some_and(|o| o.handshaken && o.member.epoch == m.epoch)
                    })
                    .cloned()
                    .collect();
                let link = guest!(self, peer);
                link.direct_routes = routes;
                link.route_reported = now;
                link.route_sequence = p.sequence;
            }
            return;
        }
        match p.kind {
            kind::ROSTER => return self.drop_guest(peer, "Only the server may publish the player roster."),
            kind::TELEPORT => return self.drop_guest(peer, "Only the server may teleport players."),
            kind::PHYSICS_TUNING | kind::PHYSICS_EXTRAS => return, // a listen host's; the server's physics are the game's own
            _ => {}
        }
        let _ = id;
        if p.kind == kind::CHAT {
            if !routed_source(&p, &link.member, peer) || !link.chat_rate.accept(now, &p.text, 1.0) {
                return;
            }
            link.last_packet = now;
            let name = guest_name(link);
            // "/" starts a command (votes; any server command for admins), answered to the sender only.
            if let Some(line) = p.text.strip_prefix('/') {
                self.log(&format!("[command] {name}: /{}", loggable(line)));
                return self.chat_command(peer, line);
            }
            self.log(&format!("[chat] {name}: {}", p.text));
            let (snapshot, player) = (self.plugin_snapshot(), self.player_info(peer));
            let (actions, allowed) = self.plugins.chat(snapshot, &player, &p.text);
            self.apply_plugin_actions(actions);
            if !allowed {
                return self.log(&format!("[chat] (kept from the others by a plugin) {name}"));
            }
            return self.broadcast(&p, true, false, p.source);
        }
        // Linked throwdowns: opaque to the server, relayed like chat to everyone else in the same world.
        if p.kind == kind::THROWDOWN {
            if p.world != world || !routed_source(&p, &link.member, peer) || !link.throwdown_budget.accept(now, 60) {
                return;
            }
            // A player whose tricks score differently takes part in nothing linked.
            if link.scoring_flagged {
                return;
            }
            link.last_packet = now;
            let at = link.latest_root.map(|r| r.position);
            if self.config.activity_log || self.config.announce_throwdowns {
                let guests = &self.guests;
                let events = self.activity.throwdown(peer, &p.throwdown, at, now, &|player| current_name(guests, player));
                self.apply_activity(events);
            }
            return self.broadcast(&p, true, false, p.source);
        }
        if p.kind == kind::PARTY {
            if !routed_source(&p, &link.member, peer) || !link.party_budget.accept(now, 20) {
                return;
            }
            link.last_packet = now;
            return self.party_request(peer, p.party_action, p.party_player);
        }
        if p.kind == kind::ADMIN {
            if !routed_source(&p, &link.member, peer) || !link.admin_budget.accept(now, 30) {
                return;
            }
            link.last_packet = now;
            let name = guest_name(link);
            let answer = if !self.is_admin(peer) {
                "You are not an admin on this server.".to_string()
            } else {
                self.log(&format!("[admin] {name}: {}", loggable(&p.text)));
                self.command(&p.text, peer)
            };
            if self.guests.contains_key(&peer) {
                let mut reply = self.packet(kind::ADMIN, self.now);
                reply.text = clean_chat_text(if answer.is_empty() { "Done." } else { &answer });
                if reply.text.is_empty() {
                    reply.text = "Done.".into();
                }
                // An answer longer than an admin line goes as its first 320 bytes.
                if !valid_admin_text(reply.text.as_bytes()) {
                    reply.text = prefix(&reply.text, MAX_ADMIN_TEXT).to_string();
                }
                self.send_packet(peer, &reply, true, false);
            }
            return;
        }
        let link = guest!(self, peer);
        if !link.world_ready {
            return;
        }
        if !routed_source(&p, &link.member, peer) {
            return;
        }
        if p.kind == kind::AWAY {
            return self.drop_guest(peer, "A player ended their session.");
        }
        if p.kind == kind::OBJECTS {
            if link.objects.receive(&p.objects) == ObjectResult::Invalid {
                return self.drop_guest(peer, "Invalid shared object revision or layout.");
            }
            link.last_packet = now;
            return;
        }
        if p.kind == kind::VOICE {
            if !self.voice_policy.accepts(&p.voice) {
                return;
            }
            if link.received_voice && !newer_sequence(p.sequence, link.voice_sequence) {
                return;
            }
            if !link.voice_budget.accept(now, p.voice.bytes.len()) {
                return;
            }
            link.received_voice = true;
            link.voice_sequence = p.sequence;
            link.last_packet = now;
            return self.broadcast(&p, false, true, p.source);
        }
        if self.accept_data(peer, &p) {
            let reliable = p.kind == kind::AUDIO && p.audio.iter().any(|s| s.event);
            self.broadcast(&p, reliable, true, p.source);
        }
    }

    fn send_others_cosmetics(&mut self, peer: u64) {
        let outfits: Vec<Vec<u8>> = self
            .guests
            .iter()
            .filter(|(&id, g)| g.handshaken && id != peer)
            .map(|(_, g)| g.cosmetic_packet.clone())
            .collect();
        for bytes in outfits {
            if !self.guests.contains_key(&peer) {
                return;
            }
            self.send_required(peer, &bytes);
        }
    }

    fn hello(&mut self, peer: u64, p: &Packet) {
        let now = self.now;
        let build = game_sha256_bytes();
        let link = guest!(self, peer);
        let error = greeting_error(p, self.secret, self.map, &build, if link.handshaken { link.member.epoch } else { 0 });
        if !error.is_empty() {
            return self.drop_guest(peer, error);
        }
        if let Some(key) = &self.password {
            if !link.handshaken {
                if link.password_challenge == 0 {
                    link.password_challenge = nonce();
                }
                if p.challenge != link.password_challenge {
                    let challenge_value = link.password_challenge;
                    let mut challenge = self.packet(kind::CHALLENGE, now);
                    challenge.challenge = challenge_value;
                    return self.send_required(peer, &encode_wire(&challenge));
                }
                let proof = password_proof(key, self.secret, self.map, self.id, peer, self.epoch, p.epoch, link.password_challenge);
                if !proof_matches(&proof, &p.proof) {
                    return self.drop_guest(peer, "Incorrect server password.");
                }
            }
        }
        let link = guest!(self, peer);
        let joined = !link.handshaken;
        link.member.epoch = p.epoch;
        if joined {
            let wanted = p.text.clone();
            let name = self.player_name(&wanted, peer);
            guest!(self, peer).member.name = name;
            self.join_backoff.joined(peer);
        }
        let link = guest!(self, peer);
        link.handshaken = true;
        link.world_ready = true;
        link.travel_since = 0;
        link.last_packet = now;
        let welcome = self.packet(kind::WELCOME, now);
        self.send_required(peer, &encode_wire(&welcome));
        if joined {
            self.send_roster();
            self.send_others_cosmetics(peer);
            if !self.config.welcome.is_empty() {
                let welcome = self.config.welcome.clone();
                self.send_chat(&welcome, Some(peer));
            }
            let Some(link) = self.guests.get(&peer) else { return };
            let loaded = if link.connected_at != 0 && now > link.connected_at {
                format!(", loaded in {} s", (now - link.connected_at) / 1_000_000)
            } else {
                String::new()
            };
            let text = format!(
                "{} joined ({}{}), {}/{} players{}",
                guest_name(link),
                peer,
                if self.is_admin(peer) { ", admin" } else { "" },
                self.players(),
                self.config.max_players,
                loaded
            );
            self.log(&text);
            let (snapshot, player) = (self.plugin_snapshot(), self.player_info(peer));
            let actions = self.plugins.join(snapshot, &player);
            self.apply_plugin_actions(actions);
        }
    }

    fn receive_cosmetics(&mut self) {
        let now = self.now;
        let world = self.world;
        let ids = self.guest_ids();
        for &id in &ids {
            let Some(link) = self.guests.get_mut(&id) else { continue };
            let pending = std::mem::take(&mut link.pending_cosmetics);
            for item in pending {
                let p = &item.packet;
                if p.world != world || now.wrapping_sub(item.received) > 10_000_000 {
                    continue;
                }
                let link = guest!(self, id);
                // Lanes may deliver an outfit before the hello or readiness; keep it until then.
                if !link.handshaken || !link.world_ready {
                    link.pending_cosmetics.push(item);
                    continue;
                }
                if p.map == self.map && routed_source(p, &link.member, id) && self.accept_data(id, p) {
                    self.broadcast(p, true, false, p.source);
                }
            }
        }
        self.spare_ids(ids);
    }

    // ---- Objects ---------------------------------------------------------------------------
    fn sync_objects(&mut self) {
        let now = self.now;
        if now < self.next_object_update {
            return;
        }
        self.next_object_update = now + 100_000;
        // Guests upload their layouts; the server alone decides what everyone else sees, and
        // freezes the layouts of players who may not build right now.
        let ids: Vec<u64> = self.guests.keys().copied().collect();
        for &id in &ids {
            let allowed = self.config.object_placement == placement::EVERYONE
                || (self.config.object_placement == placement::HOST_ONLY && self.is_admin(id));
            let g = &self.guests[&id];
            if !g.handshaken || g.objects.revision() == g.shared_from || !allowed {
                continue;
            }
            let mut layout = g.objects.layout();
            layout.retain(|object| !g.cleared.contains(&object.id));
            if self.config.activity_log {
                let before = g.shared.layout();
                let guests = &self.guests;
                let events = self.activity.objects(id, &before, &layout, &|player| current_name(guests, player));
                self.apply_activity(events);
            }
            let g = guest!(self, id);
            g.shared.replace(&layout);
            g.shared_from = g.objects.revision();
        }
        let sources: Vec<(u64, u64)> = self
            .guests
            .iter()
            .filter(|(_, g)| g.handshaken && g.world_ready)
            .map(|(&id, g)| (id, g.member.epoch))
            .collect();
        for &id in &ids {
            let Some(peer) = self.guests.get_mut(&id) else { continue };
            if !peer.handshaken || !peer.world_ready || sources.is_empty() {
                continue;
            }
            let has_source = |source: u64, epoch: u64| sources.iter().any(|&(s, e)| s == source && e == epoch);
            let delivery = &mut peer.object_delivery;
            delivery.sent.retain(|&source, row| has_source(source, row.0));
            if !delivery.chunks.is_empty() && !has_source(delivery.source, delivery.epoch) {
                delivery.chunks.clear();
            }
            for _ in 0..4 {
                if self.guests[&id].object_delivery.chunks.is_empty() {
                    for _ in 0..sources.len() {
                        let delivery = &mut guest!(self, id).object_delivery;
                        let (source_id, source_epoch) = sources[delivery.cursor % sources.len()];
                        delivery.cursor += 1;
                        let since = match delivery.sent.get(&source_id) {
                            Some(&(epoch, revision)) if epoch == source_epoch => revision,
                            _ => 0,
                        };
                        let state = &self.guests[&source_id].shared;
                        if source_id == id || state.revision() == 0 {
                            continue;
                        }
                        let chunks = state.updates(since);
                        if chunks.is_empty() {
                            continue;
                        }
                        let delivery = &mut guest!(self, id).object_delivery;
                        delivery.chunks = chunks;
                        delivery.source = source_id;
                        delivery.epoch = source_epoch;
                        delivery.next = 0;
                        break;
                    }
                    if self.guests[&id].object_delivery.chunks.is_empty() {
                        break;
                    }
                }
                let (source, epoch, chunk) = {
                    let d = &self.guests[&id].object_delivery;
                    (d.source, d.epoch, d.chunks[d.next].clone())
                };
                let mut update = self.packet(kind::OBJECTS, now);
                update.source = source;
                update.epoch = epoch;
                update.objects = chunk;
                if !self.send_packet(id, &update, true, false) {
                    self.transport.disconnect(id, "Cannot deliver shared object state. Join again.");
                    break;
                }
                let d = &mut guest!(self, id).object_delivery;
                d.next += 1;
                if d.next == d.chunks.len() {
                    d.sent.insert(d.source, (d.epoch, update.objects.revision));
                    d.chunks.clear();
                }
            }
        }
    }

    // ---- Tick ------------------------------------------------------------------------------
    pub fn tick(&mut self, now: u64) {
        if !self.running {
            return;
        }
        self.now = now;
        self.transport.poll();
        let mut links = std::mem::take(&mut self.spare_links);
        self.transport.peers_into(&mut links);
        let ids = self.guest_ids();
        for &id in &ids {
            if !links.iter().any(|l| l.id == id) {
                let reason = match self.transport.take_end_reason(id) {
                    Some(why) => format!("Disconnected: {}.", why.trim_end_matches('.')),
                    None => "Disconnected.".to_string(),
                };
                self.drop_guest(id, &reason);
            }
        }
        self.spare_ids(ids);
        for link in &links {
            if self.kicked.contains(&link.id) {
                self.transport.disconnect(link.id, "You were kicked from this server.");
                continue;
            }
            if self.is_banned(link.id) {
                self.transport.disconnect(link.id, "You are banned from this server.");
                continue;
            }
            if self.globally_banned(link.id) {
                self.transport.disconnect(link.id, crate::global_bans::BANNED_NOTICE);
                continue;
            }
            if !self.guests.contains_key(&link.id) && self.join_backoff.waiting(link.id, now) {
                self.transport.disconnect(link.id, "Too many failed attempts to join. Wait a little and try again.");
                continue;
            }
            if !self.guests.contains_key(&link.id) {
                if !individual_steam_id(link.id) || self.guests.len() >= self.config.max_players as usize {
                    self.transport.disconnect(link.id, "The server is full.");
                    continue;
                }
                let mut created = Box::<Guest>::default();
                created.member.id = link.id;
                created.last_packet = now;
                self.guests.insert(link.id, created);
            }
            let guest = guest!(self, link.id);
            if link.connected && guest.connected_at == 0 {
                guest.connected_at = now;
            }
        }
        self.spare_links = links;
        let mut messages = std::mem::take(&mut self.spare_messages);
        self.transport.receive_into(&mut messages);
        for message in &messages {
            self.receive(message.peer, &message.bytes, message.arrived);
        }
        // The messages' own bytes go; the list's memory stays for the next tick, unless a
        // backlog made it large.
        messages.clear();
        messages.shrink_to(1024);
        self.spare_messages = messages;
        self.receive_cosmetics();
        let ids = self.guest_ids();
        for &id in &ids {
            let Some(g) = self.guests.get(&id) else { continue };
            // Authorized arrivals get time for loading; this deadline is absolute.
            let handshake_timeout = if g.map_authorized { 180_000_000 } else { 8_000_000 };
            if g.handshaken && g.travel_since != 0 {
                if now.wrapping_sub(g.travel_since) > 180_000_000 {
                    self.drop_guest(id, "Could not finish loading the new map.");
                }
                continue;
            }
            if (!g.handshaken && g.connected_at != 0 && now.wrapping_sub(g.connected_at) > handshake_timeout)
                || (g.handshaken && now.wrapping_sub(g.last_packet) > GAMEPLAY_TIMEOUT_US)
            {
                self.drop_guest(id, "Timed out waiting for gameplay data.");
            }
        }
        self.spare_ids(ids);
        self.tick_parties();
        if self.roster_dirty || now.wrapping_sub(self.last_roster) > 2_000_000 {
            self.send_roster();
        }
        let ids = self.guest_ids();
        for &id in &ids {
            let Some(g) = self.guests.get(&id) else { continue };
            if !g.handshaken {
                continue;
            }
            let admin = self.is_admin(id);
            if admin && g.bans_sent != self.bans_revision {
                self.send_bans(id);
            }
            // Admins pick maps from it; with map votes on, everyone completes /vote map from it.
            if !self.guests[&id].maps_sent && (admin || self.config.votes.map.enabled) {
                self.send_maps(id);
            }
        }
        self.spare_ids(ids);
        let loading = self.guests.values().any(|g| g.handshaken && !g.world_ready);
        if self.world > 1
            && (loading || self.last_world_state == 0)
            && (self.last_world_state == 0 || now.wrapping_sub(self.last_world_state) >= 1_000_000)
        {
            self.send_world_state();
        }
        if self.travel_started != 0 && !loading {
            self.travel_started = 0;
            self.log(&format!("Everyone has loaded {}.", self.map_name()));
        }
        self.sync_objects();
        let guests = &self.guests;
        let events = self.activity.tick(now, &|player| current_name(guests, player));
        self.apply_activity(events);
        if std::mem::take(&mut self.vote_recount) {
            self.check_vote(false);
        }
        if self.vote.as_ref().is_some_and(|v| now >= v.ends) {
            self.check_vote(true);
        }
        self.tick_rotation();
        self.vote_cooldowns.retain(|_, until| now < *until);
        self.join_backoff.prune(now);
        if self.plugins.due(now) {
            let snapshot = self.plugin_snapshot();
            let actions = self.plugins.tick(now, snapshot);
            self.apply_plugin_actions(actions);
        }
    }

    // ---- Commands --------------------------------------------------------------------------
    // A SteamID64 (optionally followed by the player's session epoch, as the in-game menu
    // sends it), or the start of one connected player's name.
    fn target(&self, text: &str) -> Option<u64> {
        // Every name starts with "": a bare "kick" must not pick the only player.
        if trim(text).is_empty() {
            return None;
        }
        let (first, _) = split(text);
        if let Some(id) = number(first) {
            return self.guests.contains_key(&id).then_some(id);
        }
        let wanted = lower(text);
        let mut matched = None;
        for (&id, g) in &self.guests {
            if g.handshaken && lower(&g.member.name).starts_with(&wanted) {
                if matched.is_some() {
                    return None;
                }
                matched = Some(id);
            }
        }
        matched
    }

    fn changed(&mut self, text: String, console: bool) -> String {
        self.bans_revision += 1; // cheap: admins only get a fresh list when it moved
        self.save();
        self.roster_dirty = true;
        if !console {
            self.log(&text); // the console logs its own replies
        }
        text
    }

    // A console line, or an admin's request (`admin` = their SteamID64, 0 for the console).
    pub fn command(&mut self, line: &str, admin: u64) -> String {
        let console = admin == 0;
        let (action, argument) = split(line);
        let verb = lower(action);
        // The in-game menu's names for the same settings.
        let name = match verb.as_str() {
            "voice-allow" => "voice",
            "object-placement" => "placement",
            "world-layer-sync" => "layer-sync",
            "noclip-allow" => "noclip",
            "nobail-allow" => "nobail",
            "boosts-allow" => "boosts",
            "tuning-enforce" => "tuning",
            other => other,
        }
        .to_string();
        let no_match = |argument: &str| format!("No single connected player matches \"{argument}\".");
        match name.as_str() {
            "" | "help" => HELP_TEXT.to_string(),
            "version" => format!("ReSkate Linux Server {} (ReSkate protocol {PROTOCOL_VERSION})", crate::update::VERSION),
            "plugins" => match lower(trim(argument)).as_str() {
                "" => self.plugins.summary(),
                "reload" => {
                    let lines = self.plugins.reload();
                    if admin != 0 {
                        self.log(&format!("[admin] {} reloaded the plugins.", self.name_of(admin)));
                    }
                    lines.join("\n")
                }
                _ => "plugins [reload]".into(),
            },
            "status" => format!(
                "{} | {} | {}/{} players | {} TPS | voice {} ({} m) | password {} | code {}",
                self.config.name,
                self.map_name(),
                self.players(),
                self.config.max_players,
                self.config.tps,
                if self.voice_policy.allowed { "on" } else { "off" },
                self.config.voice_range as i32,
                if self.password.is_some() { "on" } else { "off" },
                self.invite()
            ),
            "players" => {
                let mut text = format!("{} players", self.players());
                for (&id, g) in &self.guests {
                    if g.handshaken {
                        text += &format!("\n  {id}  {}{}", guest_name(g), if self.is_admin(id) { "  (admin)" } else { "" });
                    }
                }
                text
            }
            "say" => {
                if !console {
                    return "Use chat to talk to everyone.".into();
                }
                if argument.is_empty() {
                    return "say <text>".into();
                }
                self.send_chat(argument, None);
                format!("[chat] Server: {}", clean_chat_text(argument))
            }
            "msg" | "msg-party" | "msg-admins" => {
                // Direct messages from the console or an admin, marked "[DM from ...]" so nobody
                // takes them for chat.
                let from = if console { "Server".to_string() } else { self.name_of(admin) };
                let to_admins = name == "msg-admins";
                let (who, text) = if to_admins { ("", trim(argument)) } else { split(argument) };
                if text.is_empty() {
                    return if to_admins { "msg-admins <text>".into() } else { format!("{name} <player> <text>") };
                }
                let mut recipients: Vec<u64> = Vec::new();
                let (scope, label);
                if to_admins {
                    scope = "admins";
                    label = "admins".to_string();
                    recipients = self.guests.iter().filter(|(&id, g)| g.handshaken && self.is_admin(id)).map(|(&id, _)| id).collect();
                } else {
                    let Some(player) = self.match_player(who) else {
                        return no_match(who);
                    };
                    if name == "msg" {
                        scope = "";
                        label = self.name_of(player);
                        recipients.push(player);
                    } else {
                        let Some(details) = self.parties.party(self.parties.party_of(player)) else {
                            return format!("{} is not in a party.", self.name_of(player));
                        };
                        scope = "party";
                        label = format!("{}'s party", self.name_of(player));
                        recipients = details.members.iter().copied().filter(|&m| self.handshaken(m)).collect();
                    }
                }
                if recipients.is_empty() {
                    return "No admins are online.".into();
                }
                let message = dm_line(&from, scope, text, CHAT_MAX_BYTES);
                for &id in &recipients {
                    self.send_chat(&message, Some(id));
                }
                if !console {
                    self.log(&format!("[dm] {from} -> {label}: {}", clean_chat_text(text)));
                }
                let count = if recipients.len() > 1 || to_admins { format!(" ({} players)", recipients.len()) } else { String::new() };
                format!("Sent to {label}{count}.")
            }
            "kick" => {
                let Some(id) = self.target(argument).filter(|&id| self.handshaken(id)) else {
                    return no_match(argument);
                };
                // Admins answer to the console, not to each other.
                if !console && self.is_admin(id) {
                    return "Admins cannot kick other admins.".into();
                }
                let label = self.name_of(id);
                self.kicked.insert(id);
                self.drop_guest(id, "You were kicked from this server.");
                self.changed(format!("{label} was kicked until the server restarts."), console)
            }
            "ban" => {
                let (who, reason) = split(argument);
                let guest = self.target(who);
                let id = guest.unwrap_or_else(|| number(who).unwrap_or(0));
                if !individual_steam_id(id) {
                    return "Enter a connected player or a SteamID64 (17 digits starting 7656119).".into();
                }
                if id == admin {
                    return "You cannot ban yourself.".into();
                }
                if !console && self.is_admin(id) {
                    return "Admins cannot ban other admins.".into();
                }
                if self.is_banned(id) {
                    return format!("{id} is already banned.");
                }
                let label = match guest {
                    Some(g) => self.guests[&g].member.name.clone(),
                    None => clean_chat_text(reason),
                };
                let label = prefix(&label, 64).to_string();
                let added = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                self.config.bans.push(Ban { id, name: label.clone(), added });
                if guest.is_some() {
                    self.drop_guest(id, "You were banned from this server.");
                }
                self.changed(format!("{} was banned.", if label.is_empty() { id.to_string() } else { label }), console)
            }
            "unban" => {
                let id = number(argument).unwrap_or(0);
                let Some(found) = self.config.bans.iter().position(|b| b.id == id) else {
                    return "That SteamID64 is not banned.".into();
                };
                let ban = self.config.bans.remove(found);
                let label = if ban.name.is_empty() { id.to_string() } else { ban.name };
                self.kicked.remove(&id);
                self.changed(format!("{label} was unbanned."), console)
            }
            "bans" => {
                let mut text = format!("{} banned", self.config.bans.len());
                for ban in &self.config.bans {
                    text += &format!("\n  {}  {}", ban.id, ban.name);
                }
                text
            }
            "map" => {
                // The map as it will be stored must still name a destination, or the server could
                // not tell players where to go, nor start again.
                if argument.is_empty()
                    || !valid_map_destination(&map_destination(argument))
                    || !valid_map_destination(&map_destination(&map_setting(argument)))
                {
                    return format!("No single map is called \"{argument}\". Type maps for the list.");
                }
                if map_hash(&map_destination(argument)) == self.map {
                    return "The server is already on that map.".into();
                }
                self.change_map(argument);
                self.save();
                let text = format!("Changing map to {}", self.map_name());
                self.changed(text, console)
            }
            "maps" => {
                let list = levels();
                let mut text = format!("{} maps (custom maps come from Mods next to the server)", list.len());
                for level in list {
                    let now = map_hash(&map_destination(&level.asset)) == self.map;
                    let pooled = !self.config.map_pool.is_empty() && in_map_pool(&self.config, &level.asset);
                    text += &format!("\n  {}{}{}", level.name, if now { "  (now)" } else { "" }, if pooled { "  (pool)" } else { "" });
                }
                text
            }
            "map-pool" => {
                let (what_text, map) = split(argument);
                let what = lower(what_text);
                if what.is_empty() {
                    return self.pool_text();
                }
                if what == "clear" {
                    self.config.map_pool.clear();
                    self.resend_maps();
                    return self.changed(
                        "The map pool is cleared: players vote between every map, and the rotation goes through them all.".into(),
                        console,
                    );
                }
                if what != "add" && what != "remove" {
                    return "map-pool [add|remove <map>|clear]".into();
                }
                let Some(level) = find_level(map).filter(|l| valid_map_destination(&map_destination(&l.asset))) else {
                    return format!("No single map is called \"{map}\". Type maps for the list.");
                };
                let pooled = |entry: &String| find_level(entry).is_some_and(|l| l.asset == level.asset);
                let listed = self.config.map_pool.iter().any(pooled);
                if what == "add" {
                    if listed || self.config.map_pool.is_empty() {
                        return format!("{} is already in the map pool.", level.name);
                    }
                    self.config.map_pool.push(level.name.clone());
                } else {
                    if self.config.map_pool.is_empty() {
                        // Every map: keep all the others.
                        self.config.map_pool = pool_levels(&self.config).into_iter().map(|l| l.name).collect();
                    } else if !listed {
                        return format!("{} is not in the map pool.", level.name);
                    }
                    if self.config.map_pool.iter().all(pooled) {
                        return "The map pool needs at least one map. map-pool clear allows every map again.".into();
                    }
                    self.config.map_pool.retain(|entry| !pooled(entry));
                }
                self.resend_maps();
                let text = format!("{} {} the map pool.", level.name, if what == "add" { "added to" } else { "removed from" });
                self.changed(text, console)
            }
            "rotation" => {
                if argument.is_empty() {
                    return self.rotation_text();
                }
                let value = lower(argument);
                let minutes = if value == "off" { Some(0) } else { number(&value) };
                let Some(minutes) = minutes.filter(|&m| m <= u64::from(MAX_MAP_ROTATION)) else {
                    return "rotation <1-1440 minutes>|off".into();
                };
                self.config.map_rotation = minutes as u32;
                self.map_since = self.now;
                self.rotation_warned = false;
                self.resend_maps();
                let text = self.rotation_text();
                self.changed(text, console)
            }
            "name" => {
                if !valid_server_name(argument) {
                    return format!("Server names are {SERVER_NAME_RULE}.");
                }
                self.config.name = argument.to_string();
                if contains_bad_words(&self.config.name) {
                    let text = format!(
                        "Server renamed to {}. That name contains blocked words, so the server stays out of the server browser.",
                        self.config.name
                    );
                    return self.changed(text, console);
                }
                let text = format!("Server renamed to {}.", self.config.name);
                self.changed(text, console)
            }
            "password" => {
                if argument.len() > 64 {
                    return "Passwords are at most 64 characters.".into();
                }
                self.config.password = if argument == "off" { String::new() } else { argument.to_string() };
                self.password = if self.config.password.is_empty() { None } else { password_key(&self.config.password, self.secret) };
                let text = if self.config.password.is_empty() {
                    "Password removed. Anyone can join."
                } else {
                    "Password set. Players already here stay; new ones need it."
                };
                self.changed(text.into(), console)
            }
            "welcome" => {
                if argument != "off" && !argument.is_empty() && !valid_chat_text(argument.as_bytes()) {
                    return "The welcome message is one chat line.".into();
                }
                self.config.welcome = if argument == "off" { String::new() } else { argument.to_string() };
                let text = if self.config.welcome.is_empty() { "Welcome message removed." } else { "Welcome message set." };
                self.changed(text.into(), console)
            }
            "announce-throwdowns" => {
                let Some(value) = on_off(argument) else {
                    return format!("announce-throwdowns on|off (now {})", if self.config.announce_throwdowns { "on" } else { "off" });
                };
                self.config.announce_throwdowns = value;
                let text = if value { "Placed throwdowns are announced in chat." } else { "Placed throwdowns are no longer announced." };
                self.changed(text.into(), console)
            }
            "parties" => {
                if argument.is_empty() {
                    return format!("{}{}", if self.config.parties { "Parties are on.\n" } else { "Parties are off.\n" }, self.party_status(0));
                }
                let Some(value) = on_off(argument) else { return "parties on|off".into() };
                self.config.parties = value;
                if !value {
                    let ids: Vec<u64> = self.guests.keys().copied().collect();
                    for id in ids {
                        self.parties.remove(id);
                    }
                    self.parties.take_withdrawn();
                }
                let text = if value { "Players can form parties." } else { "Parties are off; every party was ended." };
                self.changed(text.into(), console)
            }
            "speed-check" => {
                let value = lower(argument);
                if !matches!(value.as_str(), "off" | "warn" | "kick") {
                    return format!("speed-check off|warn|kick (now {})", self.config.speed_check);
                }
                self.config.speed_check = value.clone();
                if value == "off" {
                    for g in self.guests.values_mut() {
                        g.speed.restart();
                        g.speeding = false;
                    }
                }
                let text = match value.as_str() {
                    "off" => "Game speed is no longer checked.",
                    "kick" => "Players whose game runs fast are kicked.",
                    _ => "Players whose game runs fast are taken out of throwdowns and challenges.",
                };
                self.changed(text.into(), console)
            }
            "score-check" => {
                let value = lower(argument);
                if value.is_empty() {
                    let mut text = format!("score-check {} (off|warn|kick)", self.config.score_check);
                    for g in self.guests.values() {
                        if !g.handshaken {
                            continue;
                        }
                        text += &format!("\n  {}: ", guest_name(g));
                        match g.scoring {
                            None => text += "not reported",
                            Some(0) => text += "the game's own scoring",
                            Some(scoring) => {
                                text += &scoring_text(scoring);
                                if !g.scoring_mods.is_empty() {
                                    text += &format!(" ({})", g.scoring_mods);
                                }
                                text += if g.scoring_flagged { ", out of throwdowns" } else { ", allowed" };
                            }
                        }
                    }
                    return text;
                }
                if !matches!(value.as_str(), "off" | "warn" | "kick") {
                    return format!("score-check off|warn|kick (now {})", self.config.score_check);
                }
                self.config.score_check = value.clone();
                // Kicking changes the guest list: collect first.
                let ids: Vec<u64> = self.guests.keys().copied().collect();
                for id in ids {
                    if self.guests.contains_key(&id) {
                        self.check_scoring(id);
                    }
                }
                let text = match value.as_str() {
                    "off" => "Mods that change scoring or physics are no longer checked.",
                    "kick" => "Players whose mods change scoring or physics are kicked.",
                    _ => "Players whose mods change scoring or physics are taken out of throwdowns and challenges.",
                };
                self.changed(text.into(), console)
            }
            "score-allow" => {
                let (what, rest) = split(argument);
                if what.is_empty() {
                    let mut text = "score-allow <fingerprint> | score-allow remove <fingerprint>. Accepted besides the game's own:".to_string();
                    if self.config.score_allow.is_empty() {
                        text += " none";
                    }
                    for &fingerprint in &self.config.score_allow {
                        text += &format!("\n  {}", scoring_text(fingerprint));
                    }
                    return text;
                }
                let remove = lower(what) == "remove";
                let Some(fingerprint) = parse_scoring(if remove { rest } else { what }) else {
                    return "A fingerprint is 16 hex digits, as score-check lists it.".into();
                };
                let found = self.config.score_allow.iter().position(|&f| f == fingerprint);
                if remove {
                    let Some(found) = found else {
                        return format!("{} was not accepted.", scoring_text(fingerprint));
                    };
                    self.config.score_allow.remove(found);
                } else if found.is_none() {
                    self.config.score_allow.push(fingerprint);
                }
                let ids: Vec<u64> = self.guests.keys().copied().collect();
                for id in ids {
                    if self.guests.contains_key(&id) {
                        self.check_scoring(id);
                    }
                }
                let text = if remove {
                    format!("Scoring {} is no longer accepted.", scoring_text(fingerprint))
                } else {
                    format!("Scoring {} is accepted like the game's own.", scoring_text(fingerprint))
                };
                self.changed(text, console)
            }
            "party-size" => {
                let Some(value) = number(argument).filter(|v| (2..=8).contains(v)) else {
                    return format!("party-size <2-8> (now {})", self.config.party_size);
                };
                self.config.party_size = value as u32;
                self.parties.set_limit(value as usize);
                let text = format!("Parties hold up to {} players. Larger ones stay until members leave.", self.config.party_size);
                self.changed(text, console)
            }
            "activity-log" => {
                let Some(value) = on_off(argument) else {
                    return format!("activity-log on|off (now {})", if self.config.activity_log { "on" } else { "off" });
                };
                self.config.activity_log = value;
                if !value {
                    self.activity.clear();
                }
                let text = if value {
                    "Player activity (throwdowns, objects, loading) is logged."
                } else {
                    "Player activity is no longer logged."
                };
                self.changed(text.into(), console)
            }
            "listed" => {
                let Some(value) = on_off(argument) else { return "listed on|off".into() };
                self.config.listed = value;
                let text = if value { "The server is listed in the server browser." } else { "The server is hidden; players need the code." };
                self.changed(text.into(), console)
            }
            "tps" => {
                let Some(value) = number(argument).filter(|&v| v <= u64::from(u32::MAX) && valid_multiplayer_tps(v as u32)) else {
                    return "tps 20|30|60|120".into();
                };
                self.config.tps = value as u32;
                for g in self.guests.values_mut() {
                    g.pose_delivery = vec![PoseDelivery::default(); MAX_PLAYERS];
                }
                let text = format!("Network updates set to {} TPS.", self.config.tps);
                self.changed(text, console)
            }
            "voice" => {
                let Some(value) = on_off(argument) else { return "voice on|off".into() };
                self.config.voice_chat = value;
                if self.voice_policy.allowed != value {
                    self.voice_policy.allowed = value;
                    self.voice_policy.revision = self.voice_policy.revision.wrapping_add(1);
                    if self.voice_policy.revision == 0 {
                        self.voice_policy.revision = 1;
                    }
                }
                let text = if value { "Voice chat allowed." } else { "Voice chat disabled for everyone." };
                self.changed(text.into(), console)
            }
            "voice-range" => {
                let Some(range) = decimal(argument).filter(|&r| valid_voice_range(r)) else {
                    return "Choose a voice range from 50 to 1000 m.".into();
                };
                self.config.voice_range = range;
                self.changed(format!("Voice range set to {} m.", range as i32), console)
            }
            "distances" => {
                let mut values = [0i32; 4];
                let mut rest = argument;
                for value in values.iter_mut() {
                    let (token, remaining) = split(rest);
                    let Some(parsed) = integer(token) else {
                        return "distances <full> <half> <half-return> <low> (whole metres)".into();
                    };
                    *value = parsed;
                    rest = remaining;
                }
                let value = Distances { full_rate_return: values[0], half_rate_start: values[1], half_rate_return: values[2], low_rate_start: values[3] };
                if !rest.is_empty() || !value.valid() {
                    return "Use ordered distances: full < half <= half-return < low (at most 10000 m).".into();
                }
                self.config.distances = value;
                for g in self.guests.values_mut() {
                    g.pose_delivery = vec![PoseDelivery::default(); MAX_PLAYERS];
                }
                self.changed("TPS distances updated.".into(), console)
            }
            "placement" => {
                // The protocol's "host only" is admins only here: the server has no skater of its own.
                let argument = if argument == "admins" { "host" } else { argument };
                let policy = match argument {
                    "everyone" | "on" => placement::EVERYONE,
                    "host" | "off" => placement::HOST_ONLY,
                    "nobody" => placement::NOBODY,
                    "next" => match self.config.object_placement {
                        placement::EVERYONE => placement::HOST_ONLY,
                        placement::HOST_ONLY => placement::NOBODY,
                        _ => placement::EVERYONE,
                    },
                    _ => return "placement everyone|admins|nobody".into(),
                };
                self.config.object_placement = policy;
                let text = match policy {
                    placement::EVERYONE => "Everyone can place objects.",
                    placement::HOST_ONLY => "Only admins can place objects. Everyone else's are frozen.",
                    _ => "Object placement is off. Existing objects stay.",
                };
                self.changed(text.into(), console)
            }
            "votes" => self.votes_command(argument, console),
            "vote-cancel" => {
                if self.vote.is_none() {
                    return "No vote is running.".into();
                }
                self.cancel_vote(if console { "the server cancelled it" } else { "an admin cancelled it" });
                "Vote cancelled.".into()
            }
            "tpall" | "tphere" => self.teleport_command(&name, argument, admin),
            "noclip" | "nobail" | "boosts" => {
                let current = match name.as_str() {
                    "noclip" => self.config.noclip,
                    "nobail" => self.config.no_bail,
                    _ => self.config.boosts,
                };
                let value = if argument == "toggle" { Some(!current) } else { on_off(argument) };
                let Some(value) = value else {
                    return format!("{name} on|off (now {})", if current { "on" } else { "off" });
                };
                match name.as_str() {
                    "noclip" => self.config.noclip = value,
                    "nobail" => self.config.no_bail = value,
                    _ => self.config.boosts = value,
                }
                let tool = match name.as_str() {
                    "noclip" => "Noclip and teleporting",
                    "nobail" => "No Bail",
                    _ => "Boosts",
                };
                let verb = if name == "boosts" { " are" } else { " is" };
                let text = if value {
                    format!("{tool}{verb} allowed for everyone.")
                } else {
                    format!("{tool}{verb} off for players; admins keep it.")
                };
                self.changed(text, console)
            }
            "tuning" => {
                let value = if argument == "toggle" { Some(!self.config.enforce_tuning) } else { on_off(argument) };
                let Some(value) = value else {
                    return format!("tuning on|off (now {})", if self.config.enforce_tuning { "on" } else { "off" });
                };
                self.config.enforce_tuning = value;
                let text = if value {
                    "Players skate with the game's own physics tuning."
                } else {
                    "Players skate with their own physics tuning."
                };
                self.changed(text.into(), console)
            }
            "clear-objects" => {
                let mut removed = 0usize;
                for g in self.guests.values_mut() {
                    if !g.handshaken {
                        continue;
                    }
                    let ids: Vec<u64> = g.objects.objects().keys().chain(g.shared.objects().keys()).copied().collect();
                    g.cleared.extend(ids);
                    removed += g.shared.objects().len();
                    if g.shared.revision() != 0 {
                        g.shared.replace(&[]);
                    }
                    g.shared_from = g.objects.revision();
                }
                self.object_clears = self.object_clears.wrapping_add(1);
                let text = format!("Deleted {removed} placed object{}", if removed == 1 { "." } else { "s." });
                self.changed(text, console)
            }
            "park" => {
                let (lot_name, layout) = split(argument);
                let Some(index) = PARK_LOTS.iter().position(|l| l.key == lot_name) else {
                    return "park construction|historic|financial <layout, e.g. skatepark_01, or empty>".into();
                };
                if layout.is_empty() || !valid_park(index, layout) {
                    return "That is not a layout for this lot.".into();
                }
                self.config.parks[index] = layout.to_string();
                let text = format!("{} now shows {}.", PARK_LOTS[index].label, park_label(layout));
                self.changed(text, console)
            }
            "layer-sync" | "layer" | "layers" | "tod" if world_layers().is_empty() => {
                "World layers need world-layers.json next to the server (copy it from a player's \
                 %LOCALAPPDATA%\\ReSkate\\cache folder for the same game build)."
                    .into()
            }
            "layer-sync" => {
                let Some(value) = on_off(argument) else { return "layer-sync on|off".into() };
                self.config.world_layer_sync = value;
                self.apply_layers();
                let text = if value { "Everyone now follows the server's world layers." } else { "Players choose their own world layers." };
                self.changed(text.into(), console)
            }
            "layers" => {
                // Several at once, as key=mode pairs: the in-game time of day sends seven.
                let mut changes: Vec<(String, String)> = Vec::new();
                let mut rest = argument;
                while !rest.is_empty() {
                    let (pair, remaining) = split(rest);
                    rest = remaining;
                    let Some(equals) = pair.find('=') else { return "layers <key>=default|on|off ...".into() };
                    let (key, mode) = (&pair[..equals], &pair[equals + 1..]);
                    if !world_layers().iter().any(|l| l.key == key) {
                        return format!("No world layer is called \"{key}\".");
                    }
                    if !valid_world_layer_mode(mode) {
                        return "layers <key>=default|on|off ...".into();
                    }
                    changes.push((key.to_string(), mode.to_string()));
                }
                if changes.is_empty() {
                    return "layers <key>=default|on|off ...".into();
                }
                for (key, mode) in &changes {
                    if mode == "default" {
                        self.config.layers.remove(key);
                    } else {
                        self.config.layers.insert(key.clone(), mode.clone());
                    }
                }
                self.apply_layers();
                let text = format!(
                    "{} world layer{} changed{}",
                    changes.len(),
                    if changes.len() == 1 { "" } else { "s" },
                    if self.config.world_layer_sync { "." } else { ". Turn on layer-sync to apply them to everyone." }
                );
                self.changed(text, console)
            }
            "tod" => {
                // Every map's time layers ("<map>_tod_<n>_<name>"): one on and the rest off, or all
                // back to the level's own. Set for every map, so it holds across map changes.
                let wanted = lower(argument);
                let Some(found) = TIMES.iter().position(|&t| t == wanted) else {
                    return "tod default|morning|noon|afternoon|evening|night|weatherday|weathernight".into();
                };
                let slot = b'0' + found as u8;
                let mut count = 0;
                for layer in world_layers() {
                    let Some(at) = layer.key.find("_tod_") else { continue };
                    if at + 5 >= layer.key.len() {
                        continue;
                    }
                    if slot == b'0' {
                        self.config.layers.remove(&layer.key);
                    } else {
                        let mode = if layer.key.as_bytes()[at + 5] == slot { "on" } else { "off" };
                        self.config.layers.insert(layer.key.clone(), mode.into());
                    }
                    count += 1;
                }
                if count == 0 {
                    return "world-layers.json has no time-of-day layers.".into();
                }
                self.apply_layers();
                let text = format!(
                    "Time of day set to {}{}",
                    TIMES[found],
                    if self.config.world_layer_sync { " for everyone." } else { ". Turn on layer-sync to apply it to everyone." }
                );
                self.changed(text, console)
            }
            "layer" => {
                let (key, mode) = split(argument);
                let Some(found) = world_layers().iter().find(|l| l.key == key) else {
                    return format!("No world layer is called \"{key}\".");
                };
                if !valid_world_layer_mode(mode) {
                    return "layer <key> default|on|off".into();
                }
                if mode == "default" {
                    self.config.layers.remove(key);
                } else {
                    self.config.layers.insert(key.to_string(), mode.to_string());
                }
                self.apply_layers();
                let text = format!(
                    "{} set to {}{}",
                    found.label,
                    mode,
                    if self.config.world_layer_sync { "." } else { ". Turn on layer-sync to apply it to everyone." }
                );
                self.changed(text, console)
            }
            "admins" | "admin" => {
                if !console {
                    return "Only the server console manages admins.".into();
                }
                let (sub, who) = split(argument);
                if name == "admins" || sub.is_empty() {
                    let mut text = format!("{} admins", self.config.admins.len());
                    for &id in &self.config.admins {
                        text += &format!("\n  {id}");
                        if let Some(g) = self.guests.get(&id) {
                            text += &format!("  {}", guest_name(g));
                        }
                    }
                    return text;
                }
                let id = self.target(who).unwrap_or_else(|| number(who).unwrap_or(0));
                if !individual_steam_id(id) {
                    return "admin add|remove <player or SteamID64>".into();
                }
                match sub {
                    "add" => {
                        if !self.is_admin(id) {
                            self.config.admins.push(id);
                        }
                        self.resend_maps();
                        self.changed(format!("{id} is an admin."), console)
                    }
                    "remove" => {
                        self.config.admins.retain(|&a| a != id);
                        self.resend_maps();
                        self.changed(format!("{id} is no longer an admin."), console)
                    }
                    _ => "admin add|remove <player or SteamID64>".into(),
                }
            }
            _ => format!("Unknown command \"{action}\". Type help."),
        }
    }

    fn votes_command(&mut self, argument: &str, console: bool) -> String {
        // votes | votes <map|kick|tod> on|off|<percent> | votes seconds|cooldown <n>
        let (what_text, value_text) = split(argument);
        let what = lower(what_text);
        let describe = |label: &str, v: &VoteSetting| {
            format!("{label}: {}", if v.enabled { format!("on, {}% to pass", v.percent) } else { "off".into() })
        };
        if what.is_empty() {
            let votes = &self.config.votes;
            return format!(
                "{}\n{}\n{}{}\nvotes last {} s; a player waits {} s between votes{}",
                describe("map votes", &votes.map),
                describe("kick votes", &votes.kick),
                describe("time of day votes", &votes.time),
                if self.config.world_layer_sync { "" } else { " (needs layer-sync on)" },
                votes.seconds,
                votes.cooldown,
                self.vote.as_ref().map(|v| format!("\nrunning: a vote to {}", v.label)).unwrap_or_default()
            );
        }
        let value = lower(value_text);
        if what == "seconds" || what == "cooldown" {
            let seconds = what == "seconds";
            let n = number(&value).filter(|&n| if seconds { (10..=300).contains(&n) } else { n <= 3600 });
            let Some(n) = n else {
                return if seconds { "votes seconds <10-300>".into() } else { "votes cooldown <0-3600>".into() };
            };
            if seconds {
                self.config.votes.seconds = n as u32;
                return self.changed(format!("Votes now last {n} s."), console);
            }
            self.config.votes.cooldown = n as u32;
            return self.changed(format!("Players now wait {n} s between votes."), console);
        }
        let kind = match what.as_str() {
            "map" => VoteKind::Map,
            "kick" => VoteKind::Kick,
            "tod" | "time" => VoteKind::Time,
            _ => return "votes [map|kick|tod on|off|<percent>] | votes seconds <n> | votes cooldown <n>".into(),
        };
        let label = match kind {
            VoteKind::Map => "Map votes",
            VoteKind::Kick => "Kick votes",
            VoteKind::Time => "Time of day votes",
        };
        if let Some(toggle) = on_off(&value) {
            let setting = self.vote_setting_mut(kind);
            setting.enabled = toggle;
            let percent = setting.percent;
            if !toggle && self.vote.as_ref().is_some_and(|v| !self.vote_setting(v.kind).enabled) {
                self.cancel_vote("that vote was switched off");
            }
            let text = if toggle { format!("{label} are on ({percent}% to pass).") } else { format!("{label} are off.") };
            return self.changed(text, console);
        }
        let digits = value.strip_suffix('%').unwrap_or(&value);
        let Some(percent) = number(digits).filter(|p| (1..=100).contains(p)) else {
            return format!("votes {what} on|off|<1-100>");
        };
        self.vote_setting_mut(kind).percent = percent as u32;
        self.changed(format!("{label} now need {percent}% to pass."), console)
    }

    fn teleport_command(&mut self, name: &str, argument: &str, admin: u64) -> String {
        let console = admin == 0;
        // Where they go: the admin who asked, or (tpall from the console) the named player.
        let no_match = format!("No single connected player matches \"{argument}\".");
        let to: Option<u64>;
        let mut movers: Vec<u64> = Vec::new();
        if name == "tpall" {
            to = if argument.is_empty() {
                if console {
                    None
                } else {
                    self.guests.contains_key(&admin).then_some(admin)
                }
            } else {
                self.target(argument)
            };
            let Some(to_id) = to.filter(|&id| self.handshaken(id)) else {
                return if console && argument.is_empty() { "tpall <player>: everyone goes to that player.".into() } else { no_match };
            };
            movers = self.guests.iter().filter(|(&id, g)| g.handshaken && g.world_ready && id != to_id).map(|(&id, _)| id).collect();
        } else {
            if console {
                return "tphere is for admins in the game; the console can use tpall <player>.".into();
            }
            to = self.guests.contains_key(&admin).then_some(admin);
            let Some(who) = self.target(argument).filter(|&id| self.handshaken(id)) else {
                return no_match;
            };
            if Some(who) == to {
                return "That is you.".into();
            }
            movers.push(who);
        }
        let position = to.and_then(|id| self.guests[&id].latest_root);
        let Some(root) = position else {
            return format!("There is no position for {} yet.", to.map(|id| self.name_of(id)).unwrap_or_else(|| "you".into()));
        };
        let to = to.unwrap();
        if movers.is_empty() {
            return "Nobody else is in the world.".into();
        }
        let at = root.position;
        let mut sent = 0;
        for (i, &mover) in movers.iter().enumerate() {
            // A ring around them, so nobody lands inside anyone else.
            let angle = 6.2831853f32 * i as f32 / movers.len() as f32;
            let mut p = self.packet(kind::TELEPORT, self.now);
            p.teleport = [at[0] + 2.5 * angle.cos(), at[1] + 1.0, at[2] + 2.5 * angle.sin()];
            if self.send_packet(mover, &p, true, false) {
                sent += 1;
            }
        }
        let text = if movers.len() == 1 && sent != 0 {
            format!("{} was teleported to {}.", self.name_of(movers[0]), self.name_of(to))
        } else {
            format!("{sent} player(s) teleported to {}.", self.name_of(to))
        };
        if !console {
            self.log(&text);
        }
        text
    }

    // A player's mods change how tricks score: flagged players are taken out of linked
    // throwdowns and coop challenges, or kicked. The flag lasts for the session.
    fn check_scoring(&mut self, id: u64) {
        let g = &self.guests[&id];
        if !g.handshaken {
            return;
        }
        let changed = g.scoring.is_some_and(|s| s != 0 && !self.config.score_allow.contains(&s));
        let name = guest_name(g);
        if self.config.score_check == "off" || !changed {
            if !g.scoring_flagged {
                return;
            }
            guest!(self, id).scoring_flagged = false;
            self.roster_dirty = true;
            self.log(&format!("[anticheat] {name} may take part in throwdowns again (score-check {}).", self.config.score_check));
            return;
        }
        if g.scoring_flagged {
            return;
        }
        let mods = if g.scoring_mods.is_empty() { "their mods".to_string() } else { g.scoring_mods.clone() };
        let scoring = g.scoring.unwrap_or(0);
        self.log(&format!("[anticheat] {name}'s mods change scoring or physics: {mods} (scoring {}).", scoring_text(scoring)));
        if self.config.score_check == "kick" {
            return self.drop_guest(id, &format!("Your mods change scoring or physics ({mods}). Turn them off and restart Skate to play here."));
        }
        // Every player's game says so in chat when the roster flags someone.
        guest!(self, id).scoring_flagged = true;
        self.roster_dirty = true;
    }

    // A speedhack runs the player's game clock, and so their pose timestamps, faster than real
    // time. The flag clears after a minute of normal speed. False: the player was kicked.
    fn check_speed(&mut self, id: u64, sent: u64) -> bool {
        let now = self.now;
        let g = guest!(self, id);
        if self.config.speed_check == "off" || !g.handshaken || !g.world_ready || sent == 0 {
            return true;
        }
        if !g.speed.sample(sent, now) {
            return true;
        }
        let name = guest_name(g);
        let speed = g.speed.speed();
        if g.speed.flagged() && !g.speeding {
            let text = format!("{name}'s game is running at {speed:.2}x speed (a speed hack?).");
            self.log(&format!("[anticheat] {text}"));
            if self.config.speed_check == "kick" {
                self.drop_guest(id, "Your game is running faster than normal. Turn off speed hacks to play here.");
                return false;
            }
            let g = guest!(self, id);
            g.speeding = true;
            g.speed_normal_since = 0;
            self.roster_dirty = true;
            self.send_chat(
                "The server measured your game running faster than normal: throwdowns and challenges are off for you until it's back to normal speed.",
                Some(id),
            );
            let admins: Vec<u64> =
                self.guests.iter().filter(|(&other, g)| g.handshaken && other != id && self.is_admin(other)).map(|(&o, _)| o).collect();
            for admin in admins {
                self.send_chat(&format!("{text} They were taken out of throwdowns and challenges."), Some(admin));
            }
            return true;
        }
        if !g.speeding {
            return true;
        }
        if speed < SPEED_LIMIT {
            if g.speed_normal_since == 0 {
                g.speed_normal_since = now;
            }
            if now.wrapping_sub(g.speed_normal_since) >= 60_000_000 {
                g.speeding = false;
                self.roster_dirty = true;
                self.log(&format!("[anticheat] {name}'s game speed is back to normal."));
                self.send_chat("Your game speed is back to normal: throwdowns and challenges are on again.", Some(id));
            }
        } else {
            g.speed_normal_since = 0;
        }
        true
    }

    // ---- Votes (server_votes.cpp) ----------------------------------------------------------
    fn vote_setting(&self, kind: VoteKind) -> VoteSetting {
        match kind {
            VoteKind::Map => self.config.votes.map,
            VoteKind::Kick => self.config.votes.kick,
            VoteKind::Time => self.config.votes.time,
        }
    }
    fn vote_setting_mut(&mut self, kind: VoteKind) -> &mut VoteSetting {
        match kind {
            VoteKind::Map => &mut self.config.votes.map,
            VoteKind::Kick => &mut self.config.votes.kick,
            VoteKind::Time => &mut self.config.votes.time,
        }
    }
    fn enabled_votes(&self) -> u8 {
        let mut bits = 0;
        if self.config.votes.map.enabled {
            bits |= SERVER_VOTE_MAP;
        }
        if self.config.votes.kick.enabled {
            bits |= SERVER_VOTE_KICK;
        }
        // Time of day is a world layer choice: it only reaches players while layer sync is on.
        if self.config.votes.time.enabled && self.config.world_layer_sync && !world_layers().is_empty() {
            bits |= SERVER_VOTE_TIME;
        }
        bits
    }

    fn reply(&mut self, id: u64, text: &str) {
        // Chat lines are single lines: longer answers arrive a line at a time.
        let mut lines = 0;
        for line in text.split('\n') {
            if lines >= 12 {
                break;
            }
            if !trim(line).is_empty() {
                self.send_chat(line, Some(id));
                lines += 1;
            }
        }
    }

    // A SteamID64, or the start of one connected player's name.
    fn match_player(&self, text: &str) -> Option<u64> {
        let text = trim(text);
        if text.is_empty() {
            return None;
        }
        if let Some(id) = number(split(text).0) {
            return self.handshaken(id).then_some(id);
        }
        let wanted = lower(text);
        let mut matched = None;
        for (&id, g) in &self.guests {
            if g.handshaken && lower(&guest_name(g)).starts_with(&wanted) {
                if matched.is_some() {
                    return None;
                }
                matched = Some(id);
            }
        }
        matched
    }

    // ---- Map pool and rotation ----------------------------------------------------------------
    fn pool_text(&self) -> String {
        if self.config.map_pool.is_empty() {
            return "Map pool: every map".into();
        }
        let mut text = "Map pool:".to_string();
        for level in pool_levels(&self.config) {
            let now = map_hash(&map_destination(&level.asset)) == self.map;
            text += &format!("\n  {}{}", level.name, if now { "  (now)" } else { "" });
        }
        text
    }
    fn rotation_text(&self) -> String {
        if self.config.map_rotation == 0 {
            return "Map rotation is off.".into();
        }
        let every = format!("The map changes every {} min", self.config.map_rotation);
        let Some(next) = next_pool_map(&self.config, &self.config.map) else {
            return format!("{every}, but the map pool has no other map.");
        };
        if self.players() == 0 {
            return format!("{every} while players are on. Next: {}.", next.name);
        }
        let due = self.map_since + u64::from(self.config.map_rotation) * 60_000_000;
        let left = if due > self.now { (due - self.now + 59_999_999) / 60_000_000 } else { 0 };
        format!("{every}. Next: {} in about {} min.", next.name, left.max(1))
    }
    // The rotation's clock waits while nobody is on, and for a running map vote.
    fn tick_rotation(&mut self) {
        if self.config.map_rotation == 0 || self.players() == 0 {
            self.map_since = self.now;
            self.rotation_warned = false;
            return;
        }
        let due = self.map_since + u64::from(self.config.map_rotation) * 60_000_000;
        if self.now + 60_000_000 < due {
            return;
        }
        let Some(next) = next_pool_map(&self.config, &self.config.map) else {
            self.map_since = self.now;
            return;
        };
        if !self.rotation_warned && self.config.map_rotation > 1 {
            self.rotation_warned = true;
            self.send_chat(&format!("Next map in 1 minute: {}.", next.name), None);
        }
        if self.now < due || self.vote.as_ref().is_some_and(|v| v.kind == VoteKind::Map) {
            return;
        }
        self.send_chat(&format!("Changing the map to {}.", next.name), None);
        self.log(&format!("[rotation] Changing the map to {}.", next.name));
        self.change_map(&next.name);
        self.save();
    }

    fn chat_command(&mut self, id: u64, line: &str) {
        let (first, rest) = split(line);
        let verb = lower(first);
        if verb == "help" || verb == "?" {
            let wanted = lower(trim(rest)).trim_start_matches('/').to_string();
            if let Some(text) = self.plugins.describe(&wanted, self.is_admin(id)) {
                return self.reply(id, &text);
            }
        }
        if verb.is_empty() || verb == "help" || verb == "?" {
            let mut text = String::new();
            let votes = self.enabled_votes();
            if votes & SERVER_VOTE_MAP != 0 {
                text += "/vote map <map>: start a vote to change the map\n";
            }
            if votes & SERVER_VOTE_KICK != 0 {
                text += "/vote kick <player>: start a vote to kick a player\n";
            }
            if votes & SERVER_VOTE_TIME != 0 {
                text += "/vote tod <time>: vote for a time of day (morning, noon, night...)\n";
            }
            if votes != 0 {
                text += "/yes or /no: vote in the running vote\n";
            }
            if self.config.map_rotation != 0 {
                text += &format!("The map changes every {} min.\n", self.config.map_rotation);
            }
            if self.config.parties {
                text += "/party: your party (invite, accept, leave...; /party help); /p <message>: party chat\n";
            }
            let plugin_help = self.plugins.help(self.is_admin(id));
            if !plugin_help.is_empty() {
                text += &plugin_help;
                text += "\n";
            }
            if self.is_admin(id) {
                text += "Admins: any server command as /<command>, e.g. /kick, /map, /tpall, /votes, /msg, /msg-party, /msg-admins\n";
            }
            let text = if text.is_empty() { "This server has no player votes. Type /tp <player> to teleport.\n".to_string() } else { text };
            return self.reply(id, &(text + "/w <player> <message>: send a private message"));
        }
        if verb == "party" {
            return self.party_command(id, rest);
        }
        if verb == "p" {
            if !self.config.parties {
                return self.reply(id, "Parties are off on this server.");
            }
            return self.party_chat(id, rest);
        }
        if verb == "w" || verb == "whisper" || verb == "tell" {
            // A private message to one player, marked "[DM from ...]"; the sender sees an echo.
            let (who, text) = split(rest);
            if text.is_empty() {
                return self.reply(id, "/w <player> <message>, e.g. /w player hello");
            }
            let Some(other) = self.match_player(who) else {
                return self.reply(id, &format!("No single connected player matches \"{who}\"."));
            };
            if other == id {
                return self.reply(id, "You cannot message yourself.");
            }
            let message = dm_line(&self.name_of(id), "", text, CHAT_MAX_BYTES);
            self.send_chat(&message, Some(other));
            let echo = format!("[DM to {}] {}", self.name_of(other), clean_chat_text(text));
            return self.reply(id, &echo);
        }
        if verb == "yes" || verb == "y" {
            return self.cast_vote(id, true);
        }
        if verb == "no" || verb == "n" {
            return self.cast_vote(id, false);
        }
        if verb == "vote" {
            let (what_text, argument) = split(rest);
            let what = lower(what_text);
            match what.as_str() {
                "yes" | "y" => return self.cast_vote(id, true),
                "no" | "n" => return self.cast_vote(id, false),
                "map" => return self.start_vote(id, VoteKind::Map, argument),
                "kick" => return self.start_vote(id, VoteKind::Kick, argument),
                "tod" | "time" => return self.start_vote(id, VoteKind::Time, argument),
                _ => {}
            }
            if let Some(vote) = &self.vote {
                let text = format!("Running: a vote to {}. Type /yes or /no.", vote.label);
                return self.reply(id, &text);
            }
            let text = if self.enabled_votes() != 0 {
                "Start one with /vote map, /vote kick or /vote tod (see /help)."
            } else {
                "This server has no player votes."
            };
            return self.reply(id, text);
        }
        let (snapshot, player) = (self.plugin_snapshot(), self.player_info(id));
        if let Some((actions, reply)) = self.plugins.command(snapshot, &player, &verb, trim(rest)) {
            self.apply_plugin_actions(actions);
            if !reply.is_empty() && self.guests.contains_key(&id) {
                self.reply(id, &reply);
            }
            return;
        }
        // Admins run any server command from chat, as they do with "mp server".
        if self.is_admin(id) {
            self.log(&format!("[admin] {}: {}", self.name_of(id), loggable(line)));
            let answer = self.command(line, id);
            if self.guests.contains_key(&id) {
                self.reply(id, if answer.is_empty() { "Done." } else { &answer });
            }
            return;
        }
        self.reply(id, &format!("Unknown command /{verb}. Type /help for the list."));
    }

    fn start_vote(&mut self, id: u64, kind: VoteKind, argument: &str) {
        let setting = self.vote_setting(kind);
        let bit = match kind {
            VoteKind::Map => SERVER_VOTE_MAP,
            VoteKind::Kick => SERVER_VOTE_KICK,
            VoteKind::Time => SERVER_VOTE_TIME,
        };
        if self.enabled_votes() & bit == 0 {
            let text = if kind == VoteKind::Time && setting.enabled {
                "Time of day votes need world layer sync on the server.".to_string()
            } else {
                format!("{} votes are off on this server.", vote_name(kind))
            };
            return self.reply(id, &text);
        }
        if let Some(vote) = &self.vote {
            let text = format!("A vote is already running: {}. Type /yes or /no.", vote.label);
            return self.reply(id, &text);
        }
        if let Some(&wait) = self.vote_cooldowns.get(&id) {
            if self.now < wait {
                let text = format!("Wait {} s before starting another vote.", (wait - self.now) / 1_000_000 + 1);
                return self.reply(id, &text);
            }
        }
        let mut vote = Vote {
            kind,
            target: 0,
            value: String::new(),
            label: String::new(),
            yes: BTreeSet::new(),
            no: BTreeSet::new(),
            ends: 0,
            shown_yes: 0,
            shown_no: 0,
        };
        match kind {
            VoteKind::Map => {
                if argument.is_empty() {
                    return self.reply(id, "/vote map <map>, e.g. /vote map grom");
                }
                // Players vote between the server's own maps; a raw level path is admins only.
                let Some(level) = find_level(argument).filter(|_| valid_map_destination(&map_destination(argument))) else {
                    return self.reply(id, &format!("No single map is called \"{argument}\"."));
                };
                if !in_map_pool(&self.config, &level.asset) {
                    let text = format!("{} is not one of this server's maps.\n{}", level.name, self.pool_text());
                    return self.reply(id, &text);
                }
                if map_hash(&map_destination(argument)) == self.map {
                    return self.reply(id, "The server is already on that map.");
                }
                vote.value = argument.to_string();
                vote.label = format!("change the map to {}", map_label(argument));
            }
            VoteKind::Kick => {
                let Some(target) = self.match_player(argument) else {
                    return self.reply(id, &format!("No single connected player matches \"{argument}\"."));
                };
                if target == id {
                    return self.reply(id, "You cannot vote to kick yourself.");
                }
                if self.is_admin(target) {
                    return self.reply(id, "Admins cannot be kicked by a vote.");
                }
                vote.target = target;
                vote.label = format!("kick {}", self.name_of(target));
            }
            VoteKind::Time => {
                let wanted = lower(argument);
                if !TIMES.contains(&wanted.as_str()) {
                    return self.reply(id, "/vote tod <default|morning|noon|afternoon|evening|night|weatherday|weathernight>");
                }
                vote.label = format!("set the time of day to {wanted}");
                vote.value = wanted;
            }
        }
        vote.yes.insert(id); // the starter is for it
        vote.ends = self.now + u64::from(self.config.votes.seconds) * 1_000_000;
        self.vote_cooldowns.insert(id, self.now + u64::from(self.config.votes.cooldown) * 1_000_000);
        let label = vote.label.clone();
        self.vote = Some(vote);
        let name = self.name_of(id);
        self.send_chat(
            &format!(
                "{name} started a vote to {label} ({}% needed, {} s). Type /yes or /no.",
                setting.percent, self.config.votes.seconds
            ),
            None,
        );
        self.log(&format!("[vote] {name} started a vote to {label}."));
        self.check_vote(false);
    }

    fn cast_vote(&mut self, id: u64, yes: bool) {
        let Some(vote) = self.vote.as_mut() else {
            return self.reply(id, "No vote is running.");
        };
        if vote.kind == VoteKind::Kick && id == vote.target {
            return self.reply(id, "You cannot vote on your own kick.");
        }
        let changed = !(if yes { &vote.yes } else { &vote.no }).contains(&id);
        vote.yes.remove(&id);
        vote.no.remove(&id);
        if yes {
            vote.yes.insert(id);
        } else {
            vote.no.insert(id);
        }
        if !changed {
            return self.reply(id, if yes { "You already voted yes." } else { "You already voted no." });
        }
        self.check_vote(false);
    }

    fn cancel_vote(&mut self, why: &str) {
        let Some(vote) = self.vote.take() else { return };
        self.send_chat(&format!("The vote to {} was cancelled: {why}.", vote.label), None);
        self.log(&format!("[vote] The vote to {} was cancelled: {why}.", vote.label));
    }

    fn check_vote(&mut self, expired: bool) {
        let Some(vote) = self.vote.as_mut() else { return };
        // Everyone connected may vote, except the player a kick vote is about.
        let voters: BTreeSet<u64> = self.guests.iter().filter(|(&id, g)| g.handshaken && id != vote.target).map(|(&id, _)| id).collect();
        vote.yes.retain(|id| voters.contains(id));
        vote.no.retain(|id| voters.contains(id));
        if vote.kind == VoteKind::Kick && !self.guests.contains_key(&vote.target) {
            return self.cancel_vote("the player left");
        }
        let vote = self.vote.as_ref().unwrap();
        let percent = self.vote_setting(vote.kind).percent.clamp(1, 100);
        let eligible = voters.len() as u32;
        // A kick needs a second player's yes: with only the starter and the target on, it fails.
        let needed = (if vote.kind == VoteKind::Kick { 2 } else { 1 }).max((eligible * percent + 99) / 100);
        let yes = vote.yes.len() as u32;
        let no = vote.no.len() as u32;
        let passed = yes >= needed;
        let lost = !passed && (expired || yes + (eligible - eligible.min(yes + no)) < needed);
        let tally = format!("{yes} yes, {no} no, {needed} needed");
        if !passed && !lost {
            if yes != vote.shown_yes || no != vote.shown_no {
                let label = vote.label.clone();
                let vote = self.vote.as_mut().unwrap();
                vote.shown_yes = yes;
                vote.shown_no = no;
                self.send_chat(&format!("Vote to {label}: {tally}."), None);
            }
            return;
        }
        let done = self.vote.take().unwrap();
        if lost {
            self.send_chat(&format!("The vote to {} failed ({tally}).", done.label), None);
            self.log(&format!("[vote] The vote to {} failed ({tally}).", done.label));
            return;
        }
        self.send_chat(&format!("The vote to {} passed ({tally}).", done.label), None);
        self.log(&format!("[vote] The vote to {} passed ({tally}).", done.label));
        match done.kind {
            VoteKind::Map => {
                let answer = self.command(&format!("map {}", done.value), 0);
                self.log(&answer);
            }
            VoteKind::Time => {
                let answer = self.command(&format!("tod {}", done.value), 0);
                self.log(&answer);
            }
            VoteKind::Kick => {
                if self.guests.contains_key(&done.target) {
                    self.kicked.insert(done.target);
                    self.drop_guest(done.target, "You were kicked from this server by a vote.");
                }
            }
        }
    }

    // ---- Parties (server_party.cpp) --------------------------------------------------------
    fn send_party(&mut self, to: u64, action: u8, player: u64) {
        let mut notice = self.packet(kind::PARTY, self.now);
        notice.party_action = action;
        notice.party_player = player;
        self.send_packet(to, &notice, true, false);
    }

    fn party_notice(&mut self, party: u32, text: &str, except: u64) {
        let Some(details) = self.parties.party(party) else { return };
        let members = details.members.clone();
        for member in members {
            if member != except && self.handshaken(member) {
                self.send_chat(text, Some(member));
            }
        }
    }

    fn party_status(&self, id: u64) -> String {
        let name = |player: u64| self.guests.get(&player).map(|g| guest_name(g)).unwrap_or_else(|| player.to_string());
        if id != 0 {
            let Some(details) = self.parties.party(self.parties.party_of(id)) else {
                return "You're not in a party. Invite someone with /party invite <player>.".into();
            };
            let mut text = format!(
                "Your party ({}/{}{}):",
                details.members.len(),
                self.parties.limit(),
                if details.open { ", open" } else { ", invite-only" }
            );
            for (i, &member) in details.members.iter().enumerate() {
                text += if i == 0 { " " } else { ", " };
                text += &name(member);
                if member == details.leader {
                    text += " (leader)";
                }
            }
            return text;
        }
        if self.parties.parties().is_empty() {
            return "No parties.".into();
        }
        let mut lines = Vec::new();
        for (party, details) in self.parties.parties() {
            let mut text = format!("Party {party}{}", if details.open { " (open):" } else { ":" });
            for &member in &details.members {
                text += &format!(" {}{}", name(member), if member == details.leader { "*" } else { "" });
            }
            lines.push(text);
        }
        lines.join("\n")
    }

    fn party_request(&mut self, id: u64, action: u8, player: u64) {
        use party_action::*;
        if !self.config.parties {
            return self.reply(id, "Parties are off on this server.");
        }
        let my_name = self.name_of(id);
        if player != 0 && !self.handshaken(player) {
            return self.reply(id, "That player is not on the server.");
        }
        let their_name = if player != 0 { self.name_of(player) } else { String::new() };
        let now = self.now;
        let result;
        match action {
            INVITE => {
                result = self.parties.invite(id, player, now);
                if result == PartyResult::Ok {
                    self.send_party(player, INVITED, id);
                    self.reply(id, &format!("Invited {their_name} to your party."));
                    self.log(&format!("[party] {my_name} invited {their_name}"));
                }
            }
            ACCEPT | JOIN => {
                let before = self.parties.party_of(id);
                result = if action == ACCEPT { self.parties.accept(id, player, now) } else { self.parties.join(id, player, now) };
                if result == PartyResult::Ok {
                    let party = self.parties.party_of(id);
                    if before != 0 && before != party {
                        self.party_notice(before, &format!("{my_name} left the party."), 0);
                    }
                    self.party_notice(party, &format!("{my_name} joined the party."), id);
                    self.reply(id, &format!("You joined {their_name}'s party."));
                    self.log(&format!("[party] {my_name} joined {their_name}'s party"));
                } else if result == PartyResult::Closed {
                    // Ask the leader instead: they can invite.
                    let leader = self.parties.party(self.parties.party_of(player)).map(|d| d.leader);
                    if let Some(leader) = leader.filter(|l| self.guests.contains_key(l)) {
                        self.send_chat(
                            &format!(
                                "{my_name} would like to join your party. Invite them from their player card or with /party invite {my_name}"
                            ),
                            Some(leader),
                        );
                        self.reply(id, "That party is invite-only; its leader was asked to invite you.");
                        return;
                    }
                }
            }
            DECLINE => {
                result = self.parties.decline(id, player);
                if result == PartyResult::Ok {
                    self.send_chat(&format!("{my_name} declined your party invite."), Some(player));
                }
            }
            LEAVE => {
                let party = self.parties.party_of(id);
                result = self.parties.leave(id);
                if result == PartyResult::Ok {
                    self.party_notice(party, &format!("{my_name} left the party."), 0);
                    self.reply(id, "You left the party.");
                }
            }
            KICK => {
                let party = self.parties.party_of(id);
                result = self.parties.kick(id, player);
                if result == PartyResult::Ok {
                    self.send_chat("You were removed from the party.", Some(player));
                    self.party_notice(party, &format!("{their_name} was removed from the party."), 0);
                }
            }
            PROMOTE => {
                result = self.parties.promote(id, player);
                if result == PartyResult::Ok {
                    let party = self.parties.party_of(id);
                    self.party_notice(party, &format!("{their_name} now leads the party."), 0);
                }
            }
            OPEN | CLOSE => {
                let was_open = self.parties.party(self.parties.party_of(id)).is_some_and(|p| p.open);
                let open = action == OPEN;
                result = self.parties.set_open(id, open);
                if result == PartyResult::Ok && was_open != open {
                    let party = self.parties.party_of(id);
                    let text = if open { "The party is open: anyone can join." } else { "The party is invite-only." };
                    self.party_notice(party, text, 0);
                }
            }
            _ => return, // the server's own notices
        }
        if result != PartyResult::Ok {
            let text = match result {
                PartyResult::Ok => "",
                PartyResult::SelfTarget => "That's you.",
                PartyResult::SameParty => "You're already in a party together.",
                PartyResult::Full => "That party is full.",
                PartyResult::NotLeader => "Only the party leader can do that.",
                PartyResult::NotMember => "You're not in a party with them.",
                PartyResult::NoInvite => "That invite has expired.",
                PartyResult::Closed => "That party is invite-only.",
                PartyResult::NoParty => "They're not in a party.",
                PartyResult::Busy => "They already have too many party invites.",
                PartyResult::Renewed => "They already have your party invite; it stays open for another minute.",
            };
            self.reply(id, text);
        }
        if self.parties.revision() != self.party_revision {
            self.roster_dirty = true;
        }
    }

    fn party_left(&mut self, id: u64, name: &str) {
        let party = self.parties.party_of(id);
        self.parties.remove(id);
        if party != 0 {
            self.party_notice(party, &format!("{name} left the server."), 0);
        }
    }

    fn tick_parties(&mut self) {
        for lapsed in self.parties.expire(self.now) {
            if self.guests.contains_key(&lapsed.to) {
                self.send_party(lapsed.to, party_action::WITHDRAWN, lapsed.from);
            }
            if self.handshaken(lapsed.from) && self.guests.contains_key(&lapsed.to) {
                let text = format!("Your party invite to {} expired.", self.name_of(lapsed.to));
                self.send_chat(&text, Some(lapsed.from));
            }
        }
        for withdrawn in self.parties.take_withdrawn() {
            if self.guests.contains_key(&withdrawn.to) {
                self.send_party(withdrawn.to, party_action::WITHDRAWN, withdrawn.from);
            }
        }
        if self.parties.revision() != self.party_revision {
            self.party_revision = self.parties.revision();
            self.roster_dirty = true;
        }
    }

    fn party_command(&mut self, id: u64, line: &str) {
        use party_action::*;
        if !self.config.parties {
            return self.reply(id, "Parties are off on this server.");
        }
        let (first, rest) = split(line);
        let verb = lower(first);
        let target = |host: &mut Host| -> u64 {
            let found = host.match_player(rest);
            if found.is_none() {
                let text = if rest.is_empty() { "Name a player.".to_string() } else { format!("No single player matches \"{rest}\".") };
                host.reply(id, &text);
            }
            found.unwrap_or(0)
        };
        match verb.as_str() {
            "" | "status" | "list" => {
                let text = self.party_status(id);
                self.reply(id, &text)
            }
            "help" | "?" => self.reply(
                id,
                "/party invite|join|kick|promote <player>, /party accept|decline [player], /party leave, /party open|close, /p <message> talks to your party",
            ),
            "invite" | "join" | "kick" | "remove" | "promote" | "lead" | "leader" => {
                let action = match verb.as_str() {
                    "invite" => INVITE,
                    "join" => JOIN,
                    "kick" | "remove" => KICK,
                    _ => PROMOTE,
                };
                let player = target(self);
                if player != 0 {
                    self.party_request(id, action, player);
                }
            }
            "accept" | "decline" => {
                let player = if rest.is_empty() {
                    // The newest invite this player holds.
                    let mut from = 0;
                    let mut latest = 0;
                    for invite in self.parties.invites() {
                        if invite.to == id && invite.expires >= latest {
                            from = invite.from;
                            latest = invite.expires;
                        }
                    }
                    if from == 0 {
                        self.reply(id, "You have no party invites.");
                    }
                    from
                } else {
                    target(self)
                };
                if player != 0 {
                    self.party_request(id, if verb == "accept" { ACCEPT } else { DECLINE }, player);
                }
            }
            "leave" => self.party_request(id, LEAVE, 0),
            "open" => self.party_request(id, OPEN, 0),
            "close" => self.party_request(id, CLOSE, 0),
            "say" | "chat" => self.party_chat(id, rest),
            _ => self.reply(id, "Unknown party command. Type /party help."),
        }
    }

    fn party_chat(&mut self, id: u64, text: &str) {
        let party = self.parties.party_of(id);
        if party == 0 {
            return self.reply(id, "You're not in a party.");
        }
        let line = clean_chat_text(text);
        if line.is_empty() {
            return self.reply(id, "/p <message>");
        }
        // Relayed as the sender's own chat line, only to the rest of their party.
        let mut message = self.packet(kind::CHAT, self.now);
        message.source = id;
        message.epoch = self.guests[&id].member.epoch;
        message.text = clean_chat_text(&format!("[Party] {line}"));
        self.log(&format!("[party chat] {}: {line}", self.name_of(id)));
        let members = self.parties.party(party).map(|p| p.members.clone()).unwrap_or_default();
        for member in members {
            if member != id && self.handshaken(member) {
                self.send_packet(member, &message, true, false);
            }
        }
    }
}
