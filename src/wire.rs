// Compression and reference (delta) encoding on top of the packet codec:
// Extension/Multiplayer/Net/{wire_codec,block_codec,delta_codec,pose_delta}.
use crate::protocol::{decode, encode, kind, Packet, MAX_PACKET, MAX_PLAYERS, PACKET_HEADER_SIZE};
use std::collections::{BTreeMap, VecDeque};

// ---- Blocks ----------------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq)]
enum BlockCodec {
    Lz4,
    Zstd,
}

fn compress_block(raw: &[u8]) -> Vec<u8> {
    if raw.is_empty() || raw.len() > MAX_PACKET + 1024 {
        panic!("Compression input exceeds bound");
    }
    // LZ4 is the encoder; Zstd stays decodable for older or in-flight packets.
    lz4_flex::block::compress(raw)
}

fn decompress_block(bytes: &[u8], raw: &mut [u8], codec: BlockCodec) -> bool {
    if raw.is_empty() || raw.len() > MAX_PACKET + 1024 || bytes.is_empty() || bytes.len() > MAX_PACKET {
        return false;
    }
    match codec {
        BlockCodec::Lz4 => matches!(lz4_flex::block::decompress_into(bytes, raw), Ok(n) if n == raw.len()),
        BlockCodec::Zstd => {
            // One self-contained frame, known bounded output, no dictionary/trailing frames.
            match zstd_safe::get_frame_content_size(bytes) {
                Ok(Some(size)) if size == raw.len() as u64 => {}
                _ => return false,
            }
            match zstd_safe::find_frame_compressed_size(bytes) {
                Ok(size) if size == bytes.len() => {}
                _ => return false,
            }
            let mut context = zstd_safe::DCtx::create();
            matches!(context.decompress(raw, bytes), Ok(n) if n == raw.len())
        }
    }
}

// ---- Wire ------------------------------------------------------------------------------------
const COMPRESSION_HEADER: usize = 12;

fn compressed(bytes: &[u8]) -> bool {
    bytes.len() >= COMPRESSION_HEADER && (bytes.starts_with(b"RMC1") || bytes.starts_with(b"RMZ1"))
}
fn get32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub fn encode_wire(packet: &Packet) -> Vec<u8> {
    encode_wire_bytes(&encode(packet, true))
}

pub fn encode_wire_bytes(raw: &[u8]) -> Vec<u8> {
    if raw.len() < 256 {
        return raw.to_vec();
    }
    let block = compress_block(raw);
    if block.len() + COMPRESSION_HEADER >= raw.len() {
        return raw.to_vec();
    }
    let mut out = Vec::with_capacity(COMPRESSION_HEADER + block.len());
    out.extend_from_slice(b"RMC1");
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&(block.len() as u32).to_le_bytes());
    out.extend_from_slice(&block);
    out
}

pub fn decode_wire_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > MAX_PACKET {
        return None;
    }
    if !compressed(bytes) {
        return Some(bytes.to_vec());
    }
    let raw_size = get32(bytes, 4) as usize;
    let payload = get32(bytes, 8) as usize;
    if raw_size < PACKET_HEADER_SIZE || raw_size > MAX_PACKET || payload != bytes.len() - COMPRESSION_HEADER || raw_size <= bytes.len() {
        return None;
    }
    let mut raw = vec![0u8; raw_size];
    let codec = if bytes[2] == b'Z' { BlockCodec::Zstd } else { BlockCodec::Lz4 };
    decompress_block(&bytes[COMPRESSION_HEADER..], &mut raw, codec).then_some(raw)
}

pub fn decode_wire(bytes: &[u8]) -> Option<Packet> {
    decode(&decode_wire_bytes(bytes)?)
}

// ---- Pose differences (pose_delta.h) ---------------------------------------------------------
mod pose_delta {
    use super::*;

    struct Cursor<'a> {
        bytes: &'a [u8],
        at: usize,
    }
    impl<'a> Cursor<'a> {
        fn take(&mut self, n: usize) -> Option<&'a [u8]> {
            if self.at > self.bytes.len() || n > self.bytes.len() - self.at {
                return None;
            }
            let out = &self.bytes[self.at..self.at + n];
            self.at += n;
            Some(out)
        }
        fn fields(&mut self) -> Option<[&'a [u8]; 4]> {
            let flags = self.take(1)?;
            if flags[0] & !15 != 0 {
                return None;
            }
            let position = self.take(if flags[0] & 1 != 0 { 12 } else { 6 })?;
            let rotation = self.take(6)?;
            let scale = self.take(if flags[0] & 2 != 0 { 12 } else { 0 })?;
            Some([flags, position, rotation, scale])
        }
    }

    fn count(raw: &[u8]) -> usize {
        let start = PACKET_HEADER_SIZE;
        if raw.len() < start + 6 || raw[6] != 8 || raw[7] != 0 {
            return 0;
        }
        let skater = raw[start] as usize | ((raw[start + 1] as usize) << 8);
        let board = raw[start + 2] as usize | ((raw[start + 3] as usize) << 8);
        if skater > crate::protocol::MAX_SKATER_BONES || board > crate::protocol::MAX_BOARD_BONES {
            return 0;
        }
        1 + skater + board
    }

    pub fn encode(raw: &[u8], base: &[u8]) -> Vec<u8> {
        encode_inner(raw, base).unwrap_or_default()
    }
    fn encode_inner(raw: &[u8], base: &[u8]) -> Option<Vec<u8>> {
        let prefix = PACKET_HEADER_SIZE + 6;
        let n = count(raw);
        let h = PACKET_HEADER_SIZE;
        if n == 0 || n != count(base) || raw[h..h + 4] != base[h..h + 4] {
            return None;
        }
        let mut out = Vec::with_capacity(raw.len() + (n + 1) / 2);
        out.extend_from_slice(&raw[..prefix]);
        for i in 0..prefix {
            out[i] ^= base[i];
        }
        let masks = out.len();
        out.resize(masks + (n + 1) / 2, 0);
        let mut a = Cursor { bytes: raw, at: prefix };
        let mut b = Cursor { bytes: base, at: prefix };
        for i in 0..n {
            let current = a.fields()?;
            let reference = b.fields()?;
            let mut mask = 0u8;
            for f in 0..4 {
                if current[f] == reference[f] {
                    continue;
                }
                mask |= 1 << f;
                // XOR only matching widths; otherwise transmit the new field.
                let same = current[f].len() == reference[f].len();
                for j in 0..current[f].len() {
                    out.push(current[f][j] ^ if same { reference[f][j] } else { 0 });
                }
            }
            out[masks + i / 2] |= mask << ((i % 2) * 4);
        }
        if a.at != raw.len() || b.at != base.len() {
            return None;
        }
        Some(out)
    }

    pub fn decode(patch: &[u8], base: &[u8], size: usize) -> Option<Vec<u8>> {
        let prefix = PACKET_HEADER_SIZE + 6;
        let h = PACKET_HEADER_SIZE;
        let n = count(base);
        if n == 0 || size > MAX_PACKET {
            return None;
        }
        let mut delta = Cursor { bytes: patch, at: 0 };
        let mut reference = Cursor { bytes: base, at: prefix };
        let header = delta.take(prefix)?;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(header);
        for i in 0..prefix {
            out[i] ^= base[i];
        }
        if count(&out) != n || out[h..h + 4] != base[h..h + 4] {
            return None;
        }
        let masks = delta.take((n + 1) / 2)?;
        if (n & 1) != 0 && (masks[masks.len() - 1] & 0xf0) != 0 {
            return None;
        }
        for i in 0..n {
            let previous = reference.fields()?;
            let mask = (masks[i / 2] >> ((i % 2) * 4)) & 15;
            let mut flags = previous[0][0];
            if mask & 1 != 0 {
                flags ^= delta.take(1)?[0];
            }
            if flags & !15 != 0 {
                return None;
            }
            out.push(flags);
            let widths = [1usize, if flags & 1 != 0 { 12 } else { 6 }, 6, if flags & 2 != 0 { 12 } else { 0 }];
            for f in 1..4 {
                if mask & (1 << f) == 0 {
                    if widths[f] != previous[f].len() {
                        return None;
                    }
                    out.extend_from_slice(previous[f]);
                } else {
                    let value = delta.take(widths[f])?;
                    let same = value.len() == previous[f].len();
                    for j in 0..value.len() {
                        out.push(value[j] ^ if same { previous[f][j] } else { 0 });
                    }
                }
            }
            if out.len() > size {
                return None;
            }
        }
        if out.len() != size || delta.at != patch.len() || reference.at != base.len() {
            return None;
        }
        Some(out)
    }
}

// ---- Deltas (delta_codec) --------------------------------------------------------------------
fn state_kind(k: u16) -> bool {
    k == kind::POSE || k == kind::AUDIO || k == kind::COSMETICS
}
fn put(b: &mut Vec<u8>, value: u64, n: usize) {
    for i in 0..n {
        b.push((value >> (8 * i)) as u8);
    }
}
fn get(b: &[u8], at: usize, n: usize) -> u64 {
    let mut value = 0u64;
    for i in 0..n {
        value |= u64::from(b[at + i]) << (8 * i);
    }
    value
}

type StreamKey = (u64, u16);

#[derive(Default)]
pub struct WireUpdate {
    pub bytes: Vec<u8>,
    pub baseline: Vec<u8>,
}
impl WireUpdate {
    pub fn establishes_baseline(&self) -> bool {
        !self.baseline.is_empty()
    }
}

struct SenderBase {
    raw: Vec<u8>,
    epoch: u64,
    world: u64,
    time: u64,
    touched: u64,
    sequence: u32,
}

// Every delta references a reliable full snapshot, never the previous delta. Failed or skipped
// sends do not advance the sender's reference state.
#[derive(Default)]
pub struct DeltaSender {
    bases: BTreeMap<StreamKey, SenderBase>,
    clock: u64,
}
impl DeltaSender {
    pub fn prepare(&self, p: &Packet) -> WireUpdate {
        let raw = encode(p, true);
        let wire = encode_wire_bytes(&raw);
        self.prepare_with(p, &raw, &wire)
    }

    pub fn prepare_with(&self, p: &Packet, raw: &[u8], wire: &[u8]) -> WireUpdate {
        let mut out = WireUpdate::default();
        if !state_kind(p.kind) {
            out.bytes = wire.to_vec();
            return out;
        }
        let base = match self.bases.get(&(p.source, p.kind)) {
            Some(base)
                if base.epoch == p.epoch
                    && base.world == p.world
                    && p.time_us >= base.time
                    && (p.kind == kind::COSMETICS || p.time_us - base.time < 2_000_000) =>
            {
                base
            }
            _ => {
                if wire.len() + 4 > MAX_PACKET {
                    return out;
                }
                out.bytes.reserve(wire.len() + 4);
                out.bytes.extend_from_slice(b"RMB1");
                out.bytes.extend_from_slice(wire);
                out.baseline = raw.to_vec();
                return out;
            }
        };
        let best = wire.len();
        if p.kind == kind::POSE {
            // Pose patches encode changed fields directly.
            let patch = pose_delta::encode(raw, &base.raw);
            if !patch.is_empty() {
                let mut sparse = b"RMS1".to_vec();
                put(&mut sparse, p.source, 8);
                put(&mut sparse, p.epoch, 8);
                put(&mut sparse, u64::from(p.kind), 2);
                put(&mut sparse, u64::from(base.sequence), 4);
                put(&mut sparse, raw.len() as u64, 4);
                put(&mut sparse, patch.len() as u64, 4);
                let compressed = compress_block(&patch);
                if 34 + compressed.len() < best {
                    sparse.extend_from_slice(&compressed);
                    out.bytes = sparse;
                }
            }
        } else {
            let mut difference = raw.to_vec();
            for i in 0..raw.len().min(base.raw.len()) {
                difference[i] ^= base.raw[i];
            }
            let mut delta = b"RMD1".to_vec();
            put(&mut delta, p.source, 8);
            put(&mut delta, p.epoch, 8);
            put(&mut delta, u64::from(p.kind), 2);
            put(&mut delta, u64::from(base.sequence), 4);
            put(&mut delta, raw.len() as u64, 4);
            let block = compress_block(&difference);
            if 30 + block.len() < best {
                delta.extend_from_slice(&block);
                out.bytes = delta;
            }
        }
        if out.bytes.is_empty() {
            out.bytes = wire.to_vec();
        }
        out
    }

    pub fn sent(&mut self, p: &Packet, update: WireUpdate) {
        let key = (p.source, p.kind);
        if !update.establishes_baseline() {
            self.clock += 1;
            if let Some(base) = self.bases.get_mut(&key) {
                base.touched = self.clock;
            }
            return;
        }
        // Evict old departed streams without allowing unbounded retained cosmetics.
        if !self.bases.contains_key(&key) && self.bases.len() >= MAX_PLAYERS * 3 {
            if let Some(oldest) = self.bases.iter().min_by_key(|(_, b)| b.touched).map(|(k, _)| *k) {
                self.bases.remove(&oldest);
            }
        }
        self.clock += 1;
        self.bases.insert(
            key,
            SenderBase { raw: update.baseline, epoch: p.epoch, world: p.world, time: p.time_us, touched: self.clock, sequence: p.sequence },
        );
    }
}

struct ReceiverBase {
    raw: Vec<u8>,
    epoch: u64,
    world: u64,
    session: u64,
    map: u64,
    sequence: u32,
}
#[derive(Default)]
struct Stream {
    bases: VecDeque<ReceiverBase>,
    touched: u64,
}

#[derive(Default)]
pub struct DeltaReceiver {
    streams: BTreeMap<StreamKey, Stream>,
    clock: u64,
}
impl DeltaReceiver {
    // Missing references are a recoverable dropped frame, not protocol failure.
    pub fn receive(&mut self, bytes: &[u8], missing: &mut bool, accepted_world: u64) -> Option<Packet> {
        *missing = false;
        if bytes.len() > MAX_PACKET {
            return None;
        }
        if bytes.starts_with(b"RMB1") {
            let raw = decode_wire_bytes(&bytes[4..])?;
            let p = decode(&raw)?;
            if !state_kind(p.kind) {
                return None;
            }
            // Late snapshots must not evict references for the current world.
            if accepted_world != 0 && p.world != accepted_world {
                return Some(p);
            }
            let key = (p.source, p.kind);
            if !self.streams.contains_key(&key) && self.streams.len() >= MAX_PLAYERS * 3 {
                if let Some(oldest) = self.streams.iter().min_by_key(|(_, s)| s.touched).map(|(k, _)| *k) {
                    self.streams.remove(&oldest);
                }
            }
            self.clock += 1;
            let stream = self.streams.entry(key).or_default();
            stream.touched = self.clock;
            if stream.bases.back().map_or(true, |b| b.epoch != p.epoch) {
                stream.bases.clear();
            }
            if !stream.bases.iter().any(|b| b.sequence == p.sequence) {
                stream.bases.push_back(ReceiverBase {
                    raw,
                    epoch: p.epoch,
                    world: p.world,
                    session: p.session,
                    map: p.map,
                    sequence: p.sequence,
                });
            }
            while stream.bases.len() > 3 {
                stream.bases.pop_front();
            }
            return Some(p);
        }
        let sparse = bytes.starts_with(b"RMS1") || bytes.starts_with(b"RMS2");
        if !sparse && !bytes.starts_with(b"RMD1") && !bytes.starts_with(b"RMD2") {
            return decode_wire(bytes);
        }
        let header = if sparse { 34 } else { 30 };
        if bytes.len() <= header {
            return None;
        }
        let key: StreamKey = (get(bytes, 4, 8), get(bytes, 20, 2) as u16);
        let epoch = get(bytes, 12, 8);
        let sequence = get(bytes, 22, 4) as u32;
        let size = get(bytes, 26, 4) as usize;
        if !state_kind(key.1) || epoch == 0 || size < PACKET_HEADER_SIZE || size > MAX_PACKET || bytes.len() >= size {
            return None;
        }
        let Some(stream) = self.streams.get_mut(&key) else {
            *missing = true;
            return None;
        };
        let Some(base) = stream.bases.iter().find(|b| b.epoch == epoch && b.sequence == sequence) else {
            *missing = true;
            return None;
        };
        let unpacked = if sparse { get(bytes, 30, 4) as usize } else { size };
        if sparse && (key.1 != kind::POSE || unpacked < PACKET_HEADER_SIZE + 6 || unpacked > MAX_PACKET + 1024) {
            return None;
        }
        let mut raw = vec![0u8; unpacked];
        let codec = if bytes[3] == b'2' { BlockCodec::Zstd } else { BlockCodec::Lz4 };
        if !decompress_block(&bytes[header..], &mut raw, codec) {
            return None;
        }
        if sparse {
            raw = pose_delta::decode(&raw, &base.raw, size)?;
        } else {
            for i in 0..raw.len().min(base.raw.len()) {
                raw[i] ^= base.raw[i];
            }
        }
        let p = decode(&raw)?;
        if p.source != key.0 || p.kind != key.1 || p.epoch != epoch || p.world != base.world || p.session != base.session || p.map != base.map {
            return None;
        }
        self.clock += 1;
        stream.touched = self.clock;
        Some(p)
    }
}
