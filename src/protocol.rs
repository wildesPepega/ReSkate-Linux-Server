// The ReSkate multiplayer wire protocol (Extension/Multiplayer/Net/protocol.{h,cpp} and the
// session model headers it uses). Byte for byte what the game speaks: protocol version 38.
use crate::world::{park_id, world_layers, ParkChoices, PARK_FAMILIES, PARK_LOTS, WORLD_LAYER_MODES};

pub const MAX_SKATER_BONES: usize = 512;
pub const MAX_BOARD_BONES: usize = 64;
pub const MAX_PACKET: usize = 24576;
pub const PACKET_HEADER_SIZE: usize = 64;
pub const PROTOCOL_VERSION: u16 = 38;
pub const MAX_THROWDOWN_MESSAGE: usize = 4096;
pub const MAX_PHYSICS_TUNING: usize = 16384;
pub const SERVER_VOTE_MAP: u8 = 1;
pub const SERVER_VOTE_KICK: u8 = 2;
pub const SERVER_VOTE_TIME: u8 = 4;

// Everyone a session can hold: the wire protocol's limit, reached by dedicated servers.
pub const MAX_PLAYERS: usize = 250;
pub const MAX_REMOTE_PLAYERS: usize = MAX_PLAYERS - 1;

pub const MAX_MEMBER_NAME: usize = 64;
pub const MAX_ADMIN_TEXT: usize = 320;
pub const MAX_BAN_ROWS: usize = 256;
pub const MAX_SERVER_MAPS: usize = 128;
pub const MAX_MAP_ASSET: usize = 128;
pub const CHAT_MAX_BYTES: usize = 200;

// Cosmetics (Remote/cosmetics.h).
pub const SKATER_RECIPE_KEY: u32 = 2759515148;
pub const BOARD_RECIPE_KEY: u32 = 1583459055;
pub const MAX_COSMETIC_SLOTS: usize = 64;
pub const MAX_COSMETIC_SCALARS: usize = 64;
pub const MAX_COSMETIC_PARAMETERS: usize = 128;
pub const MAX_COSMETIC_ASSET: usize = 255;

// Audio (Remote/audio_state.h).
pub const AUDIO_FLOAT_COUNT: usize = 58;
pub const AUDIO_SELECTOR_COUNT: usize = 26;
pub const AUDIO_FLAG_COUNT: usize = 43;
pub const MAX_AUDIO_SAMPLES: usize = 32;

// Voice (Voice/voice_state.h, voice_settings.h).
pub const MAX_VOICE_BYTES: usize = 16384;
pub const MAX_VOICE_VOLUME: f32 = 10.0;
pub const MIN_HEARING_DISTANCE: f32 = 5.0;
pub const MAX_HEARING_DISTANCE: f32 = 1000.0;
pub const MIN_VOICE_RANGE: f32 = 50.0;
pub const MAX_VOICE_RANGE: f32 = 1000.0;
pub const DEFAULT_VOICE_RANGE: f32 = 300.0;

// Objects (Session/object_state.h).
pub const MAX_OWNED_OBJECTS: usize = 1024;
pub const OBJECT_CHUNK_ENTRIES: usize = 64;
pub const MAX_OBJECT_PARTS: u16 = 32;

// Tick settings.
pub const TICK_RATES: [u32; 4] = [20, 30, 60, 120];
pub const DEFAULT_TPS: u32 = 30;

pub fn valid_multiplayer_tps(tps: u32) -> bool {
    TICK_RATES.contains(&tps)
}
pub fn multiplayer_pose_interval(tps: u32) -> u32 {
    1_000_000 / if valid_multiplayer_tps(tps) { tps } else { DEFAULT_TPS }
}
pub fn valid_pose_interval(interval: u32) -> bool {
    interval == 100_000 || interval == 200_000 || TICK_RATES.iter().any(|&r| interval == multiplayer_pose_interval(r))
}

pub mod kind {
    pub const HELLO: u16 = 1;
    pub const WELCOME: u16 = 2;
    pub const POSE: u16 = 3;
    pub const AWAY: u16 = 4;
    pub const COSMETICS: u16 = 5;
    pub const AUDIO: u16 = 6;
    pub const ROSTER: u16 = 7;
    pub const CHALLENGE: u16 = 11;
    pub const ROUTES: u16 = 12;
    pub const PEER_HELLO: u16 = 13;
    pub const PEER_WELCOME: u16 = 14;
    pub const MAP_REQUEST: u16 = 15;
    pub const MAP_OFFER: u16 = 16;
    pub const WORLD_STATE: u16 = 17;
    pub const WORLD_READY: u16 = 18;
    pub const OBJECTS: u16 = 19;
    pub const VOICE: u16 = 20;
    pub const CHAT: u16 = 21;
    pub const ADMIN: u16 = 22;
    pub const BANS: u16 = 23;
    pub const MAPS: u16 = 24;
    pub const THROWDOWN: u16 = 25;
    pub const TELEPORT: u16 = 26;
    pub const PHYSICS_TUNING: u16 = 27;
    pub const PARTY: u16 = 28;
    pub const SCORING: u16 = 29;
}

// Packet::party_action (see PartyAction in protocol.h).
pub mod party_action {
    pub const INVITE: u8 = 1;
    pub const ACCEPT: u8 = 2;
    pub const DECLINE: u8 = 3;
    pub const JOIN: u8 = 4;
    pub const LEAVE: u8 = 5;
    pub const KICK: u8 = 6;
    pub const PROMOTE: u8 = 7;
    pub const OPEN: u8 = 8;
    pub const CLOSE: u8 = 9;
    pub const INVITED: u8 = 10;
    pub const WITHDRAWN: u8 = 11;
}

// Object placement policy; wire values 0/1/2.
pub mod placement {
    pub const HOST_ONLY: u8 = 0;
    pub const EVERYONE: u8 = 1;
    pub const NOBODY: u8 = 2;
}

pub fn valid_party_request(action: u8, player: u64) -> bool {
    use party_action::*;
    match action {
        INVITE | ACCEPT | DECLINE | JOIN | KICK | PROMOTE | INVITED | WITHDRAWN => individual_steam_id(player),
        LEAVE | OPEN | CLOSE => player == 0,
        _ => false,
    }
}

// Steam accounts in the public universe. Players are individual accounts; a dedicated server
// signs in anonymously as a game server (type 3 or 4) and gets a new ID each time it starts.
pub fn individual_steam_id(id: u64) -> bool {
    (id >> 56) == 1 && ((id >> 52) & 15) == 1 && (id & 0xffff_ffff) != 0
}
pub fn game_server_steam_id(id: u64) -> bool {
    let kind = (id >> 52) & 15;
    (id >> 56) == 1 && (kind == 3 || kind == 4) && (id & 0xffff_ffff) != 0
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Transform { position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
    }
}

#[derive(Clone, Default, Debug)]
pub struct Pose {
    pub root: Transform,
    pub skater: Vec<Transform>,
    pub board: Vec<Transform>,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Member {
    pub id: u64,
    pub epoch: u64,
    pub name: String,
    pub admin: bool,
    pub party: u32,
    pub party_leader: bool,
    pub party_open: bool,
    pub speeding: bool,
    pub scoring: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct CosmeticSlot {
    pub slot: u32,
    pub asset: Vec<u8>,
    pub parameters: Vec<u32>,
}
#[derive(Clone, Default, PartialEq, Debug)]
pub struct CosmeticRecipe {
    pub key: u32,
    pub version: u32,
    pub scalars: Vec<u32>,
    pub items: Vec<CosmeticSlot>,
}
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct PlayerCard {
    pub background: u32,
    pub emblem: u32,
    pub title: u32,
}
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Appearance {
    pub skater: CosmeticRecipe,
    pub board: CosmeticRecipe,
    pub card: PlayerCard,
}

#[derive(Clone, PartialEq, Debug)]
pub struct AudioState {
    pub values: [f32; AUDIO_FLOAT_COUNT],
    pub selectors: [u32; AUDIO_SELECTOR_COUNT],
    pub flags: [u8; AUDIO_FLAG_COUNT],
}
impl Default for AudioState {
    fn default() -> Self {
        AudioState { values: [0.0; AUDIO_FLOAT_COUNT], selectors: [0; AUDIO_SELECTOR_COUNT], flags: [0; AUDIO_FLAG_COUNT] }
    }
}
#[derive(Clone, Default, Debug)]
pub struct AudioSample {
    pub age_us: u32,
    pub state: AudioState,
    pub event: bool,
}

pub fn valid_audio(s: &AudioState) -> bool {
    s.values.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0) && s.flags.iter().all(|&v| v <= 1) && s.flags[0] == 0
}
pub fn valid_audio_batch(samples: &[AudioSample]) -> bool {
    if samples.is_empty() || samples.len() > MAX_AUDIO_SAMPLES {
        return false;
    }
    let mut previous: u32 = 1_000_000;
    for s in samples {
        if s.age_us > previous || !valid_audio(&s.state) {
            return false;
        }
        previous = s.age_us;
    }
    true
}

#[derive(Clone, Debug)]
pub struct VoiceData {
    pub distance: f32,
    pub gain: f32,
    pub policy_revision: u32,
    pub bytes: Vec<u8>,
}
impl Default for VoiceData {
    fn default() -> Self {
        VoiceData { distance: 35.0, gain: 1.0, policy_revision: 1, bytes: Vec::new() }
    }
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VoicePolicy {
    pub allowed: bool,
    pub revision: u32,
}
impl Default for VoicePolicy {
    fn default() -> Self {
        VoicePolicy { allowed: true, revision: 1 }
    }
}
impl VoicePolicy {
    pub fn accepts(&self, voice: &VoiceData) -> bool {
        self.allowed && self.revision != 0 && voice.policy_revision == self.revision
    }
}
pub fn valid_voice_volume(value: f32) -> bool {
    value.is_finite() && (0.0..=MAX_VOICE_VOLUME).contains(&value)
}
pub fn valid_voice_range(value: f32) -> bool {
    value.is_finite() && (MIN_VOICE_RANGE..=MAX_VOICE_RANGE).contains(&value)
}
pub fn valid_voice(voice: &VoiceData) -> bool {
    !voice.bytes.is_empty()
        && voice.bytes.len() <= MAX_VOICE_BYTES
        && voice.policy_revision != 0
        && valid_voice_volume(voice.gain)
        && voice.distance.is_finite()
        && (voice.distance == 0.0 || (voice.distance >= MIN_HEARING_DISTANCE && voice.distance <= MAX_HEARING_DISTANCE))
}
#[derive(Clone, Default)]
pub struct VoiceBudget {
    since: u64,
    bytes: u64,
    packets: u64,
}
impl VoiceBudget {
    pub fn accept(&mut self, now: u64, size: usize) -> bool {
        if now < self.since || now - self.since >= 1_000_000 {
            self.since = now;
            self.bytes = 0;
            self.packets = 0;
        }
        if size == 0 || size > MAX_VOICE_BYTES || self.packets >= 80 || self.bytes + size as u64 > 96 * 1024 {
            return false;
        }
        self.packets += 1;
        self.bytes += size as u64;
        true
    }
}
// How loud a speaker is at a distance, for a listener who hears out to `radius`.
pub fn voice_gain(listener: &[f32; 3], speaker: &[f32; 3], radius: f32) -> f32 {
    let mut squared = 0.0f32;
    for i in 0..3 {
        let d = listener[i] - speaker[i];
        squared += d * d;
    }
    if !squared.is_finite() || !radius.is_finite() || radius < 0.0 {
        return 0.0;
    }
    if radius == 0.0 {
        return 1.0;
    }
    let distance = squared.sqrt();
    if distance >= radius {
        return 0.0;
    }
    let full_volume = 3.0f32.max(radius * 0.15);
    if distance <= full_volume {
        return 1.0;
    }
    let mut gain = full_volume / distance;
    let fade_start = radius * 0.75;
    if distance > fade_start {
        let t = (distance - fade_start) / (radius - fade_start);
        gain *= 1.0 - t * t * (3.0 - 2.0 * t);
    }
    gain
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Distances {
    pub full_rate_return: i32,
    pub half_rate_start: i32,
    pub half_rate_return: i32,
    pub low_rate_start: i32,
}
impl Default for Distances {
    fn default() -> Self {
        Distances { full_rate_return: 50, half_rate_start: 60, half_rate_return: 150, low_rate_start: 170 }
    }
}
impl Distances {
    pub fn valid(&self) -> bool {
        self.full_rate_return >= 0
            && self.full_rate_return < self.half_rate_start
            && self.half_rate_start <= self.half_rate_return
            && self.half_rate_return < self.low_rate_start
            && self.low_rate_start <= 10000
    }
}

#[derive(Clone, Default, Debug)]
pub struct Ban {
    pub id: u64,
    pub name: String,
    pub added: i64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct NetworkObject {
    pub id: u64,
    pub item: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: f32,
}
impl Default for NetworkObject {
    fn default() -> Self {
        NetworkObject { id: 0, item: String::new(), position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0 }
    }
}

#[derive(Clone, Debug)]
pub struct ObjectChunk {
    pub base: u64,
    pub revision: u64,
    pub part: u16,
    pub parts: u16,
    pub objects: Vec<NetworkObject>,
    pub removed: Vec<u64>,
}
impl Default for ObjectChunk {
    fn default() -> Self {
        ObjectChunk { base: 0, revision: 0, part: 0, parts: 1, objects: Vec::new(), removed: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct Packet {
    pub kind: u16,
    pub sequence: u32,
    pub session: u64,
    pub map: u64,
    pub epoch: u64,
    pub time_us: u64,
    pub source: u64,
    pub world: u64,
    pub build: [u8; 32],
    pub proof: [u8; 32],
    pub challenge: u64,
    pub destination: String,
    pub map_authorized: bool,
    pub world_ready: bool,
    pub pose: Pose,
    pub pose_interval_us: u32,
    pub player_collision: bool,
    pub appearance: Appearance,
    pub audio: Vec<AudioSample>,
    pub voice: VoiceData,
    pub voice_policy: VoicePolicy,
    pub capacity: u32,
    pub members: Vec<Member>,
    pub distances: Distances,
    pub tps: u32,
    pub object_placement: u8,
    pub object_clears: u32,
    pub force_world_layers: bool,
    pub layers: Vec<u8>,
    pub parks: ParkChoices,
    pub objects: ObjectChunk,
    pub text: String,
    pub voice_range: f32,
    pub guest_noclip: bool,
    pub guest_no_bail: bool,
    pub guest_boosts: bool,
    pub enforce_tuning: bool,
    pub server_votes: u8,
    pub bans: Vec<Ban>,
    pub ban_total: u32,
    pub maps: Vec<String>,
    pub throwdown: Vec<u8>,
    pub teleport: [f32; 3],
    pub tuning: Vec<u8>,
    pub party_action: u8,
    pub party_player: u64,
    pub scoring: u64,
}
impl Default for Packet {
    fn default() -> Self {
        Packet {
            kind: kind::POSE,
            sequence: 0,
            session: 0,
            map: 0,
            epoch: 0,
            time_us: 0,
            source: 0,
            world: 1,
            build: [0; 32],
            proof: [0; 32],
            challenge: 0,
            destination: String::new(),
            map_authorized: false,
            world_ready: false,
            pose: Pose::default(),
            pose_interval_us: 50000,
            player_collision: false,
            appearance: Appearance::default(),
            audio: Vec::new(),
            voice: VoiceData::default(),
            voice_policy: VoicePolicy::default(),
            capacity: MAX_PLAYERS as u32,
            members: Vec::new(),
            distances: Distances::default(),
            tps: DEFAULT_TPS,
            object_placement: placement::EVERYONE,
            object_clears: 0,
            force_world_layers: false,
            layers: vec![0; world_layers().len()],
            parks: Default::default(),
            objects: ObjectChunk::default(),
            text: String::new(),
            voice_range: DEFAULT_VOICE_RANGE,
            guest_noclip: true,
            guest_no_bail: true,
            guest_boosts: true,
            enforce_tuning: true,
            server_votes: 0,
            bans: Vec::new(),
            ban_total: 0,
            maps: Vec::new(),
            throwdown: Vec::new(),
            teleport: [0.0; 3],
            tuning: Vec::new(),
            party_action: party_action::LEAVE,
            party_player: 0,
            scoring: 0,
        }
    }
}

// ---- Validation ------------------------------------------------------------------------------
pub fn valid_transform(t: &Transform) -> bool {
    if !t.position.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0) {
        return false;
    }
    if !t.scale.iter().all(|&v| v.is_finite() && (0.0001..=100.0).contains(&v)) {
        return false;
    }
    let mut norm = 0.0f32;
    for &v in &t.rotation {
        if !v.is_finite() {
            return false;
        }
        norm += v * v;
    }
    norm > 0.8 && norm < 1.2
}

pub fn valid_pose(p: &Pose) -> bool {
    valid_transform(&p.root)
        && p.skater.len() <= MAX_SKATER_BONES
        && p.board.len() <= MAX_BOARD_BONES
        && p.skater.iter().all(valid_transform)
        && p.board.iter().all(valid_transform)
}

pub fn valid_roster(members: &[Member], capacity: u32) -> bool {
    if capacity < 2 || capacity as usize > MAX_PLAYERS || members.is_empty() || members.len() > capacity as usize {
        return false;
    }
    for (i, m) in members.iter().enumerate() {
        // The host comes first, and may be a dedicated server rather than a player.
        let server = i == 0 && game_server_steam_id(m.id);
        let identity = individual_steam_id(m.id) || server;
        if !identity || m.epoch == 0 || m.name.len() > 128 || (i == 0 && m.admin) {
            return false;
        }
        // A server is in no party; only a party's leader leads or opens it.
        if (server && m.party != 0) || (m.party == 0 && (m.party_leader || m.party_open)) || (m.party_open && !m.party_leader) {
            return false;
        }
        if m.name.bytes().any(|c| c < 32 || c == 127) {
            return false;
        }
        let mut leaders = u32::from(m.party_leader);
        let mut size = 1u32;
        for (j, other) in members.iter().enumerate() {
            if j < i && other.id == m.id {
                return false;
            }
            if j != i && m.party != 0 && other.party == m.party {
                leaders += u32::from(other.party_leader);
                size += 1;
            }
        }
        // Each party has one leader and at least one other member.
        if m.party != 0 && (leaders != 1 || size < 2) {
            return false;
        }
    }
    true
}

pub fn valid_routes(members: &[Member]) -> bool {
    if members.len() > MAX_REMOTE_PLAYERS {
        return false;
    }
    for (i, m) in members.iter().enumerate() {
        if !individual_steam_id(m.id) || m.epoch == 0 || !m.name.is_empty() || m.admin {
            return false;
        }
        if members[..i].iter().any(|o| o.id == m.id) {
            return false;
        }
    }
    true
}

pub fn valid_appearance(a: &Appearance) -> bool {
    if a.skater.key != SKATER_RECIPE_KEY || a.skater.version != 2 || a.board.key != BOARD_RECIPE_KEY || a.board.version != 1 {
        return false;
    }
    let mut size = PACKET_HEADER_SIZE + 12;
    for r in [&a.skater, &a.board] {
        if r.scalars.len() > MAX_COSMETIC_SCALARS || r.items.is_empty() || r.items.len() > MAX_COSMETIC_SLOTS {
            return false;
        }
        size += 12 + r.scalars.len() * 4;
        for (i, item) in r.items.iter().enumerate() {
            if item.slot == 0 || item.asset.len() > MAX_COSMETIC_ASSET || item.parameters.len() > MAX_COSMETIC_PARAMETERS {
                return false;
            }
            if item.asset.iter().any(|&c| c < 32 || c == 127) {
                return false;
            }
            if r.items[..i].iter().any(|o| o.slot == item.slot) {
                return false;
            }
            size += 8 + item.asset.len() + item.parameters.len() * 4;
        }
    }
    size <= MAX_PACKET
}

pub fn valid_network_object(object: &NetworkObject) -> bool {
    if object.id == 0
        || object.item.len() > 256
        || !object.item.starts_with("own_bk")
        || !object.item.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return false;
    }
    if !object.position.iter().all(|v| v.is_finite() && v.abs() <= 100000.0) {
        return false;
    }
    if !object.scale.is_finite() || object.scale < 0.01 || object.scale > 100.0 {
        return false;
    }
    let mut norm = 0.0f32;
    for &v in &object.rotation {
        if !v.is_finite() {
            return false;
        }
        norm += v * v;
    }
    norm.is_finite() && norm > 0.98 && norm < 1.02
}

pub fn valid_object_chunk(chunk: &ObjectChunk) -> bool {
    if chunk.revision == 0
        || chunk.base >= chunk.revision
        || chunk.parts == 0
        || chunk.parts > MAX_OBJECT_PARTS
        || chunk.part >= chunk.parts
        || chunk.objects.len() + chunk.removed.len() > OBJECT_CHUNK_ENTRIES
        || (chunk.base == 0 && !chunk.removed.is_empty())
    {
        return false;
    }
    let mut ids = std::collections::BTreeSet::new();
    for object in &chunk.objects {
        if !valid_network_object(object) || !ids.insert(object.id) {
            return false;
        }
    }
    for &id in &chunk.removed {
        if id == 0 || !ids.insert(id) {
            return false;
        }
    }
    true
}

pub fn valid_map_destination(value: &str) -> bool {
    let bytes = value.as_bytes();
    match value.find('|') {
        Some(split) => {
            !value.is_empty()
                && value.len() <= 256
                && split > 0
                && !value[split + 1..].contains('|')
                && !bytes.iter().any(|&c| c < 32 || c == 127)
        }
        None => false,
    }
}

// A level asset as a server's map list carries it: printable ASCII, no '|'.
pub fn valid_map_asset(asset: &str) -> bool {
    !asset.is_empty() && asset.len() <= MAX_MAP_ASSET && asset.bytes().all(|c| c > 32 && c < 127 && c != b'|')
}

// Length of the UTF-8 sequence starting at `at`, 0 when it is malformed, overlong, a surrogate
// or past U+10FFFF.
fn utf8_length(text: &[u8], at: usize) -> usize {
    let lead = text[at] as u32;
    if lead < 0x80 {
        return 1;
    }
    let (length, mut code, minimum) = if (lead & 0xE0) == 0xC0 {
        (2, lead & 0x1F, 0x80)
    } else if (lead & 0xF0) == 0xE0 {
        (3, lead & 0x0F, 0x800)
    } else if (lead & 0xF8) == 0xF0 {
        (4, lead & 0x07, 0x10000)
    } else {
        return 0;
    };
    if at + length > text.len() {
        return 0;
    }
    for i in 1..length {
        let byte = text[at + i] as u32;
        if (byte & 0xC0) != 0x80 {
            return 0;
        }
        code = (code << 6) | (byte & 0x3F);
    }
    if code < minimum || code > 0x10FFFF || (0xD800..=0xDFFF).contains(&code) {
        return 0;
    }
    length
}
fn chat_control(text: &[u8], at: usize, length: usize) -> bool {
    let lead = text[at];
    // C0 controls and DEL, and the C1 range (U+0080..U+009F, encoded C2 80..C2 9F).
    (length == 1 && (lead < 0x20 || lead == 0x7F)) || (length == 2 && lead == 0xC2 && text[at + 1] < 0xA0)
}

// A chat message: 1..200 bytes of UTF-8 with no control characters and something visible.
pub fn valid_chat_text(text: &[u8]) -> bool {
    if text.is_empty() || text.len() > CHAT_MAX_BYTES {
        return false;
    }
    let mut visible = false;
    let mut at = 0;
    while at < text.len() {
        let length = utf8_length(text, at);
        if length == 0 || chat_control(text, at, length) {
            return false;
        }
        visible |= text[at] != b' ';
        at += length;
    }
    visible
}

fn valid_line(text: &[u8]) -> bool {
    let mut at = 0;
    while at < text.len() {
        let length = utf8_length(text, at);
        if length == 0 || chat_control(text, at, length) {
            return false;
        }
        at += length;
    }
    true
}

// A player name in a hello or roster: UTF-8 without control characters.
pub fn valid_member_name(text: &[u8]) -> bool {
    text.len() <= MAX_MEMBER_NAME && valid_line(text)
}

pub fn valid_admin_text(text: &[u8]) -> bool {
    !text.is_empty() && text.len() <= MAX_ADMIN_TEXT && valid_line(text)
}

// The message a player typed, made valid: control characters and broken UTF-8 dropped,
// surrounding blanks trimmed, cut to the byte limit on a character boundary.
pub fn clean_chat_text(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut result: Vec<u8> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let length = utf8_length(bytes, at);
        if length == 0 {
            at += 1;
            continue;
        }
        if chat_control(bytes, at, length) {
            // Tabs and line breaks read as a space; other controls vanish.
            if matches!(bytes[at], b'\t' | b'\n' | b'\r') {
                result.push(b' ');
            }
            at += length;
            continue;
        }
        if result.len() + length > CHAT_MAX_BYTES {
            break;
        }
        result.extend_from_slice(&bytes[at..at + length]);
        at += length;
    }
    let text = String::from_utf8(result).unwrap_or_default();
    text.trim_matches(' ').to_string()
}

pub fn newer_sequence(a: u32, b: u32) -> bool {
    let d = a.wrapping_sub(b);
    d != 0 && d < 0x8000_0000
}

pub fn map_hash(value: &str) -> u64 {
    let mut result: u64 = 14695981039346656037;
    for &byte in value.as_bytes() {
        let mut c = byte;
        if c.is_ascii_uppercase() {
            c += b'a' - b'A';
        }
        if c == b'\\' {
            c = b'/';
        }
        result = (result ^ u64::from(c)).wrapping_mul(1099511628211);
    }
    result
}

// Empty means a compatible greeting.
pub fn greeting_error(p: &Packet, session: u64, map: u64, build: &[u8; 32], peer_epoch: u64) -> &'static str {
    if !matches!(p.kind, kind::HELLO | kind::WELCOME | kind::CHALLENGE | kind::PEER_HELLO | kind::PEER_WELCOME)
        || p.session == 0
        || p.epoch == 0
        || p.session != session
    {
        return "Join code or multiplayer protocol did not match.";
    }
    if &p.build != build {
        return "Both players need the same supported game build.";
    }
    if map == 0 || p.map != map {
        return "Map mismatch. Load the same map on both machines, then join again.";
    }
    if peer_epoch != 0 && p.epoch != peer_epoch {
        return "Peer changed level or session. Join again.";
    }
    ""
}

pub fn format_invite(steam_id: u64, secret: u64) -> String {
    format!("{steam_id}-{secret:016x}")
}

// ---- Encoding --------------------------------------------------------------------------------
const MAGIC: u32 = 0x3150_4d52; // RMP1, little endian

fn encode_park(lot: usize, choice: &str) -> u8 {
    if choice.is_empty() || choice == "empty" {
        return 0;
    }
    for family in 0..PARK_FAMILIES.len() {
        for variant in 1..=PARK_LOTS[lot].counts[family] {
            if choice == park_id(family, variant) {
                return (1 + family as u32 * 16 + variant - 1) as u8;
            }
        }
    }
    panic!("Invalid park selection");
}
fn decode_park(lot: usize, code: u32) -> Option<String> {
    if code == 0 {
        return Some("empty".into());
    }
    let family = ((code - 1) / 16) as usize;
    let variant = (code - 1) % 16 + 1;
    if family >= PARK_FAMILIES.len() || variant > PARK_LOTS[lot].counts[family] {
        return None;
    }
    Some(park_id(family, variant))
}

// std::lround on a float, then the C++ narrowing to int16_t (modular).
fn round_i16(value: f32) -> u16 {
    (value.round() as i64) as i16 as u16
}

struct Writer {
    bytes: Vec<u8>,
}
impl Writer {
    fn int(&mut self, value: u64, width: usize) {
        for i in 0..width {
            self.bytes.push((value >> (8 * i)) as u8);
        }
    }
    fn float(&mut self, value: f32) {
        self.int(u64::from(value.to_bits()), 4);
    }
    fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
    fn transform(&mut self, t: &Transform) {
        for &v in t.position.iter().chain(t.rotation.iter()).chain(t.scale.iter()) {
            self.float(v);
        }
    }
    fn compact_transform(&mut self, t: &Transform) {
        let mut largest = 0usize;
        for i in 1..4 {
            if t.rotation[i].abs() > t.rotation[largest].abs() {
                largest = i;
            }
        }
        let wide = t.position.iter().any(|v| v.abs() > 32.767);
        let scale = t.scale != [1.0, 1.0, 1.0];
        self.int(((largest as u64) << 2) | u64::from(wide) | if scale { 2 } else { 0 }, 1);
        for &v in &t.position {
            if wide {
                self.float(v);
            } else {
                self.int(u64::from(round_i16(v * 1000.0)), 2);
            }
        }
        let mut norm = 0.0f32;
        for &v in &t.rotation {
            norm += v * v;
        }
        let factor = (if t.rotation[largest] < 0.0 { -1.0f32 } else { 1.0 }) * 46339.5358f32 / norm.sqrt();
        for i in 0..4 {
            if i != largest {
                self.int(u64::from(round_i16(t.rotation[i] * factor)), 2);
            }
        }
        if scale {
            for &v in &t.scale {
                self.float(v);
            }
        }
    }
    fn words(&mut self, values: &[u32]) {
        self.int(values.len() as u64, 2);
        for &v in values {
            self.int(u64::from(v), 4);
        }
    }
    fn recipe(&mut self, r: &CosmeticRecipe) {
        self.int(u64::from(r.key), 4);
        self.int(u64::from(r.version), 4);
        self.words(&r.scalars);
        self.int(r.items.len() as u64, 2);
        for item in &r.items {
            self.int(u64::from(item.slot), 4);
            self.int(item.asset.len() as u64, 2);
            self.raw(&item.asset);
            self.words(&item.parameters);
        }
    }
}

fn greeting_kind(k: u16) -> bool {
    matches!(
        k,
        kind::HELLO | kind::WELCOME | kind::CHALLENGE | kind::PEER_HELLO | kind::PEER_WELCOME | kind::MAP_REQUEST | kind::MAP_OFFER
    )
}

// Throws (panics) for an invalid packet, as the C++ encoder throws.
pub fn encode(p: &Packet, compact_pose: bool) -> Vec<u8> {
    let interval = p.pose_interval_us;
    if p.session == 0
        || p.epoch == 0
        || (p.world == 0 && p.kind != kind::MAP_REQUEST)
        || (p.kind == kind::POSE && (p.map == 0 || !valid_pose(&p.pose)))
        || (p.kind == kind::COSMETICS && (p.map == 0 || !valid_appearance(&p.appearance)))
        || (p.kind == kind::AUDIO && (p.map == 0 || !valid_audio_batch(&p.audio)))
    {
        panic!("Invalid multiplayer packet");
    }
    if p.kind == kind::ROSTER
        && (p.map == 0 || !valid_roster(&p.members, p.capacity) || !p.distances.valid() || p.voice_policy.revision == 0)
    {
        panic!("Invalid session roster");
    }
    if p.kind == kind::ROUTES && (p.map == 0 || !valid_routes(&p.members)) {
        panic!("Invalid direct route report");
    }
    if p.kind == kind::OBJECTS && (p.map == 0 || p.source == 0 || !valid_object_chunk(&p.objects)) {
        panic!("Invalid object update");
    }
    if p.kind == kind::VOICE && (p.map == 0 || p.source == 0 || !valid_voice(&p.voice)) {
        panic!("Invalid voice packet");
    }
    if p.kind == kind::CHAT && (p.source == 0 || !valid_chat_text(p.text.as_bytes())) {
        panic!("Invalid chat message");
    }
    if p.kind == kind::ADMIN && (p.source == 0 || !valid_admin_text(p.text.as_bytes())) {
        panic!("Invalid admin message");
    }
    if p.kind == kind::THROWDOWN && (p.source == 0 || p.throwdown.is_empty() || p.throwdown.len() > MAX_THROWDOWN_MESSAGE) {
        panic!("Invalid throwdown message");
    }
    if p.kind == kind::PHYSICS_TUNING && (p.source == 0 || p.tuning.len() > MAX_PHYSICS_TUNING) {
        panic!("Invalid physics tuning");
    }
    if p.kind == kind::PARTY && (p.source == 0 || !valid_party_request(p.party_action, p.party_player)) {
        panic!("Invalid party message");
    }
    if p.kind == kind::SCORING && (p.source == 0 || (!p.text.is_empty() && !valid_admin_text(p.text.as_bytes()))) {
        panic!("Invalid scoring report");
    }
    if p.kind == kind::TELEPORT && (p.source == 0 || p.teleport.iter().any(|v| !v.is_finite() || v.abs() > 1e6)) {
        panic!("Invalid teleport");
    }
    if p.kind == kind::BANS
        && (p.source == 0
            || p.bans.len() > MAX_BAN_ROWS
            || (p.ban_total as usize) < p.bans.len()
            || p.bans.iter().any(|b| !individual_steam_id(b.id) || !valid_member_name(b.name.as_bytes())))
    {
        panic!("Invalid ban list");
    }
    if p.kind == kind::MAPS && (p.source == 0 || p.maps.len() > MAX_SERVER_MAPS || p.maps.iter().any(|a| !valid_map_asset(a))) {
        panic!("Invalid server map list");
    }
    if p.kind == kind::HELLO && !p.text.is_empty() && !valid_member_name(p.text.as_bytes()) {
        panic!("Invalid player name");
    }
    if p.kind == kind::ROSTER && !valid_voice_range(p.voice_range) {
        panic!("Invalid voice range");
    }
    if p.kind == kind::MAP_OFFER && (!valid_map_destination(&p.destination) || p.map != map_hash(&p.destination)) {
        panic!("Invalid host map destination");
    }
    if p.kind == kind::WORLD_STATE
        && (if p.destination.is_empty() {
            p.map != 0 || p.world_ready
        } else {
            !valid_map_destination(&p.destination) || p.map != map_hash(&p.destination)
        })
    {
        panic!("Invalid world transition");
    }
    let packed = compact_pose && p.kind == kind::POSE;
    let greeting = greeting_kind(p.kind);
    let known = greeting
        || matches!(
            p.kind,
            kind::POSE
                | kind::AWAY
                | kind::COSMETICS
                | kind::AUDIO
                | kind::ROSTER
                | kind::ROUTES
                | kind::WORLD_STATE
                | kind::WORLD_READY
                | kind::OBJECTS
                | kind::VOICE
                | kind::CHAT
                | kind::ADMIN
                | kind::BANS
                | kind::MAPS
                | kind::THROWDOWN
                | kind::TELEPORT
                | kind::PHYSICS_TUNING
                | kind::PARTY
                | kind::SCORING
        );
    if !known {
        panic!("Unknown packet kind");
    }
    let payload = if greeting {
        72
    } else if p.kind == kind::AWAY {
        0
    } else {
        44 + (p.pose.skater.len() + p.pose.board.len()) * 40
    };
    let mut w = Writer { bytes: Vec::with_capacity(PACKET_HEADER_SIZE + payload) };
    w.int(u64::from(MAGIC), 4);
    w.int(u64::from(PROTOCOL_VERSION), 2);
    w.int(if packed { 8 } else { u64::from(p.kind) }, 2);
    w.int(payload as u64, 4);
    w.int(u64::from(p.sequence), 4);
    w.int(p.session, 8);
    w.int(p.map, 8);
    w.int(p.epoch, 8);
    w.int(p.time_us, 8);
    w.int(p.source, 8);
    w.int(p.world, 8);
    if greeting {
        w.raw(&p.build);
        w.int(p.challenge, 8);
        w.raw(&p.proof);
        if p.kind == kind::MAP_OFFER {
            w.int(u64::from(p.map_authorized), 1);
            w.int(p.destination.len() as u64, 2);
            w.raw(p.destination.as_bytes());
        }
        if p.kind == kind::HELLO {
            w.int(p.text.len() as u64, 1);
            w.raw(p.text.as_bytes());
        }
    } else if p.kind == kind::WORLD_STATE {
        w.raw(&p.build);
        w.int(u64::from(p.world_ready), 1);
        w.int(p.destination.len() as u64, 2);
        w.raw(p.destination.as_bytes());
    } else if p.kind == kind::WORLD_READY {
        w.int(u64::from(p.world_ready), 1);
    } else if p.kind == kind::POSE {
        w.int(p.pose.skater.len() as u64, 2);
        w.int(p.pose.board.len() as u64, 2);
        if !valid_pose_interval(interval) {
            panic!("Invalid pose update interval");
        }
        w.int(u64::from((1_000_000 / interval) | if p.player_collision { 0x8000 } else { 0 }), 2);
        let transform = |w: &mut Writer, t: &Transform| {
            if packed {
                w.compact_transform(t)
            } else {
                w.transform(t)
            }
        };
        transform(&mut w, &p.pose.root);
        for t in &p.pose.skater {
            transform(&mut w, t);
        }
        for t in &p.pose.board {
            transform(&mut w, t);
        }
    } else if p.kind == kind::COSMETICS {
        w.recipe(&p.appearance.skater);
        w.recipe(&p.appearance.board);
        w.int(u64::from(p.appearance.card.background), 4);
        w.int(u64::from(p.appearance.card.emblem), 4);
        w.int(u64::from(p.appearance.card.title), 4);
    } else if p.kind == kind::AUDIO {
        w.int(p.audio.len() as u64, 2);
        let mut previous = AudioState::default();
        for s in &p.audio {
            w.int(u64::from(s.age_us | if s.event { 0x8000_0000 } else { 0 }), 4);
            let (mut values, mut selectors, mut flags) = (0u64, 0u64, 0u64);
            for i in 0..AUDIO_FLOAT_COUNT {
                if s.state.values[i].to_bits() != previous.values[i].to_bits() {
                    values |= 1 << i;
                }
            }
            for i in 0..AUDIO_SELECTOR_COUNT {
                if s.state.selectors[i] != previous.selectors[i] {
                    selectors |= 1 << i;
                }
            }
            for i in 0..AUDIO_FLAG_COUNT {
                flags |= u64::from(s.state.flags[i]) << i;
            }
            w.int(values, 8);
            w.int(selectors, 4);
            w.int(flags, 6);
            for i in 0..AUDIO_FLOAT_COUNT {
                if values & (1 << i) != 0 {
                    w.float(s.state.values[i]);
                }
            }
            for i in 0..AUDIO_SELECTOR_COUNT {
                if selectors & (1 << i) != 0 {
                    w.int(u64::from(s.state.selectors[i]), 4);
                }
            }
            previous = s.state.clone();
        }
    }
    if p.kind == kind::VOICE {
        w.float(p.voice.distance);
        w.float(p.voice.gain);
        w.int(u64::from(p.voice.policy_revision), 4);
        w.int(p.voice.bytes.len() as u64, 2);
        w.raw(&p.voice.bytes);
    }
    if p.kind == kind::CHAT || p.kind == kind::ADMIN {
        w.int(p.text.len() as u64, 2);
        w.raw(p.text.as_bytes());
    }
    if p.kind == kind::THROWDOWN {
        w.int(p.throwdown.len() as u64, 2);
        w.raw(&p.throwdown);
    }
    if p.kind == kind::TELEPORT {
        for &v in &p.teleport {
            w.float(v);
        }
    }
    if p.kind == kind::PARTY {
        w.int(u64::from(p.party_action), 1);
        w.int(p.party_player, 8);
    }
    if p.kind == kind::PHYSICS_TUNING {
        w.int(p.tuning.len() as u64, 2);
        w.raw(&p.tuning);
    }
    if p.kind == kind::SCORING {
        w.int(p.scoring, 8);
        w.int(p.text.len() as u64, 2);
        w.raw(p.text.as_bytes());
    }
    if p.kind == kind::MAPS {
        w.int(p.maps.len() as u64, 2);
        for asset in &p.maps {
            w.int(asset.len() as u64, 1);
            w.raw(asset.as_bytes());
        }
    }
    if p.kind == kind::BANS {
        w.int(u64::from(p.ban_total), 4);
        w.int(p.bans.len() as u64, 2);
        for ban in &p.bans {
            w.int(ban.id, 8);
            w.int(ban.added as u64, 8);
            w.int(ban.name.len() as u64, 1);
            w.raw(ban.name.as_bytes());
        }
    }
    if p.kind == kind::ROSTER {
        w.int(u64::from(p.capacity), 1);
        w.int(p.members.len() as u64, 1);
        for m in &p.members {
            w.int(m.id, 8);
            w.int(m.epoch, 8);
            w.int(m.name.len() as u64, 1);
            w.raw(m.name.as_bytes());
            let flags = u64::from(m.admin)
                | if m.party_leader { 2 } else { 0 }
                | if m.party_open { 4 } else { 0 }
                | if m.speeding { 8 } else { 0 }
                | if m.scoring { 16 } else { 0 };
            w.int(flags, 1);
            w.int(u64::from(m.party), 4);
        }
        w.int(p.distances.full_rate_return as u64, 2);
        w.int(p.distances.half_rate_start as u64, 2);
        w.int(p.distances.half_rate_return as u64, 2);
        w.int(p.distances.low_rate_start as u64, 2);
        for lot in 0..p.parks.len() {
            w.int(u64::from(encode_park(lot, &p.parks[lot])), 1);
        }
        w.int(u64::from(p.object_clears), 4);
        if !valid_multiplayer_tps(p.tps) {
            panic!("Invalid session TPS");
        }
        w.int(u64::from(p.tps), 1);
        w.int(u64::from(p.object_placement), 1);
        w.int(u64::from(p.force_world_layers), 1);
        // No layers (a dedicated server without the catalog) means every layer at its default.
        if p.layers.len() > 0xffff || (p.layers.is_empty() && p.force_world_layers) {
            panic!("Invalid world layer state");
        }
        w.int(p.layers.len() as u64, 2);
        for &choice in &p.layers {
            if choice as usize >= WORLD_LAYER_MODES.len() {
                panic!("Invalid world layer mode");
            }
            w.int(u64::from(choice), 1);
        }
        w.int(u64::from(p.voice_policy.allowed), 1);
        w.int(u64::from(p.voice_policy.revision), 4);
        w.int(u64::from(round_i16(p.voice_range)), 2);
        if p.server_votes > 7 {
            panic!("Invalid server votes");
        }
        let tools = u64::from(p.guest_noclip)
            | if p.guest_no_bail { 2 } else { 0 }
            | if p.guest_boosts { 4 } else { 0 }
            | (u64::from(p.server_votes) << 3)
            | if p.enforce_tuning { 64 } else { 0 };
        w.int(tools, 1);
    }
    if p.kind == kind::ROUTES {
        w.int(p.members.len() as u64, 1);
        for m in &p.members {
            w.int(m.id, 8);
            w.int(m.epoch, 8);
        }
    }
    if p.kind == kind::OBJECTS {
        let chunk = &p.objects;
        w.int(chunk.base, 8);
        w.int(chunk.revision, 8);
        w.int(u64::from(chunk.part), 2);
        w.int(u64::from(chunk.parts), 2);
        w.int(chunk.objects.len() as u64, 2);
        w.int(chunk.removed.len() as u64, 2);
        for object in &chunk.objects {
            w.int(object.id, 8);
            w.int(object.item.len() as u64, 2);
            w.raw(object.item.as_bytes());
            for &v in &object.position {
                w.float(v);
            }
            for &v in &object.rotation {
                w.float(v);
            }
            w.float(object.scale);
        }
        for &id in &chunk.removed {
            w.int(id, 8);
        }
    }
    let actual_payload = (w.bytes.len() - PACKET_HEADER_SIZE) as u32;
    w.bytes[8..12].copy_from_slice(&actual_payload.to_le_bytes());
    w.bytes
}

// ---- Decoding --------------------------------------------------------------------------------
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn int(&mut self, width: usize) -> Option<u64> {
        if self.at > self.bytes.len() || width > self.bytes.len() - self.at {
            return None;
        }
        let mut result = 0u64;
        for i in 0..width {
            result |= u64::from(self.bytes[self.at]) << (8 * i);
            self.at += 1;
        }
        Some(result)
    }
    fn float(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.int(4)? as u32))
    }
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }
    fn take(&mut self, length: usize) -> &'a [u8] {
        let slice = &self.bytes[self.at..self.at + length];
        self.at += length;
        slice
    }
    fn transform(&mut self) -> Option<Transform> {
        let mut t = Transform::default();
        for v in t.position.iter_mut().chain(t.rotation.iter_mut()).chain(t.scale.iter_mut()) {
            *v = self.float()?;
        }
        valid_transform(&t).then_some(t)
    }
    fn compact_transform(&mut self) -> Option<Transform> {
        let mut t = Transform::default();
        let flags = self.int(1)?;
        if flags & !15 != 0 {
            return None;
        }
        for i in 0..3 {
            t.position[i] = if flags & 1 != 0 { self.float()? } else { f32::from(self.int(2)? as u16 as i16) / 1000.0 };
        }
        let largest = (flags >> 2) as usize;
        let mut norm = 0.0f32;
        for i in 0..4 {
            if i != largest {
                t.rotation[i] = f32::from(self.int(2)? as u16 as i16) / 46339.5358f32;
                norm += t.rotation[i] * t.rotation[i];
            }
        }
        if norm > 1.0001 {
            return None;
        }
        t.rotation[largest] = (1.0 - norm).max(0.0).sqrt();
        if flags & 2 != 0 {
            for i in 0..3 {
                t.scale[i] = self.float()?;
            }
        }
        valid_transform(&t).then_some(t)
    }
    fn words(&mut self, limit: usize) -> Option<Vec<u32>> {
        let count = self.int(2)? as usize;
        if count > limit || count > self.left() / 4 {
            return None;
        }
        let mut result = Vec::with_capacity(count);
        for _ in 0..count {
            result.push(self.int(4)? as u32);
        }
        Some(result)
    }
    fn recipe(&mut self) -> Option<CosmeticRecipe> {
        let mut r = CosmeticRecipe {
            key: self.int(4)? as u32,
            version: self.int(4)? as u32,
            scalars: self.words(MAX_COSMETIC_SCALARS)?,
            items: Vec::new(),
        };
        let count = self.int(2)? as usize;
        if count == 0 || count > MAX_COSMETIC_SLOTS {
            return None;
        }
        for _ in 0..count {
            let slot = self.int(4)? as u32;
            let length = self.int(2)? as usize;
            if length > MAX_COSMETIC_ASSET || length > self.left() {
                return None;
            }
            let asset = self.take(length).to_vec();
            let parameters = self.words(MAX_COSMETIC_PARAMETERS)?;
            r.items.push(CosmeticSlot { slot, asset, parameters });
        }
        Some(r)
    }
}

pub fn decode(bytes: &[u8]) -> Option<Packet> {
    if bytes.len() < PACKET_HEADER_SIZE || bytes.len() > MAX_PACKET {
        return None;
    }
    let mut r = Reader { bytes, at: 0 };
    if r.int(4)? != u64::from(MAGIC) || r.int(2)? != u64::from(PROTOCOL_VERSION) {
        return None;
    }
    let mut p = Packet::default();
    let raw_kind = r.int(2)?;
    let packed = raw_kind == 8;
    p.kind = if packed { kind::POSE } else { raw_kind as u16 };
    if r.int(4)? != (bytes.len() - PACKET_HEADER_SIZE) as u64 {
        return None;
    }
    p.sequence = r.int(4)? as u32;
    p.session = r.int(8)?;
    p.map = r.int(8)?;
    p.epoch = r.int(8)?;
    p.time_us = r.int(8)?;
    p.source = r.int(8)?;
    p.world = r.int(8)?;
    if p.session == 0 || p.epoch == 0 || (p.world == 0 && p.kind != kind::MAP_REQUEST) {
        return None;
    }
    if greeting_kind(p.kind) {
        for v in p.build.iter_mut() {
            *v = r.int(1)? as u8;
        }
        p.challenge = r.int(8)?;
        for v in p.proof.iter_mut() {
            *v = r.int(1)? as u8;
        }
        if p.kind == kind::MAP_OFFER {
            let authorized = r.int(1)?;
            let length = r.int(2)? as usize;
            if authorized > 1 || length > 256 || length > r.left() {
                return None;
            }
            p.map_authorized = authorized != 0;
            p.destination = String::from_utf8_lossy(r.take(length)).into_owned();
            if !valid_map_destination(&p.destination) || p.map != map_hash(&p.destination) {
                return None;
            }
        }
        if p.kind == kind::HELLO {
            let length = r.int(1)? as usize;
            if length > MAX_MEMBER_NAME || length > r.left() {
                return None;
            }
            let name = r.take(length);
            if !name.is_empty() && !valid_member_name(name) {
                return None;
            }
            p.text = String::from_utf8(name.to_vec()).ok()?;
        }
    } else if p.kind == kind::WORLD_STATE || p.kind == kind::WORLD_READY {
        if p.kind == kind::WORLD_STATE {
            for v in p.build.iter_mut() {
                *v = r.int(1)? as u8;
            }
        }
        let ready = r.int(1)?;
        if ready > 1 {
            return None;
        }
        p.world_ready = ready != 0;
        if p.kind == kind::WORLD_STATE {
            let length = r.int(2)? as usize;
            if length > 256 || length > r.left() {
                return None;
            }
            p.destination = String::from_utf8_lossy(r.take(length)).into_owned();
            let bad = if p.destination.is_empty() {
                p.map != 0 || p.world_ready
            } else {
                !valid_map_destination(&p.destination) || p.map != map_hash(&p.destination)
            };
            if bad {
                return None;
            }
        }
    } else if p.kind == kind::POSE {
        let skater = r.int(2)? as usize;
        let board = r.int(2)? as usize;
        let rate = r.int(2)? as u32;
        let tps = rate & 0x7fff;
        p.player_collision = rate & 0x8000 != 0;
        if !valid_multiplayer_tps(tps) && tps != 10 && tps != 5 {
            return None;
        }
        p.pose_interval_us = 1_000_000 / tps;
        if p.map == 0
            || skater > MAX_SKATER_BONES
            || board > MAX_BOARD_BONES
            || !valid_pose_interval(p.pose_interval_us)
            || (!packed && bytes.len() != PACKET_HEADER_SIZE + 46 + (skater + board) * 40)
        {
            return None;
        }
        let transform = |r: &mut Reader| if packed { r.compact_transform() } else { r.transform() };
        p.pose.root = transform(&mut r)?;
        p.pose.skater.reserve(skater);
        p.pose.board.reserve(board);
        for _ in 0..skater {
            let t = transform(&mut r)?;
            p.pose.skater.push(t);
        }
        for _ in 0..board {
            let t = transform(&mut r)?;
            p.pose.board.push(t);
        }
    } else if p.kind == kind::COSMETICS {
        p.appearance.skater = r.recipe()?;
        p.appearance.board = r.recipe()?;
        p.appearance.card.background = r.int(4)? as u32;
        p.appearance.card.emblem = r.int(4)? as u32;
        p.appearance.card.title = r.int(4)? as u32;
        if p.map == 0 || !valid_appearance(&p.appearance) {
            return None;
        }
    } else if p.kind == kind::AUDIO {
        let count = r.int(2)? as usize;
        if p.map == 0 || count == 0 || count > MAX_AUDIO_SAMPLES {
            return None;
        }
        let mut previous = AudioState::default();
        for _ in 0..count {
            let mut s = AudioSample::default();
            let age = r.int(4)? as u32;
            s.age_us = age & 0x7fff_ffff;
            s.event = age & 0x8000_0000 != 0;
            let values = r.int(8)?;
            let selectors = r.int(4)?;
            let flags = r.int(6)?;
            if (values >> AUDIO_FLOAT_COUNT) != 0 || (selectors >> AUDIO_SELECTOR_COUNT) != 0 || (flags >> AUDIO_FLAG_COUNT) != 0 {
                return None;
            }
            s.state = previous.clone();
            for i in 0..AUDIO_FLOAT_COUNT {
                if values & (1 << i) != 0 {
                    s.state.values[i] = r.float()?;
                }
            }
            for i in 0..AUDIO_SELECTOR_COUNT {
                if selectors & (1 << i) != 0 {
                    s.state.selectors[i] = r.int(4)? as u32;
                }
            }
            for i in 0..AUDIO_FLAG_COUNT {
                s.state.flags[i] = ((flags >> i) & 1) as u8;
            }
            previous = s.state.clone();
            p.audio.push(s);
        }
        if !valid_audio_batch(&p.audio) {
            return None;
        }
    } else if p.kind == kind::VOICE {
        p.voice.distance = r.float()?;
        p.voice.gain = r.float()?;
        p.voice.policy_revision = r.int(4)? as u32;
        let count = r.int(2)? as usize;
        if p.map == 0 || p.source == 0 || count == 0 || count > MAX_VOICE_BYTES || count != r.left() {
            return None;
        }
        p.voice.bytes = r.take(count).to_vec();
        if !valid_voice(&p.voice) {
            return None;
        }
    } else if p.kind == kind::CHAT {
        let length = r.int(2)? as usize;
        if p.source == 0 || length == 0 || length > CHAT_MAX_BYTES || length != r.left() {
            return None;
        }
        let text = r.take(length);
        if !valid_chat_text(text) {
            return None;
        }
        p.text = String::from_utf8(text.to_vec()).ok()?;
    } else if p.kind == kind::THROWDOWN {
        let length = r.int(2)? as usize;
        if p.source == 0 || length == 0 || length > MAX_THROWDOWN_MESSAGE || length != r.left() {
            return None;
        }
        p.throwdown = r.take(length).to_vec();
    } else if p.kind == kind::PHYSICS_TUNING {
        let length = r.int(2)? as usize;
        if p.source == 0 || length > MAX_PHYSICS_TUNING || length != r.left() {
            return None;
        }
        p.tuning = r.take(length).to_vec();
    } else if p.kind == kind::SCORING {
        p.scoring = r.int(8)?;
        let length = r.int(2)? as usize;
        if p.source == 0 || length > MAX_ADMIN_TEXT || length != r.left() {
            return None;
        }
        let text = r.take(length);
        if !text.is_empty() && !valid_admin_text(text) {
            return None;
        }
        p.text = String::from_utf8(text.to_vec()).ok()?;
    } else if p.kind == kind::PARTY {
        p.party_action = r.int(1)? as u8;
        p.party_player = r.int(8)?;
        if p.source == 0 || !valid_party_request(p.party_action, p.party_player) {
            return None;
        }
    } else if p.kind == kind::TELEPORT {
        if p.source == 0 {
            return None;
        }
        for i in 0..3 {
            let v = r.float()?;
            if !v.is_finite() || v.abs() > 1e6 {
                return None;
            }
            p.teleport[i] = v;
        }
    } else if p.kind == kind::ADMIN {
        let length = r.int(2)? as usize;
        if p.source == 0 || length == 0 || length > MAX_ADMIN_TEXT || length != r.left() {
            return None;
        }
        let text = r.take(length);
        if !valid_admin_text(text) {
            return None;
        }
        p.text = String::from_utf8(text.to_vec()).ok()?;
    } else if p.kind == kind::MAPS {
        let count = r.int(2)? as usize;
        if p.source == 0 || count > MAX_SERVER_MAPS {
            return None;
        }
        for _ in 0..count {
            let length = r.int(1)? as usize;
            if length > MAX_MAP_ASSET || length > r.left() {
                return None;
            }
            let asset = String::from_utf8_lossy(r.take(length)).into_owned();
            if !valid_map_asset(&asset) {
                return None;
            }
            p.maps.push(asset);
        }
    } else if p.kind == kind::BANS {
        p.ban_total = r.int(4)? as u32;
        let count = r.int(2)? as usize;
        if p.source == 0 || count > MAX_BAN_ROWS || count > p.ban_total as usize {
            return None;
        }
        for _ in 0..count {
            let id = r.int(8)?;
            let added = r.int(8)? as i64;
            let length = r.int(1)? as usize;
            if length > MAX_MEMBER_NAME || length > r.left() {
                return None;
            }
            let name = r.take(length);
            if !individual_steam_id(id) || !valid_member_name(name) {
                return None;
            }
            p.bans.push(Ban { id, added, name: String::from_utf8(name.to_vec()).ok()? });
        }
    } else if p.kind == kind::ROSTER {
        p.capacity = r.int(1)? as u32;
        let count = r.int(1)? as usize;
        if p.map == 0 || count > MAX_PLAYERS {
            return None;
        }
        for _ in 0..count {
            let mut m = Member { id: r.int(8)?, epoch: r.int(8)?, ..Default::default() };
            let length = r.int(1)? as usize;
            if length > 128 || length > r.left() {
                return None;
            }
            m.name = String::from_utf8_lossy(r.take(length)).into_owned();
            let flags = r.int(1)?;
            if flags > 31 {
                return None;
            }
            m.admin = flags & 1 != 0;
            m.speeding = flags & 8 != 0;
            m.scoring = flags & 16 != 0;
            m.party_leader = flags & 2 != 0;
            m.party_open = flags & 4 != 0;
            m.party = r.int(4)? as u32;
            p.members.push(m);
        }
        p.distances.full_rate_return = r.int(2)? as i32;
        p.distances.half_rate_start = r.int(2)? as i32;
        p.distances.half_rate_return = r.int(2)? as i32;
        p.distances.low_rate_start = r.int(2)? as i32;
        for lot in 0..p.parks.len() {
            let code = r.int(1)? as u32;
            p.parks[lot] = decode_park(lot, code)?;
        }
        p.object_clears = r.int(4)? as u32;
        p.tps = r.int(1)? as u32;
        if !valid_multiplayer_tps(p.tps) {
            return None;
        }
        let placement = r.int(1)?;
        if placement > 2 {
            return None;
        }
        p.object_placement = placement as u8;
        let forced = r.int(1)?;
        if forced > 1 {
            return None;
        }
        p.force_world_layers = forced != 0;
        let layer_count = r.int(2)? as usize;
        if layer_count != world_layers().len() && (layer_count != 0 || p.force_world_layers) {
            return None;
        }
        p.layers = vec![0; world_layers().len()];
        for i in 0..layer_count {
            let mode = r.int(1)?;
            if mode as usize >= WORLD_LAYER_MODES.len() {
                return None;
            }
            p.layers[i] = mode as u8;
        }
        let voice_allowed = r.int(1)?;
        p.voice_policy.revision = r.int(4)? as u32;
        if voice_allowed > 1 || p.voice_policy.revision == 0 {
            return None;
        }
        p.voice_policy.allowed = voice_allowed != 0;
        p.voice_range = r.int(2)? as f32;
        if !valid_voice_range(p.voice_range) {
            return None;
        }
        let tools = r.int(1)?;
        if tools > 127 {
            return None;
        }
        p.guest_noclip = tools & 1 != 0;
        p.guest_no_bail = tools & 2 != 0;
        p.guest_boosts = tools & 4 != 0;
        p.server_votes = ((tools >> 3) & 7) as u8;
        p.enforce_tuning = tools & 64 != 0;
        if !valid_roster(&p.members, p.capacity) || !p.distances.valid() {
            return None;
        }
    } else if p.kind == kind::ROUTES {
        let count = r.int(1)? as usize;
        if p.map == 0 || count > MAX_REMOTE_PLAYERS {
            return None;
        }
        for _ in 0..count {
            let id = r.int(8)?;
            let epoch = r.int(8)?;
            p.members.push(Member { id, epoch, ..Default::default() });
        }
        if !valid_routes(&p.members) {
            return None;
        }
    } else if p.kind == kind::OBJECTS {
        let chunk = &mut p.objects;
        chunk.base = r.int(8)?;
        chunk.revision = r.int(8)?;
        chunk.part = r.int(2)? as u16;
        chunk.parts = r.int(2)? as u16;
        let count = r.int(2)? as usize;
        let removed = r.int(2)? as usize;
        if p.map == 0 || p.source == 0 || count + removed > OBJECT_CHUNK_ENTRIES {
            return None;
        }
        for _ in 0..count {
            let mut object = NetworkObject { id: r.int(8)?, ..Default::default() };
            let length = r.int(2)? as usize;
            if length > 256 || length > r.left() {
                return None;
            }
            object.item = String::from_utf8_lossy(r.take(length)).into_owned();
            for v in object.position.iter_mut() {
                *v = r.float()?;
            }
            for v in object.rotation.iter_mut() {
                *v = r.float()?;
            }
            object.scale = r.float()?;
            chunk.objects.push(object);
        }
        for _ in 0..removed {
            chunk.removed.push(r.int(8)?);
        }
        if !valid_object_chunk(chunk) {
            return None;
        }
    } else if p.kind != kind::AWAY {
        return None;
    }
    if r.at != bytes.len() {
        return None;
    }
    Some(p)
}

// ---- Session helpers (Session/room.h) --------------------------------------------------------
pub fn routed_source(p: &Packet, member: &Member, connection: u64) -> bool {
    // Hosting: only the connection's own identity may send as itself.
    member.id != 0 && member.epoch != 0 && p.source == member.id && p.epoch == member.epoch && connection == member.id
}

// A route report is a short lease: missing reports or a source reconnect restore relay.
pub fn needs_relay(routes: &[Member], reported: u64, source_id: u64, source_epoch: u64, now: u64) -> bool {
    reported == 0
        || now < reported
        || now - reported > 1_500_000
        || !routes.iter().any(|m| m.id == source_id && m.epoch == source_epoch)
}

pub fn pose_interval(distance_squared: f32, previous: u32, settings: &Distances, tps: u32) -> u32 {
    let full = multiplayer_pose_interval(tps);
    if !distance_squared.is_finite() || distance_squared < 0.0 || !settings.valid() {
        return full;
    }
    let beyond = |metres: i32| distance_squared > (metres.wrapping_mul(metres)) as f32;
    if !beyond(settings.full_rate_return) {
        return full;
    }
    if beyond(settings.low_rate_start) {
        return 200_000;
    }
    if previous == 200_000 && beyond(settings.half_rate_return) {
        return 200_000;
    }
    if beyond(settings.half_rate_start) || previous == 100_000 || previous == 200_000 {
        return 100_000;
    }
    full
}

#[derive(Clone, Copy)]
pub struct PoseDelivery {
    pub source: u64,
    pub epoch: u64,
    pub last_sent: u64,
    pub interval_us: u32,
    pub next_source_time: u64,
}
impl Default for PoseDelivery {
    fn default() -> Self {
        PoseDelivery { source: 0, epoch: 0, last_sent: 0, interval_us: 50000, next_source_time: 0 }
    }
}

// Preserve the timer phase across variable client frames/packet arrivals.
pub fn advance_pose_deadline(next: &mut u64, now: u64, interval: u64) {
    let behind = if *next != 0 && now >= *next { (now - *next) % interval } else { 0 };
    *next = now.wrapping_add(interval).wrapping_sub(behind);
}

#[derive(Clone, Default)]
pub struct ReceiveBudget {
    since: u64,
    bytes: u64,
    packets: u64,
}
impl ReceiveBudget {
    pub fn accept(&mut self, now: u64, size: usize, sources: u64) -> bool {
        if now.wrapping_sub(self.since) >= 1_000_000 {
            self.since = now;
            self.bytes = 0;
            self.packets = 0;
        }
        self.bytes += size as u64;
        self.packets += 1;
        sources != 0
            && sources <= MAX_REMOTE_PLAYERS as u64
            && self.bytes <= 2 * 1024 * 1024 * sources
            && self.packets <= (2 * u64::from(TICK_RATES[3]) + 80 + 32) * sources
    }
}

// Chat flood control (Engine/Game/Multiplayer/chat_rate.h).
#[derive(Clone)]
pub struct ChatRate {
    tokens: f64,
    refilled: u64,
    previous_at: u64,
    previous: String,
}
impl Default for ChatRate {
    fn default() -> Self {
        ChatRate { tokens: 4.0, refilled: 0, previous_at: 0, previous: String::new() }
    }
}
impl ChatRate {
    const BURST: f64 = 4.0;
    const INTERVAL_US: f64 = 1_500_000.0;
    const REPEAT_US: u64 = 15_000_000;
    // True when accepted.
    pub fn accept(&mut self, now: u64, text: &str, slack: f64) -> bool {
        let limit = Self::BURST + slack;
        if self.refilled == 0 || now < self.refilled {
            self.tokens = limit;
        } else {
            self.tokens = limit.min(self.tokens + (now - self.refilled) as f64 / Self::INTERVAL_US);
        }
        self.refilled = now;
        if !self.previous.is_empty() && text == self.previous && now >= self.previous_at && now - self.previous_at < Self::REPEAT_US {
            return false;
        }
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        self.previous = text.to_string();
        self.previous_at = now;
        true
    }
}

pub fn traffic_lane(k: u16) -> u16 {
    match k {
        kind::VOICE => 3,
        kind::POSE | kind::AUDIO => 0,
        kind::COSMETICS => 2,
        _ => 1,
    }
}

// The supported game build (Engine/Game/Build/supported_build.h): players and the server must
// agree on it.
pub const GAME_SHA256: &str = "fbce74d5e28ef525dbba2cb4adbebc13405bdbd88f31bc940bca45e4ae88b8f9";
pub fn game_sha256_bytes() -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&GAME_SHA256[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}
pub const STEAM_APP_ID: u32 = 3354750;
