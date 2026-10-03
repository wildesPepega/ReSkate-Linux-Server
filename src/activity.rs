// Console lines about what players are doing (Server/server_activity.cpp): throwdowns (drops
// placed, queues, starts, turns, results) and objects placed or removed. Read from the packets
// the server relays anyway; nothing here changes what it sends.
use crate::protocol::NetworkObject;
use crate::throwdown::{decode_throwdown, td_kind};
use std::collections::{BTreeMap, BTreeSet};

const JAM: &str = "JamSession";
const SPOT_BATTLE: &str = "SpotBattle";
const SKATE: &str = "ThrowdownSkate";
const QUEUE_FORGET_US: u64 = 60_000_000;
const JAM_QUIET_US: u64 = 60_000_000;
const TURNS_QUIET_US: u64 = 90_000_000;
const LEAD_INTERVAL_US: u64 = 15_000_000;
const ANNOUNCE_INTERVAL_US: u64 = 30_000_000;
const LISTED_OBJECTS: usize = 3;

// What the log produced: console lines, and lines worth telling the players in chat.
pub enum Activity {
    Log(String),
    Announce(String),
}

fn mode(series: &str) -> &'static str {
    match series {
        JAM => "Jam",
        SPOT_BATTLE => "Spot Battle",
        SKATE => "S.K.A.T.E.",
        _ => "throwdown", // the series is peer text: never echo it into chat
    }
}
fn points(value: i64) -> String {
    let mut digits = value.unsigned_abs().to_string();
    let mut at = digits.len() as isize - 3;
    while at > 0 {
        digits.insert(at as usize, ',');
        at -= 3;
    }
    if value < 0 {
        format!("-{digits}")
    } else {
        digits
    }
}
fn place(at: &[f32; 3]) -> String {
    format!("({:.0}, {:.0}, {:.0})", at[0], at[1], at[2])
}
fn item_name(object: &NetworkObject) -> String {
    let text = if object.item.is_empty() { "an object" } else { object.item.as_str() };
    crate::text::prefix(text, 64).to_string()
}

#[derive(Default)]
struct Throwdown {
    series: String,
    queue: Vec<u64>,
    players: Vec<u64>,
    quit: BTreeSet<u64>,
    started: bool,
    last: u64,
    total: BTreeMap<u64, i64>,
    turn: BTreeMap<u64, i64>,
    tries: BTreeMap<u64, [u32; 2]>,
    leading: u64,
    lead_logged: u64,
}

type Key = (u64, u32);

#[derive(Default)]
pub struct ActivityLog {
    names: BTreeMap<u64, String>,
    throwdowns: BTreeMap<Key, Throwdown>,
    announced: BTreeMap<u64, u64>,
}

impl ActivityLog {
    // `current` gives a connected player's name, or empty.
    fn name(&mut self, player: u64, current: &dyn Fn(u64) -> String) -> String {
        let now = current(player);
        if !now.is_empty() {
            self.names.insert(player, now.clone());
            return now;
        }
        self.names.get(&player).cloned().unwrap_or_else(|| player.to_string())
    }
    fn title(&mut self, key: Key, series: &str, current: &dyn Fn(u64) -> String) -> String {
        format!("{}'s {}", self.name(key.0, current), mode(series))
    }

    pub fn clear(&mut self) {
        self.throwdowns.clear();
    }

    // One relayed throwdown message from `sender`; `at` is where the sender skates, if known.
    pub fn throwdown(
        &mut self,
        sender: u64,
        bytes: &[u8],
        at: Option<[f32; 3]>,
        now: u64,
        current: &dyn Fn(u64) -> String,
    ) -> Vec<Activity> {
        let mut out = Vec::new();
        let Some(m) = decode_throwdown(bytes) else { return out };
        let key: Key = (m.leader, m.id);
        if m.kind == td_kind::OFFER {
            if sender != m.leader {
                return out;
            }
            if let Some(t) = self.throwdowns.get_mut(&key) {
                t.last = now;
                if t.series.is_empty() {
                    t.series = m.series.clone();
                }
                return out;
            }
            // A leader has one drop at a time: one of theirs still here has finished.
            let theirs: Vec<Key> = self.throwdowns.keys().filter(|k| k.0 == m.leader).copied().collect();
            for k in theirs {
                let t = self.throwdowns.remove(&k).unwrap();
                if t.started {
                    out.extend(self.finish(k, &t, current));
                }
            }
            let mut t = Throwdown { series: m.series.clone(), last: now, ..Default::default() };
            for &player in &m.order {
                if player != m.leader {
                    t.queue.push(player);
                }
            }
            self.throwdowns.insert(key, t);
            let leader = self.name(m.leader, current);
            out.push(Activity::Log(format!(
                "[throwdown] {leader} placed a {} drop{}",
                mode(&m.series),
                at.map(|at| format!(" near {}", place(&at))).unwrap_or_default()
            )));
            let due = self.announced.get(&m.leader).map_or(true, |&last| now.wrapping_sub(last) >= ANNOUNCE_INTERVAL_US);
            if due {
                self.announced.insert(m.leader, now);
                out.push(Activity::Announce(format!(
                    "{leader} placed a {} throwdown. Find it on the map to join.",
                    mode(&m.series)
                )));
            }
            return out;
        }
        if !self.throwdowns.contains_key(&key) {
            // Started before the server saw its offer (it restarted, or the map changed back).
            if m.kind == td_kind::CLOSE || m.kind == td_kind::LEAVE {
                return out;
            }
            self.throwdowns.insert(key, Throwdown::default());
        }
        let who = self.name(sender, current);
        let series = {
            let t = self.throwdowns.get_mut(&key).unwrap();
            t.last = now;
            t.series.clone()
        };
        let title = self.title(key, &series, current);
        match m.kind {
            td_kind::CLOSE => {
                let started = self.throwdowns[&key].started;
                if sender != m.leader || started {
                    return out;
                }
                out.push(Activity::Log(format!("[throwdown] {title} drop was removed")));
                self.throwdowns.remove(&key);
            }
            td_kind::JOIN => {
                let t = self.throwdowns.get_mut(&key).unwrap();
                if t.started || sender == m.leader || t.queue.contains(&sender) {
                    return out;
                }
                t.queue.push(sender);
                out.push(Activity::Log(format!("[throwdown] {who} joined {title} ({} in the queue)", t.queue.len() + 1)));
            }
            td_kind::LEAVE => {
                let t = self.throwdowns.get_mut(&key).unwrap();
                if t.started {
                    if !t.quit.insert(sender) {
                        return out;
                    }
                    out.push(Activity::Log(format!("[throwdown] {who} quit {title}")));
                    if t.players.iter().all(|p| t.quit.contains(p)) {
                        self.throwdowns.remove(&key);
                    }
                    return out;
                }
                if let Some(at) = t.queue.iter().position(|&p| p == sender) {
                    t.queue.remove(at);
                    out.push(Activity::Log(format!("[throwdown] {who} left the queue for {title}")));
                }
            }
            td_kind::START => {
                {
                    let t = &self.throwdowns[&key];
                    if sender != m.leader || t.started {
                        return out;
                    }
                }
                let mut list = String::new();
                for &player in &m.order {
                    let name = self.name(player, current);
                    if !list.is_empty() {
                        list.push_str(", ");
                    }
                    list.push_str(&name);
                }
                let t = self.throwdowns.get_mut(&key).unwrap();
                t.started = true;
                t.players = m.order.clone();
                t.queue.clear();
                out.push(Activity::Log(format!(
                    "[throwdown] {title} started with {} player{}: {list}",
                    t.players.len(),
                    if t.players.len() == 1 { "" } else { "s" }
                )));
            }
            td_kind::SCORE => {
                // Jam: the sender's running total.
                let t = self.throwdowns.get_mut(&key).unwrap();
                t.total.insert(sender, i64::from(m.value));
                // Ties keep the player already in front.
                let leading = t.leading;
                let mut top: Option<(u64, i64)> = None;
                for (&player, &value) in &t.total {
                    match top {
                        None => top = Some((player, value)),
                        Some((best, best_value)) => {
                            if best_value < value || (best_value == value && player == leading) {
                                top = Some((player, value));
                            }
                            let _ = best;
                        }
                    }
                }
                let Some((player, value)) = top else { return out };
                if player == t.leading || value <= 0 {
                    return out;
                }
                t.leading = player;
                if t.lead_logged != 0 && now.wrapping_sub(t.lead_logged) < LEAD_INTERVAL_US {
                    return out;
                }
                t.lead_logged = now;
                let name = self.name(player, current);
                out.push(Activity::Log(format!("[throwdown] {name} takes the lead in {title} with {}", points(value))));
            }
            td_kind::ROW => {
                // Spot Battle: the round board adds each line; the overall board is set to the best turn.
                let t = self.throwdowns.get_mut(&key).unwrap();
                if m.add {
                    *t.turn.entry(sender).or_insert(0) += i64::from(m.value);
                } else {
                    t.total.insert(sender, i64::from(m.value));
                }
            }
            td_kind::TURN_END => {
                let t = self.throwdowns.get_mut(&key).unwrap();
                let scored = std::mem::replace(t.turn.entry(sender).or_insert(0), 0);
                let best = t.total.get(&sender).map(|&b| format!(" (best {})", points(b))).unwrap_or_default();
                out.push(Activity::Log(format!(
                    "[throwdown] {title}: {who} scored {} in round {}{best}",
                    points(scored),
                    m.value
                )));
            }
            td_kind::ATTEMPT => {
                let t = self.throwdowns.get_mut(&key).unwrap();
                t.tries.entry(sender).or_insert([0, 0])[if m.add { 0 } else { 1 }] += 1;
                out.push(Activity::Log(format!(
                    "[throwdown] {title}: {who} {} (turn {})",
                    if m.add { "landed" } else { "missed" },
                    m.value
                )));
            }
            _ => {}
        }
        out
    }

    fn finish(&mut self, key: Key, t: &Throwdown, current: &dyn Fn(u64) -> String) -> Vec<Activity> {
        let mut order = t.players.clone();
        for &player in t.total.keys() {
            if !order.contains(&player) {
                order.push(player);
            }
        }
        let total = |player: u64| t.total.get(&player).copied().unwrap_or(0);
        // S.K.A.T.E. is won on letters, which the server does not see: its players stay in turn order.
        let ranked = t.series != SKATE;
        if ranked {
            order.sort_by(|&a, &b| total(b).cmp(&total(a)));
        }
        let mut results = String::new();
        for (i, &player) in order.iter().enumerate() {
            let name = self.name(player, current);
            let mut entry = if ranked {
                format!("{}. {} {}", i + 1, name, points(total(player)))
            } else {
                let tries = t.tries.get(&player).copied().unwrap_or([0, 0]);
                format!("{} {} landed / {} missed", name, tries[0], tries[1])
            };
            if t.quit.contains(&player) {
                entry.push_str(" (quit)");
            }
            if !results.is_empty() {
                results.push_str(", ");
            }
            results.push_str(&entry);
        }
        let title = self.title(key, &t.series, current);
        vec![Activity::Log(format!(
            "[throwdown] {title} has finished{}",
            if results.is_empty() { String::new() } else { format!(": {results}") }
        ))]
    }

    // Ends throwdowns that went quiet: a finished one sends nothing more.
    pub fn tick(&mut self, now: u64, current: &dyn Fn(u64) -> String) -> Vec<Activity> {
        let mut out = Vec::new();
        let keys: Vec<Key> = self.throwdowns.keys().copied().collect();
        for key in keys {
            let t = &self.throwdowns[&key];
            let quiet = if now > t.last { now - t.last } else { 0 };
            if !t.started && quiet > QUEUE_FORGET_US {
                self.throwdowns.remove(&key); // its leader stopped offering it without saying so
                continue;
            }
            let limit = if t.series == JAM { JAM_QUIET_US } else { TURNS_QUIET_US };
            if t.started && quiet > limit {
                let t = self.throwdowns.remove(&key).unwrap();
                out.extend(self.finish(key, &t, current));
            }
        }
        out
    }

    pub fn left(&mut self, player: u64, current: &dyn Fn(u64) -> String) {
        self.name(player, current); // keep the name for results after they left
        self.throwdowns.retain(|key, t| {
            t.queue.retain(|&p| p != player);
            // A queue goes with its leader; a running throwdown carries on for the others.
            t.started || key.0 != player
        });
    }

    // What `owner` shares with everyone went from `before` to `after`.
    pub fn objects(
        &mut self,
        owner: u64,
        before: &[NetworkObject],
        after: &[NetworkObject],
        current: &dyn Fn(u64) -> String,
    ) -> Vec<Activity> {
        let mut out = Vec::new();
        let mut old: BTreeMap<u64, &NetworkObject> = BTreeMap::new();
        for object in before {
            old.entry(object.id).or_insert(object);
        }
        let mut added = Vec::new();
        for object in after {
            if old.remove(&object.id).is_none() {
                added.push(object);
            }
        }
        let removed: Vec<&NetworkObject> = old.values().copied().collect();
        if added.is_empty() && removed.is_empty() {
            return out; // moved, turned or resized
        }
        let who = self.name(owner, current);
        let total = format!(" ({} in total)", after.len());
        if added.len() > LISTED_OBJECTS {
            out.push(Activity::Log(format!(
                "[objects] {who} {} {} objects{total}",
                if before.is_empty() { "brought" } else { "placed" },
                added.len()
            )));
        } else {
            for object in added {
                out.push(Activity::Log(format!("[objects] {who} placed {} at {}", item_name(object), place(&object.position))));
            }
        }
        if removed.len() > LISTED_OBJECTS {
            out.push(Activity::Log(format!("[objects] {who} removed {} objects{total}", removed.len())));
        } else {
            for object in removed {
                out.push(Activity::Log(format!("[objects] {who} removed {} at {}", item_name(object), place(&object.position))));
            }
        }
        out
    }
}
