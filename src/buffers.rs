// What the server keeps of each player's streams (Extension/Multiplayer/Remote/playback_buffers.cpp).
// The server never plays anything back: these keep only the ordering and acceptance rules that
// decide which poses, audio and outfits it relays.
use crate::protocol::{kind, newer_sequence, valid_appearance, valid_audio_batch, valid_pose_interval, valid_transform};
use crate::protocol::{Appearance, Packet, Transform, MAX_BOARD_BONES, MAX_SKATER_BONES};
use std::collections::VecDeque;

#[derive(Default)]
pub struct AppearanceBuffer {
    value: Option<Appearance>,
    epoch: u64,
    sequence: u32,
}
impl AppearanceBuffer {
    pub fn push(&mut self, p: &Packet) -> bool {
        if p.kind != kind::COSMETICS
            || p.epoch == 0
            || !valid_appearance(&p.appearance)
            || (self.value.is_some() && (p.epoch != self.epoch || !newer_sequence(p.sequence, self.sequence)))
        {
            return false;
        }
        self.value = Some(p.appearance.clone());
        self.epoch = p.epoch;
        self.sequence = p.sequence;
        true
    }
}

const INT64_MAX: u64 = i64::MAX as u64;

#[derive(Default)]
pub struct AudioBuffer {
    frames: usize,
    epoch: u64,
    last_arrival: u64,
    seen: u64,
    sequence: u32,
}
impl AudioBuffer {
    pub fn push(&mut self, p: &Packet, arrival: u64) -> bool {
        if p.kind != kind::AUDIO
            || p.epoch == 0
            || !valid_audio_batch(&p.audio)
            || arrival > INT64_MAX
            || p.time_us > INT64_MAX
            || (self.epoch != 0 && (self.epoch != p.epoch || arrival < self.last_arrival))
        {
            return false;
        }
        // Deduplicate host and direct copies without losing a late edge.
        if self.seen == 0 {
            self.sequence = p.sequence;
            self.seen = 1;
        } else if newer_sequence(p.sequence, self.sequence) {
            let shift = p.sequence.wrapping_sub(self.sequence);
            self.seen = if shift >= 64 { 1 } else { (self.seen << shift) | 1 };
            self.sequence = p.sequence;
        } else {
            let behind = self.sequence.wrapping_sub(p.sequence);
            if behind >= 64 || self.seen & (1u64 << behind) != 0 {
                return false;
            }
            self.seen |= 1u64 << behind;
        }
        self.frames = (self.frames + p.audio.len()).min(256);
        self.epoch = p.epoch;
        self.last_arrival = arrival;
        true
    }
    pub fn clear(&mut self) {
        *self = AudioBuffer::default();
    }
    pub fn size(&self) -> usize {
        self.frames
    }
}

struct PoseFrame {
    root: Transform,
    skater: usize,
    board: usize,
    arrival: u64,
    source: u64,
}

#[derive(Default)]
pub struct PoseBuffer {
    frames: VecDeque<PoseFrame>,
    epoch: u64,
    sequence: u32,
    sender_clock: bool,
}
impl PoseBuffer {
    // For packets returned by decode(), which already validated every transform.
    pub fn push_validated(&mut self, p: &Packet, arrival: u64) -> bool {
        if p.kind != kind::POSE
            || p.epoch == 0
            || arrival > INT64_MAX
            || p.time_us > INT64_MAX
            || !valid_pose_interval(p.pose_interval_us)
            || !valid_transform(&p.pose.root)
            || p.pose.skater.len() > MAX_SKATER_BONES
            || p.pose.board.len() > MAX_BOARD_BONES
        {
            return false;
        }
        if !self.frames.is_empty() && p.epoch == self.epoch && !newer_sequence(p.sequence, self.sequence) {
            return false;
        }
        if let Some(back) = self.frames.back() {
            if arrival < back.arrival || (p.epoch == self.epoch && self.sender_clock && p.time_us <= back.source) {
                return false;
            }
        }
        let reset = p.epoch != self.epoch
            || self.frames.back().is_some_and(|back| {
                let mut d = 0.0f32;
                for i in 0..3 {
                    let v = back.root.position[i] - p.pose.root.position[i];
                    d += v * v;
                }
                d > 400.0 || back.skater != p.pose.skater.len() || back.board != p.pose.board.len()
            });
        if reset {
            self.clear();
        }
        self.epoch = p.epoch;
        self.sequence = p.sequence;
        if self.frames.is_empty() {
            self.sender_clock = p.time_us != 0;
        }
        let source = if self.sender_clock { p.time_us } else { arrival };
        self.frames.push_back(PoseFrame {
            root: p.pose.root,
            skater: p.pose.skater.len(),
            board: p.pose.board.len(),
            arrival,
            source,
        });
        while self.frames.len() > 64 {
            self.frames.pop_front();
        }
        while self.frames.len() > 3 && self.frames[self.frames.len() - 1].source.wrapping_sub(self.frames[1].source) >= 1_000_000 {
            self.frames.pop_front();
        }
        true
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.epoch = 0;
        self.sequence = 0;
        self.sender_clock = false;
    }
    pub fn size(&self) -> usize {
        self.frames.len()
    }
}
