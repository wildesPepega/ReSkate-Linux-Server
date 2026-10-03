// Messages that link the players' own throwdowns (Extension/Throwdowns/throwdown_wire.cpp).
// Relayed opaque; decoded only for the activity log.

pub mod td_kind {
    pub const OFFER: u8 = 1;
    pub const CLOSE: u8 = 2;
    pub const JOIN: u8 = 3;
    pub const LEAVE: u8 = 4;
    pub const START: u8 = 5;
    pub const SCORE: u8 = 6;
    pub const ROW: u8 = 7;
    pub const TURN_END: u8 = 8;
    pub const ATTEMPT: u8 = 9;
    pub const CHALLENGE_START: u8 = 10;
    pub const CHALLENGE_OPTOUT: u8 = 11;
    pub const CHALLENGE_ATTEMPT: u8 = 12;
    pub const CHALLENGE_SLAM: u8 = 13;
    pub const CHALLENGE_LEAVE: u8 = 14;
    pub const BEACON: u8 = 15;
}

const MAX_SERIES: usize = 48;
const MAX_PARAMS: usize = 1536;
const MAX_ORDER: usize = 32;
const MAX_BOARDS: u8 = 16;
const MAX_CHALLENGE_ID: usize = 96;
const MAX_CRITERIA: usize = 64;
const CRITERIA_SIZE: usize = 0x14;
const MAX_CHALLENGE_PLAYERS: usize = 4;

#[derive(Clone, Default, Debug)]
pub struct ThrowdownMessage {
    pub kind: u8,
    pub leader: u64,
    pub id: u32,
    pub series: String,
    pub placement: Vec<u8>,
    pub settings: Vec<u8>,
    pub order: Vec<u64>,
    pub value: i32,
    pub board: u8,
    pub add: bool,
    pub challenge: String,
    pub criteria: Vec<u8>,
    pub indexes: Vec<u8>,
    pub location: [f32; 16],
}

fn individual(id: u64) -> bool {
    (id >> 56) == 1 && ((id >> 52) & 15) == 1 && (id & 0xffff_ffff) != 0
}
fn valid_series(series: &str) -> bool {
    !series.is_empty() && series.len() <= MAX_SERIES && series.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn valid_challenge_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_CHALLENGE_ID && id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

pub fn valid_throwdown(m: &ThrowdownMessage) -> bool {
    use td_kind::*;
    if !individual(m.leader) || m.id == 0 {
        return false;
    }
    match m.kind {
        OFFER => {
            valid_series(&m.series)
                && !m.placement.is_empty()
                && m.placement.len() <= MAX_PARAMS
                && m.settings.len() <= MAX_PARAMS
                && m.order.len() <= MAX_ORDER
                && m.order.iter().all(|&id| individual(id))
        }
        START => !m.order.is_empty() && m.order.len() <= MAX_ORDER && m.order.iter().all(|&id| individual(id)),
        ROW => m.board < MAX_BOARDS,
        ATTEMPT | TURN_END => m.value > 0,
        CLOSE | JOIN | LEAVE | SCORE => true,
        CHALLENGE_START => {
            valid_series(&m.series)
                && valid_challenge_id(&m.challenge)
                && m.order.len() >= 2
                && m.order.len() <= MAX_CHALLENGE_PLAYERS
                && m.order[0] == m.leader
                && m.order.iter().all(|&id| individual(id))
        }
        CHALLENGE_ATTEMPT => {
            !m.criteria.is_empty()
                && m.criteria.len() % CRITERIA_SIZE == 0
                && m.criteria.len() <= MAX_CRITERIA * CRITERIA_SIZE
                && m.indexes.len() % 4 == 0
                && m.indexes.len() <= MAX_CRITERIA * 4
        }
        CHALLENGE_OPTOUT | CHALLENGE_SLAM | CHALLENGE_LEAVE => true,
        BEACON => m.leader != 0 && m.location.iter().all(|v| v.is_finite() && v.abs() < 1e6),
        _ => false,
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn int(&mut self, size: usize) -> Option<u64> {
        if size > self.bytes.len() - self.at {
            return None;
        }
        let mut value = 0u64;
        for i in 0..size {
            value |= u64::from(self.bytes[self.at + i]) << (8 * i);
        }
        self.at += size;
        Some(value)
    }
    fn blob(&mut self, size: usize, limit: usize) -> Option<Vec<u8>> {
        let length = self.int(size)? as usize;
        if length > limit || length > self.bytes.len() - self.at {
            return None;
        }
        let value = self.bytes[self.at..self.at + length].to_vec();
        self.at += length;
        Some(value)
    }
    fn text(&mut self, limit: usize) -> Option<String> {
        let length = self.int(1)? as usize;
        if length > limit || length > self.bytes.len() - self.at {
            return None;
        }
        let value = String::from_utf8_lossy(&self.bytes[self.at..self.at + length]).into_owned();
        self.at += length;
        Some(value)
    }
}

pub fn decode_throwdown(bytes: &[u8]) -> Option<ThrowdownMessage> {
    use td_kind::*;
    let mut r = Reader { bytes, at: 0 };
    let mut m = ThrowdownMessage::default();
    let k = r.int(1)?;
    if !(1..=15).contains(&k) {
        return None;
    }
    m.kind = k as u8;
    m.leader = r.int(8)?;
    m.id = r.int(4)? as u32;
    match m.kind {
        OFFER | START => {
            if m.kind == OFFER {
                m.series = r.text(MAX_SERIES)?;
                m.placement = r.blob(2, MAX_PARAMS)?;
                m.settings = r.blob(2, MAX_PARAMS)?;
            }
            let count = r.int(1)? as usize;
            if count > MAX_ORDER {
                return None;
            }
            for _ in 0..count {
                m.order.push(r.int(8)?);
            }
        }
        SCORE | TURN_END | CHALLENGE_SLAM => m.value = r.int(4)? as u32 as i32,
        ROW => {
            m.board = r.int(1)? as u8;
            let add = r.int(1)?;
            if add > 1 {
                return None;
            }
            m.add = add != 0;
            m.value = r.int(4)? as u32 as i32;
        }
        ATTEMPT => {
            m.value = r.int(4)? as u32 as i32;
            let landed = r.int(1)?;
            if landed > 1 {
                return None;
            }
            m.add = landed != 0;
            // The trick record, bit-exact (28 bytes): relayed, never read here.
            if r.bytes.len() - r.at < 28 {
                return None;
            }
            r.at += 28;
        }
        CHALLENGE_START => {
            m.series = r.text(MAX_SERIES)?;
            m.challenge = r.text(MAX_CHALLENGE_ID)?;
            let contest = r.int(1)?;
            if contest > 1 {
                return None;
            }
            m.add = contest != 0;
            let count = r.int(1)? as usize;
            if count > MAX_CHALLENGE_PLAYERS {
                return None;
            }
            for _ in 0..count {
                m.order.push(r.int(8)?);
            }
        }
        CHALLENGE_ATTEMPT => {
            m.criteria = r.blob(2, MAX_CRITERIA * CRITERIA_SIZE)?;
            m.indexes = r.blob(2, MAX_CRITERIA * 4)?;
        }
        BEACON => {
            let placed = r.int(1)?;
            if placed > 1 {
                return None;
            }
            m.add = placed != 0;
            if m.add {
                for v in m.location.iter_mut() {
                    *v = f32::from_bits(r.int(4)? as u32);
                }
            }
        }
        _ => {}
    }
    (r.at == bytes.len() && valid_throwdown(&m)).then_some(m)
}
