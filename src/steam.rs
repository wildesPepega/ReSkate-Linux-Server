// The Steam side: a game server signed in anonymously for skate.'s app (Server/steam_server.cpp)
// and its networking sockets (Extension/Multiplayer/Steam/steam_transport.cpp, steam_lanes.h).
// libsteam_api.so is loaded from the server's folder at run time, like the Windows server loads
// steam_api64.dll; its flat C API is the same on both.
use crate::protocol::{MAX_PACKET, MAX_PLAYERS, PROTOCOL_VERSION, STEAM_APP_ID};
use libloading::Library;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

// ---- Steam types, laid out as the Linux SDK headers lay them out ------------------------------
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct NetIdentity {
    kind: i32,
    size: i32,
    data: [u8; 128],
}
const IDENTITY_STEAM_ID: i32 = 16;
impl NetIdentity {
    fn steam_id64(&self) -> u64 {
        if self.kind != IDENTITY_STEAM_ID {
            return 0;
        }
        let data = self.data;
        u64::from_le_bytes(data[..8].try_into().unwrap())
    }
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
struct ConnectionInfo {
    identity_remote: NetIdentity,
    user_data: i64,
    listen_socket: u32,
    address_remote: [u8; 18],
    pad: u16,
    pop_remote: u32,
    pop_relay: u32,
    state: i32,
    end_reason: i32,
    end_debug: [c_char; 128],
    description: [c_char; 128],
    flags: i32,
    reserved: [u32; 63],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct StatusChanged {
    connection: u32,
    info: ConnectionInfo,
    old_state: i32,
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
struct RealTimeStatus {
    state: i32,
    ping: i32,
    quality_local: f32,
    quality_remote: f32,
    out_packets: f32,
    out_bytes: f32,
    in_packets: f32,
    in_bytes: f32,
    send_rate: i32,
    pending_unreliable: i32,
    pending_reliable: i32,
    sent_unacked: i32,
    queue_time: i64,
    max_jitter: i32,
    reserved: [u32; 15],
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
struct LaneStatus {
    pending_unreliable: i32,
    pending_reliable: i32,
    sent_unacked: i32,
    pad: i32,
    queue_time: i64,
    reserved: [u32; 10],
}

#[repr(C)]
struct NetMessage {
    data: *mut c_void,
    size: c_int,
    connection: u32,
    identity_peer: NetIdentity,
    connection_user_data: i64,
    time_received: i64,
    message_number: i64,
    free_data: *mut c_void,
    release: *mut c_void,
    channel: c_int,
    flags: c_int,
    user_data: i64,
    lane: u16,
    pad: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ConfigValue {
    value: i32,
    data_type: i32,
    data: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SteamIp {
    ip: [u8; 16],
    kind: i32,
}

const _: () = {
    assert!(std::mem::size_of::<NetIdentity>() == 136);
    assert!(std::mem::size_of::<ConnectionInfo>() == 696);
    assert!(std::mem::size_of::<StatusChanged>() == 704);
    assert!(std::mem::size_of::<RealTimeStatus>() == 120);
    assert!(std::mem::size_of::<LaneStatus>() == 64);
    assert!(std::mem::size_of::<NetMessage>() == 216);
    assert!(std::mem::size_of::<ConfigValue>() == 16);
    assert!(std::mem::size_of::<SteamIp>() == 20);
};

const STATE_CONNECTING: i32 = 1;
const STATE_CONNECTED: i32 = 3;
const STATE_CLOSED_BY_PEER: i32 = 4;
const STATE_PROBLEM_DETECTED_LOCALLY: i32 = 5;
const RESULT_OK: i32 = 1;
const SEND_UNRELIABLE_NO_DELAY: c_int = 5;
const SEND_RELIABLE: c_int = 8;
const SEND_RELIABLE_NO_NAGLE: c_int = 9;
const CONFIG_SEND_RATE_MAX: i32 = 11;
const CONFIG_P2P_TRANSPORT_ICE_ENABLE: i32 = 104;
const ICE_ENABLE_DISABLE: u32 = 0;
const CONFIG_CALLBACK_STATUS_CHANGED: i32 = 201;
const CONFIG_INT32: i32 = 1;
const CONFIG_PTR: i32 = 5;

fn symbol<T: Copy>(library: &Library, name: &str) -> Result<T, String> {
    let bytes = format!("{name}\0");
    unsafe { library.get::<T>(bytes.as_bytes()).map(|s| *s).map_err(|_| format!("Missing Steam export: {name}")) }
}

// ---- Quiet Steam: its library prints its own notes while it starts --------------------------
struct QuietSteam {
    saved: [c_int; 2],
    null: c_int,
}
impl QuietSteam {
    fn new() -> Self {
        unsafe {
            libc::fflush(std::ptr::null_mut());
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            let mut saved = [-1, -1];
            if null >= 0 {
                for (i, fd) in [1, 2].into_iter().enumerate() {
                    saved[i] = libc::dup(fd);
                    if saved[i] >= 0 {
                        libc::dup2(null, fd);
                    }
                }
            }
            QuietSteam { saved, null }
        }
    }
}
impl Drop for QuietSteam {
    fn drop(&mut self) {
        if self.null < 0 {
            return;
        }
        unsafe {
            libc::fflush(std::ptr::null_mut());
            for (i, fd) in [1, 2].into_iter().enumerate() {
                if self.saved[i] >= 0 {
                    libc::dup2(self.saved[i], fd);
                    libc::close(self.saved[i]);
                }
            }
            libc::close(self.null);
        }
    }
}

// ---- The game server -------------------------------------------------------------------------
// What the server browser shows. Players behind a NAT often cannot query the server directly,
// so everything a browser row needs also rides in the game tags.
pub struct Advertisement {
    pub name: String,
    pub map: String,
    pub players: u32,
    pub max_players: u32,
    pub password: bool,
    pub listed: bool,
    pub secret: u64,
}

// Tags are comma separated, so a name loses its commas; the whole list must stay under
// Steam's 128 byte limit.
fn tag_text(text: &str, limit: usize) -> Vec<u8> {
    let text = text.replace(',', " ");
    // Cut at the limit only, and never through a character: a whole last character is kept
    // ("Café" stays "Café"), one the limit splits is dropped.
    crate::text::prefix(&text, limit).as_bytes().to_vec()
}

pub fn server_tags(a: &Advertisement) -> Vec<u8> {
    let mut tags = format!(
        "reskate,v{},k{:016x},p{},c{}{}",
        PROTOCOL_VERSION,
        a.secret,
        a.players,
        a.max_players,
        if a.password { ",w1" } else { ",w0" }
    )
    .into_bytes();
    tags.extend_from_slice(b",m");
    tags.extend(tag_text(&a.map, 24));
    tags.extend_from_slice(b",n");
    let limit = 127usize.wrapping_sub(tags.len()).wrapping_sub(2);
    tags.extend(tag_text(&a.name, limit));
    tags
}

fn c_text(bytes: &[u8]) -> CString {
    CString::new(bytes.iter().copied().filter(|&c| c != 0).collect::<Vec<u8>>()).unwrap()
}

type InitFn = unsafe extern "C" fn(u32, u16, u16, c_int, *const c_char, *const c_char, *mut c_char) -> c_int;
type VoidFn = unsafe extern "C" fn();
type PtrFn = unsafe extern "C" fn() -> *mut c_void;
type TextFn = unsafe extern "C" fn(*mut c_void, *const c_char);
type BoolSetFn = unsafe extern "C" fn(*mut c_void, bool);
type IntSetFn = unsafe extern "C" fn(*mut c_void, c_int);
type SelfFn = unsafe extern "C" fn(*mut c_void);
type BoolGetFn = unsafe extern "C" fn(*mut c_void) -> bool;
type U64GetFn = unsafe extern "C" fn(*mut c_void) -> u64;
type IpGetFn = unsafe extern "C" fn(*mut c_void) -> SteamIp;

pub struct SteamServer {
    library: Option<Library>,
    server: *mut c_void,
    started: bool,
    tags: Vec<u8>,
}

impl SteamServer {
    // Loads libsteam_api.so from `folder`, next to the server. `token`: a game server login token
    // (steam_token), or empty to sign in anonymously with a new Steam ID every start.
    pub fn start(folder: &Path, port: u16, query_port: u16, token: &str) -> Result<SteamServer, String> {
        let mut steam = SteamServer { library: None, server: std::ptr::null_mut(), started: false, tags: Vec::new() };
        match steam.start_inner(folder, port, query_port, token) {
            Ok(()) => Ok(steam),
            Err(e) => {
                steam.stop();
                Err(e)
            }
        }
    }

    fn start_inner(&mut self, folder: &Path, port: u16, query_port: u16, token: &str) -> Result<(), String> {
        let quiet = QuietSteam::new();
        let path = folder.join("libsteam_api.so");
        let library = unsafe { Library::new(&path) }
            .map_err(|e| format!("Cannot load {} ({e}). Put libsteam_api.so next to the server.", path.display()))?;
        // The game server reads its app from the environment, like the game does.
        std::env::set_var("SteamAppId", STEAM_APP_ID.to_string());
        std::env::set_var("SteamGameId", STEAM_APP_ID.to_string());
        let versions = b"SteamUtils010\0SteamNetworkingUtils004\0SteamGameServer015\0SteamNetworkingSockets012\0\0";
        let mut message = [0 as c_char; 1024];
        let init: InitFn = symbol(&library, "SteamInternal_GameServer_Init_V2")?;
        // 2: authentication mode. Players connect through Steam networking, not the game port.
        let result = unsafe {
            init(0, port, query_port, 2, c"1.0.0.0".as_ptr(), versions.as_ptr() as *const c_char, message.as_mut_ptr())
        };
        drop(quiet);
        self.library = Some(library);
        if result != 0 {
            let text = unsafe { CStr::from_ptr(message.as_ptr()) }.to_string_lossy().into_owned();
            return Err(format!(
                "Steam game server did not start: {}. If another server is running on this machine, give this one different port and query_port.",
                if text.is_empty() {
                    "is steamclient.so in ~/.steam/sdk64 or next to the server?".to_string()
                } else {
                    text
                }
            ));
        }
        self.started = true;
        let library = self.library.as_ref().unwrap();
        self.server = unsafe { symbol::<PtrFn>(library, "SteamAPI_SteamGameServer_v015")?() };
        if self.server.is_null() {
            return Err("Steam game server interface is unavailable.".into());
        }
        for (name, value) in [
            ("SteamAPI_ISteamGameServer_SetModDir", c"skate"),
            ("SteamAPI_ISteamGameServer_SetProduct", c"reskate"),
            ("SteamAPI_ISteamGameServer_SetGameDescription", c"ReSkate dedicated server"),
        ] {
            unsafe { symbol::<TextFn>(library, name)?(self.server, value.as_ptr()) };
        }
        unsafe {
            symbol::<BoolSetFn>(library, "SteamAPI_ISteamGameServer_SetDedicatedServer")?(self.server, true);
            if token.is_empty() {
                symbol::<SelfFn>(library, "SteamAPI_ISteamGameServer_LogOnAnonymous")?(self.server);
            } else {
                // config_error lets only letters and digits through, so it has no NUL.
                let token = std::ffi::CString::new(token).map_err(|_| "steam_token holds a NUL".to_string())?;
                symbol::<TextFn>(library, "SteamAPI_ISteamGameServer_LogOn")?(self.server, token.as_ptr());
            }
        }
        Ok(())
    }

    pub fn library(&self) -> &Library {
        self.library.as_ref().unwrap()
    }

    pub fn run_callbacks(&self) {
        if self.started {
            if let Ok(run) = symbol::<VoidFn>(self.library(), "SteamGameServer_RunCallbacks") {
                unsafe { run() };
            }
        }
    }
    pub fn logged_on(&self) -> bool {
        self.started
            && symbol::<BoolGetFn>(self.library(), "SteamAPI_ISteamGameServer_BLoggedOn").is_ok_and(|f| unsafe { f(self.server) })
    }
    pub fn steam_id(&self) -> u64 {
        if !self.started {
            return 0;
        }
        symbol::<U64GetFn>(self.library(), "SteamAPI_ISteamGameServer_GetSteamID").map_or(0, |f| unsafe { f(self.server) })
    }
    pub fn public_ip(&self) -> String {
        if !self.started {
            return String::new();
        }
        let Ok(get) = symbol::<IpGetFn>(self.library(), "SteamAPI_ISteamGameServer_GetPublicIP") else {
            return String::new();
        };
        let ip = unsafe { get(self.server) };
        if ip.kind != 0 {
            return "IPv6".into();
        }
        let bytes = ip.ip;
        let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        format!("{}.{}.{}.{}", value >> 24, (value >> 16) & 255, (value >> 8) & 255, value & 255)
    }

    pub fn advertise(&mut self, a: &Advertisement) {
        if !self.started {
            return;
        }
        let library = self.library.as_ref().unwrap();
        let text = |name: &str, value: &[u8]| {
            if let Ok(f) = symbol::<TextFn>(library, name) {
                let value = c_text(value);
                unsafe { f(self.server, value.as_ptr()) };
            }
        };
        text("SteamAPI_ISteamGameServer_SetServerName", a.name.as_bytes());
        text("SteamAPI_ISteamGameServer_SetMapName", crate::text::prefix(&a.map, 31).as_bytes());
        if let Ok(f) = symbol::<IntSetFn>(library, "SteamAPI_ISteamGameServer_SetMaxPlayerCount") {
            unsafe { f(self.server, a.max_players as c_int) };
        }
        if let Ok(f) = symbol::<BoolSetFn>(library, "SteamAPI_ISteamGameServer_SetPasswordProtected") {
            unsafe { f(self.server, a.password) };
        }
        let tags = server_tags(a);
        if tags != self.tags {
            text("SteamAPI_ISteamGameServer_SetGameTags", &tags);
            self.tags = tags;
        }
        if let Ok(f) = symbol::<BoolSetFn>(library, "SteamAPI_ISteamGameServer_SetAdvertiseServerActive") {
            unsafe { f(self.server, a.listed) };
        }
    }

    pub fn stop(&mut self) {
        if self.started {
            let library = self.library.as_ref().unwrap();
            unsafe {
                if let Ok(f) = symbol::<BoolSetFn>(library, "SteamAPI_ISteamGameServer_SetAdvertiseServerActive") {
                    f(self.server, false);
                }
                if let Ok(f) = symbol::<SelfFn>(library, "SteamAPI_ISteamGameServer_LogOff") {
                    f(self.server);
                }
            }
            for _ in 0..10 {
                self.run_callbacks();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if let Ok(f) = symbol::<VoidFn>(library, "SteamGameServer_Shutdown") {
                unsafe { f() };
            }
            self.started = false;
        }
        self.server = std::ptr::null_mut();
    }
}
impl Drop for SteamServer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- Networking ------------------------------------------------------------------------------
// Gameplay uses lane zero; control goes first; gameplay/cosmetics share remaining bandwidth.
const LANE_PRIORITIES: [c_int; 4] = [1, 0, 1, 1];
const LANE_WEIGHTS: [u16; 4] = [16, 1, 1, 4];
const VIRTUAL_PORT: c_int = 37;

pub struct TransportPeer {
    pub id: u64,
    pub connected: bool,
}
pub struct TransportMessage {
    pub peer: u64,
    pub bytes: Vec<u8>,
    // When Steam received it, in microseconds on a clock of its own (only differences mean
    // anything); 0 when unknown. A server held up for a while reads everything that arrived
    // meanwhile in one go: this is what tells that from a flood.
    pub arrived: u64,
}

#[derive(Clone, Copy)]
struct Api {
    sockets: *mut c_void,
    utils: *mut c_void,
    listen: unsafe extern "C" fn(*mut c_void, c_int, c_int, *const ConfigValue) -> u32,
    accept: unsafe extern "C" fn(*mut c_void, u32) -> c_int,
    close: unsafe extern "C" fn(*mut c_void, u32, c_int, *const c_char, bool) -> bool,
    close_listener: unsafe extern "C" fn(*mut c_void, u32) -> bool,
    get_info: unsafe extern "C" fn(*mut c_void, u32, *mut ConnectionInfo) -> bool,
    real_time: unsafe extern "C" fn(*mut c_void, u32, *mut RealTimeStatus, c_int, *mut LaneStatus) -> c_int,
    send_message: unsafe extern "C" fn(*mut c_void, u32, *const c_void, u32, c_int, *mut i64) -> c_int,
    receive_messages: unsafe extern "C" fn(*mut c_void, u32, *mut *mut NetMessage, c_int) -> c_int,
    release_message: unsafe extern "C" fn(*mut NetMessage),
    run_callbacks: unsafe extern "C" fn(*mut c_void),
    lanes: Option<Lanes>,
}

#[derive(Clone, Copy)]
struct Lanes {
    configure: unsafe extern "C" fn(*mut c_void, u32, c_int, *const c_int, *const u16) -> c_int,
    allocate: unsafe extern "C" fn(*mut c_void, c_int) -> *mut NetMessage,
    send: unsafe extern "C" fn(*mut c_void, c_int, *const *mut NetMessage, *mut i64),
}

impl Api {
    fn setup_lanes(&self, connection: u32) -> bool {
        match self.lanes {
            Some(l) => unsafe {
                (l.configure)(self.sockets, connection, LANE_PRIORITIES.len() as c_int, LANE_PRIORITIES.as_ptr(), LANE_WEIGHTS.as_ptr())
                    == RESULT_OK
            },
            None => false,
        }
    }
    fn transmit(&self, connection: u32, bytes: &[u8], flags: c_int, lane: u16) -> bool {
        let Some(l) = self.lanes else { return false };
        if bytes.is_empty() || bytes.len() > MAX_PACKET {
            return false;
        }
        unsafe {
            let message = (l.allocate)(self.utils, bytes.len() as c_int);
            if message.is_null() {
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*message).data as *mut u8, bytes.len());
            (*message).connection = connection;
            (*message).flags = flags;
            (*message).lane = lane;
            let mut result: i64 = 0;
            // Steam owns and frees the message on success and failure.
            (l.send)(self.sockets, 1, &message, &mut result);
            result > 0
        }
    }
}

// Status changes arrive on Steam's callback inside RunCallbacks; they only queue here.
struct CallbackState {
    events: VecDeque<StatusChanged>,
    close: Option<(usize, unsafe extern "C" fn(*mut c_void, u32, c_int, *const c_char, bool) -> bool)>,
}
static CALLBACK: Mutex<CallbackState> = Mutex::new(CallbackState { events: VecDeque::new(), close: None });

extern "C" fn status_changed(event: *mut StatusChanged) {
    if event.is_null() {
        return;
    }
    let event = unsafe { std::ptr::read_unaligned(event) };
    let Ok(mut state) = CALLBACK.lock() else { return };
    if state.events.len() < 64 {
        state.events.push_back(event);
        return;
    }
    let info = event.info;
    if info.state == STATE_CONNECTING {
        if let Some((sockets, close)) = state.close {
            unsafe { close(sockets as *mut c_void, event.connection, 4001, c"ReSkate callback queue full".as_ptr(), false) };
        }
    }
}

struct Link {
    handle: u32,
    connecting_since: u64,
    connected: bool,
    prioritized: bool,
    status: RealTimeStatus,
    lanes: [LaneStatus; 4],
    status_frame: u64,
    status_ok: bool,
    recheck: bool,
}

pub struct SteamTransport {
    api: Option<Api>,
    ready: bool,
    hosting: bool,
    pub local_id: u64,
    pub detail: String,
    listener: u32,
    links: BTreeMap<u64, Link>,
    peers: Vec<(u64, bool)>,
    frame: u64,
    next_sweep: u64,
    capacity: usize,
    clock: Instant,
    // Why Steam ended a player's connection, until the host reads it for the log.
    ended: BTreeMap<u64, String>,
    // A player's new connection that replaced their old one, held back for one poll so the host
    // sees the old one end before the new one appears.
    rejoining: BTreeMap<u64, Link>,
    // Reused by poll() and receive_into(), which run hundreds of times a second.
    spare_ids: Vec<u64>,
    spare_handles: Vec<(u64, u32, bool)>,
}

// Steam's reason for ending a connection (ESteamNetConnectionEnd), in words.
fn end_reason_text(code: i32) -> &'static str {
    match code {
        1000..=1999 => "closed by the player's game",
        2000..=2999 => "closed by the player's game after an error",
        3001 => "server: Steam is in offline mode",
        3002 => "server: cannot reach enough Steam relays",
        3003 => "server: lost its Steam relay",
        3004 => "server: network configuration problem",
        3005 => "server: not allowed to use the network",
        3000..=3999 => "server-side network problem",
        4001 => "timed out: the player stopped answering",
        4002 | 4003 => "player's connection failed authentication",
        4006 => "player's Steam is too old or too new",
        4000..=4999 => "problem on the player's side",
        5003 => "timed out",
        5005 => "Steam connectivity problem",
        5006 => "no Steam relay route to the player",
        5008 | 5009 => "could not establish a route to the player",
        5000..=5999 => "connection problem",
        _ => "connection ended",
    }
}

pub(crate) fn ended_text(state: i32, code: i32, debug: &str) -> String {
    let who = if state == STATE_CLOSED_BY_PEER { "closed by player" } else { "problem detected" };
    let detail = if debug.is_empty() { String::new() } else { format!(": {debug}") };
    format!("Steam {code}, {}, {who}{detail}", end_reason_text(code))
}

impl SteamTransport {
    pub fn new() -> Self {
        SteamTransport {
            api: None,
            ready: false,
            hosting: false,
            local_id: 0,
            detail: String::new(),
            listener: 0,
            links: BTreeMap::new(),
            peers: Vec::new(),
            frame: 0,
            next_sweep: 0,
            capacity: MAX_PLAYERS,
            clock: Instant::now(),
            ended: BTreeMap::new(),
            rejoining: BTreeMap::new(),
            spare_ids: Vec::new(),
            spare_handles: Vec::new(),
        }
    }

    fn millis(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64 + 1
    }

    // Dedicated server: the logged-on Steam game server's networking.
    pub fn open_game_server(&mut self, library: &Library) -> bool {
        if self.ready {
            return true;
        }
        match self.bind(library) {
            Ok(()) => true,
            Err(e) => {
                self.detail = e;
                false
            }
        }
    }

    fn bind(&mut self, library: &Library) -> Result<(), String> {
        unsafe {
            let user = symbol::<unsafe extern "C" fn() -> c_int>(library, "SteamGameServer_GetHSteamUser")?();
            if user == 0 {
                return Err("The Steam game server is not initialized.".into());
            }
            let find = symbol::<unsafe extern "C" fn(c_int, *const c_char) -> *mut c_void>(
                library,
                "SteamInternal_FindOrCreateGameServerInterface",
            )?;
            let sockets = symbol::<PtrFn>(library, "SteamAPI_SteamGameServerNetworkingSockets_SteamAPI_v012")?();
            if sockets.is_null() {
                return Err("Steam Networking Sockets v012 is unavailable.".into());
            }
            let utils = find(user, c"SteamNetworkingUtils004".as_ptr());
            let prefix = "SteamAPI_ISteamNetworkingSockets_";
            let lanes = (|| -> Result<Lanes, String> {
                Ok(Lanes {
                    configure: symbol(library, &format!("{prefix}ConfigureConnectionLanes"))?,
                    allocate: symbol(library, "SteamAPI_ISteamNetworkingUtils_AllocateMessage")?,
                    send: symbol(library, &format!("{prefix}SendMessages"))?,
                })
            })()
            .ok();
            let api = Api {
                sockets,
                utils,
                listen: symbol(library, &format!("{prefix}CreateListenSocketP2P"))?,
                accept: symbol(library, &format!("{prefix}AcceptConnection"))?,
                close: symbol(library, &format!("{prefix}CloseConnection"))?,
                close_listener: symbol(library, &format!("{prefix}CloseListenSocket"))?,
                get_info: symbol(library, &format!("{prefix}GetConnectionInfo"))?,
                real_time: symbol(library, &format!("{prefix}GetConnectionRealTimeStatus"))?,
                send_message: symbol(library, &format!("{prefix}SendMessageToConnection"))?,
                receive_messages: symbol(library, &format!("{prefix}ReceiveMessagesOnConnection"))?,
                release_message: symbol(library, "SteamAPI_SteamNetworkingMessage_t_Release")?,
                run_callbacks: symbol(library, &format!("{prefix}RunCallbacks"))?,
                lanes: if utils.is_null() { None } else { lanes },
            };
            let get_identity = symbol::<unsafe extern "C" fn(*mut c_void, *mut NetIdentity) -> bool>(
                library,
                &format!("{prefix}GetIdentity"),
            )?;
            let mut identity: NetIdentity = std::mem::zeroed();
            if !get_identity(sockets, &mut identity) || identity.steam_id64() == 0 {
                return Err("Steam identity is unavailable.".into());
            }
            self.local_id = identity.steam_id64();
            if utils.is_null() {
                return Err("Steam networking utilities are unavailable.".into());
            }
            symbol::<SelfFn>(library, "SteamAPI_ISteamNetworkingUtils_InitRelayNetworkAccess")?(utils);
            symbol::<unsafe extern "C" fn(*mut c_void) -> c_int>(library, &format!("{prefix}InitAuthentication"))?(sockets);
            CALLBACK.lock().unwrap().close = Some((sockets as usize, api.close));
            self.api = Some(api);
        }
        self.ready = true;
        self.detail = "Steam ready. Relay authentication may still be connecting.".into();
        Ok(())
    }

    fn options() -> [ConfigValue; 3] {
        // The callback, this connection's send-rate ceiling (Steam keeps congestion control),
        // and no ICE: every connection goes through Steam's relays, so no player's IP address
        // is shared with anyone, whatever their own Steam setting is.
        [
            ConfigValue {
                value: CONFIG_CALLBACK_STATUS_CHANGED,
                data_type: CONFIG_PTR,
                data: status_changed as extern "C" fn(*mut StatusChanged) as usize as u64,
            },
            ConfigValue { value: CONFIG_SEND_RATE_MAX, data_type: CONFIG_INT32, data: u64::from((1024 * 1024) as u32) },
            ConfigValue { value: CONFIG_P2P_TRANSPORT_ICE_ENABLE, data_type: CONFIG_INT32, data: u64::from(ICE_ENABLE_DISABLE) },
        ]
    }

    pub fn host(&mut self, capacity: usize) -> bool {
        self.stop();
        if !self.ready {
            return false;
        }
        if !(2..=MAX_PLAYERS).contains(&capacity) {
            return false;
        }
        self.capacity = capacity;
        let api = self.api.unwrap();
        let options = Self::options();
        self.listener = unsafe { (api.listen)(api.sockets, VIRTUAL_PORT, options.len() as c_int, options.as_ptr()) };
        self.hosting = self.listener != 0;
        self.detail =
            if self.listener != 0 { "Waiting for players to join." } else { "Steam could not open the P2P listener." }.into();
        self.listener != 0
    }

    pub fn disconnect(&mut self, id: u64, reason: &str) {
        let Some(link) = self.links.remove(&id) else { return };
        if let Some(api) = self.api {
            if link.handle != 0 {
                let reason = CString::new(reason.replace('\0', "")).unwrap();
                unsafe { (api.close)(api.sockets, link.handle, 1000, reason.as_ptr(), false) };
            }
        }
        self.detail = reason.to_string();
        self.publish_links();
    }

    pub fn stop(&mut self) {
        let listener = std::mem::replace(&mut self.listener, 0);
        self.hosting = false;
        if listener != 0 {
            if let Some(api) = self.api {
                unsafe { (api.close_listener)(api.sockets, listener) };
            }
        }
        while let Some(&id) = self.links.keys().next() {
            self.disconnect(id, "Disconnected.");
        }
        for (id, link) in std::mem::take(&mut self.rejoining) {
            self.links.insert(id, link);
            self.disconnect(id, "Disconnected.");
        }
        if self.ready {
            self.poll();
        }
    }

    fn publish_links(&mut self) {
        self.peers = self.links.iter().map(|(&id, link)| (id, link.connected)).collect();
    }

    // Why Steam ended this player's connection, once (None if the server closed it).
    pub fn take_end_reason(&mut self, id: u64) -> Option<String> {
        self.ended.remove(&id)
    }

    // The connections, into `out` (cleared first) so the caller can reuse its memory.
    pub fn peers_into(&self, out: &mut Vec<TransportPeer>) {
        out.clear();
        out.extend(self.peers.iter().map(|&(id, connected)| TransportPeer { id, connected }));
    }

    fn new_link(&self, handle: u32, now: u64) -> Link {
        let api = self.api.unwrap();
        Link {
            handle,
            connecting_since: now,
            connected: false,
            prioritized: api.setup_lanes(handle),
            status: unsafe { std::mem::zeroed() },
            lanes: unsafe { std::mem::zeroed() },
            status_frame: 0,
            status_ok: false,
            recheck: false,
        }
    }

    pub fn poll(&mut self) {
        if !self.ready {
            return;
        }
        let api = self.api.unwrap();
        self.frame += 1;
        unsafe { (api.run_callbacks)(api.sockets) };
        let now = self.millis();
        // Rejoins held back by the last poll: the host has dropped the old connection by now.
        for (id, link) in std::mem::take(&mut self.rejoining) {
            self.links.insert(id, link);
        }
        let events: VecDeque<StatusChanged> = std::mem::take(&mut CALLBACK.lock().unwrap().events);
        for event in events {
            let info = event.info;
            let connection = event.connection;
            if info.state != STATE_CONNECTING || info.listen_socket == 0 {
                // A tracked connection changed state: read it below in this poll.
                for link in self.links.values_mut() {
                    if link.handle == connection {
                        link.recheck = true;
                    }
                }
                continue;
            }
            let identity = info.identity_remote;
            let id = identity.steam_id64();
            let existing = self.links.get(&id).map(|l| l.handle);
            if existing == Some(connection) {
                continue;
            }
            let close = |code: c_int, text: &CStr| unsafe { (api.close)(api.sockets, connection, code, text.as_ptr(), false) };
            // A player whose old connection is still open (their game crashed or their network
            // dropped, and they joined again) replaces it; the C++ server turned them away.
            let others = self.links.len() - usize::from(existing.is_some()) + self.rejoining.len();
            if info.listen_socket != self.listener
                || self.listener == 0
                || id == 0
                || id == self.local_id
                || others >= self.capacity - 1
                || self.rejoining.contains_key(&id)
                || identity.kind != IDENTITY_STEAM_ID
            {
                close(4002, c"ReSkate session full or unavailable");
                continue;
            }
            if unsafe { (api.accept)(api.sockets, connection) } != RESULT_OK {
                close(4003, c"Cannot accept connection");
                continue;
            }
            let link = self.new_link(connection, now);
            if existing.is_some() {
                self.disconnect(id, "Replaced by a new connection from the same player.");
                self.ended.insert(id, "rejoined with a new connection".into());
                self.rejoining.insert(id, link);
                continue;
            }
            self.ended.remove(&id);
            self.links.insert(id, link);
        }
        // Connected links report changes through the status callback, so read their state
        // only then, plus once a second in case a callback was dropped.
        let sweep = now >= self.next_sweep;
        if sweep {
            self.next_sweep = now + 1000;
        }
        let mut ids = std::mem::take(&mut self.spare_ids);
        ids.clear();
        ids.extend(self.links.keys().copied());
        for &id in &ids {
            let Some(link) = self.links.get_mut(&id) else { continue };
            if link.connected && !link.recheck && !sweep {
                continue;
            }
            link.recheck = false;
            let mut info: ConnectionInfo = unsafe { std::mem::zeroed() };
            if !unsafe { (api.get_info)(api.sockets, link.handle, &mut info) } {
                self.ended.insert(id, "Steam connection disappeared".into());
                self.disconnect(id, "Steam connection disappeared.");
                continue;
            }
            let state = info.state;
            if state == STATE_CONNECTED {
                if !link.connected && !link.prioritized {
                    link.prioritized = api.setup_lanes(link.handle);
                }
                link.connected = true;
                self.detail = "Steam connected.".into();
            } else if state == STATE_CLOSED_BY_PEER || state == STATE_PROBLEM_DETECTED_LOCALLY {
                let debug = info.end_debug;
                let bytes: Vec<u8> = debug.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
                let text = String::from_utf8_lossy(&bytes).into_owned();
                self.ended.insert(id, ended_text(state, info.end_reason, &text));
                self.disconnect(id, if text.is_empty() { "Steam connection closed." } else { &text });
            } else if now.wrapping_sub(link.connecting_since) > 20000 {
                self.ended.insert(id, "Steam connection did not finish connecting within 20 s".into());
                self.disconnect(id, "Steam connection timed out.");
            }
        }
        self.spare_ids = ids;
        self.publish_links();
    }

    fn read_status(api: &Api, frame: u64, link: &mut Link) {
        link.status = unsafe { std::mem::zeroed() };
        link.lanes = unsafe { std::mem::zeroed() };
        let count = if link.prioritized { link.lanes.len() as c_int } else { 0 };
        let lanes = if count > 0 { link.lanes.as_mut_ptr() } else { std::ptr::null_mut() };
        link.status_ok = unsafe { (api.real_time)(api.sockets, link.handle, &mut link.status, count, lanes) } == RESULT_OK;
        link.status_frame = frame;
    }

    fn congested(link: &Link, lane: u16) -> bool {
        let status = link.status;
        if status.state != STATE_CONNECTED {
            return false;
        }
        let index = lane as usize;
        let lanes = link.lanes;
        let (queue, pending) = if link.prioritized && index < lanes.len() {
            let l = lanes[index];
            (l.queue_time, i64::from(l.pending_reliable) + i64::from(l.pending_unreliable))
        } else {
            (status.queue_time, i64::from(status.pending_reliable) + i64::from(status.pending_unreliable))
        };
        // Steam uses INT64_MAX when it cannot estimate the queue: not congestion.
        let queue = if queue == i64::MAX { 0 } else { queue.max(0) as u64 };
        queue > 75000 || pending > 256 * 1024
    }

    pub fn send(&mut self, id: u64, bytes: &[u8], reliable: bool, fresh: bool, lane: u16) -> bool {
        let Some(api) = self.api else { return false };
        let frame = self.frame;
        let Some(link) = self.links.get_mut(&id) else { return false };
        if !link.connected || bytes.len() > MAX_PACKET || bytes.is_empty() {
            return false;
        }
        if fresh {
            if link.status_frame != frame {
                Self::read_status(&api, frame, link);
            }
            if link.status_ok && Self::congested(link, lane) {
                return false; // Let the next current pose/audio state replace this one.
            }
        }
        let flags = if reliable {
            if lane == 2 {
                SEND_RELIABLE
            } else {
                SEND_RELIABLE_NO_NAGLE
            }
        } else {
            SEND_UNRELIABLE_NO_DELAY
        };
        if link.prioritized {
            api.transmit(link.handle, bytes, flags, lane)
        } else {
            unsafe {
                (api.send_message)(api.sockets, link.handle, bytes.as_ptr() as *const c_void, bytes.len() as u32, flags, std::ptr::null_mut())
                    == RESULT_OK
            }
        }
    }

    // What arrived since the last call, into `result` (cleared first) so the caller can reuse
    // its memory.
    pub fn receive_into(&mut self, result: &mut Vec<TransportMessage>) {
        result.clear();
        let Some(api) = self.api else { return };
        let mut ids = std::mem::take(&mut self.spare_handles);
        ids.clear();
        ids.extend(self.links.iter().map(|(&id, l)| (id, l.handle, l.connected)));
        for &(id, handle, connected) in &ids {
            if !connected {
                continue;
            }
            let mut messages: [*mut NetMessage; 128] = [std::ptr::null_mut(); 128];
            let count = unsafe { (api.receive_messages)(api.sockets, handle, messages.as_mut_ptr(), 128) };
            if count < 0 {
                self.disconnect(id, "Steam receive failed.");
                continue;
            }
            for &message in &messages[..count as usize] {
                if message.is_null() {
                    continue;
                }
                unsafe {
                    let size = (*message).size;
                    let data = (*message).data;
                    if size > 0 && size as usize <= MAX_PACKET && !data.is_null() {
                        let bytes = std::slice::from_raw_parts(data as *const u8, size as usize).to_vec();
                        let arrived = u64::try_from((*message).time_received).unwrap_or(0);
                        result.push(TransportMessage { peer: id, bytes, arrived });
                    }
                    (api.release_message)(message);
                }
            }
        }
        self.spare_handles = ids;
    }
}

impl Drop for SteamTransport {
    fn drop(&mut self) {
        self.stop();
        if let Ok(mut state) = CALLBACK.lock() {
            state.close = None;
            state.events.clear();
        }
    }
}
