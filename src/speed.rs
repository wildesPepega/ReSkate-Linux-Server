// Game-speed check for one player (Server/speed_check.h): a speedhack speeds up the game's
// clock, which the player's pose timestamps come from, so the player's clock runs ahead of the
// server's. Only the smallest arrival-minus-send gap per block is used, so lag cannot look
// like speed.
use std::collections::VecDeque;

pub const BLOCK_US: u64 = 2_000_000;
pub const WINDOW_US: u64 = 20_000_000;
pub const GAP_US: u64 = 2_500_000;
pub const LIMIT: f64 = 1.06;
pub const STRIKES_NEEDED: u32 = 3;

#[derive(Clone, Copy)]
struct Block {
    start: u64,
    min_offset: i64,
}

#[derive(Clone)]
pub struct SpeedCheck {
    blocks: VecDeque<Block>,
    last_sent: u64,
    last_arrived: u64,
    speed: f64,
    strikes: u32,
    started: bool,
}

impl Default for SpeedCheck {
    fn default() -> Self {
        SpeedCheck { blocks: VecDeque::new(), last_sent: 0, last_arrived: 0, speed: 1.0, strikes: 0, started: false }
    }
}

impl SpeedCheck {
    // One pose: the sender's timestamp and when it arrived. True when this sample completes a
    // measurement.
    pub fn sample(&mut self, sent: u64, arrived: u64) -> bool {
        if self.started && sent <= self.last_sent && self.last_sent - sent < GAP_US && arrived >= self.last_arrived {
            return false;
        }
        if !self.started || sent < self.last_sent || arrived < self.last_arrived || arrived - self.last_arrived > GAP_US {
            self.restart();
            self.started = true;
        }
        self.last_sent = sent;
        self.last_arrived = arrived;
        let offset = (arrived as i64).wrapping_sub(sent as i64);
        if self.blocks.back().map_or(true, |b| arrived.wrapping_sub(b.start) >= BLOCK_US) {
            let measured = self.measure();
            self.blocks.push_back(Block { start: arrived, min_offset: offset });
            while self.blocks.len() > 1 && arrived.wrapping_sub(self.blocks[1].start) >= WINDOW_US {
                self.blocks.pop_front();
            }
            return measured;
        }
        let back = self.blocks.back_mut().unwrap();
        back.min_offset = back.min_offset.min(offset);
        false
    }
    pub fn speed(&self) -> f64 {
        self.speed
    }
    pub fn flagged(&self) -> bool {
        self.strikes >= STRIKES_NEEDED
    }
    pub fn restart(&mut self) {
        self.blocks.clear();
        self.speed = 1.0;
        self.strikes = 0;
        self.started = false;
    }
    fn measure(&mut self) -> bool {
        if self.blocks.len() < 2 {
            return false;
        }
        let first = self.blocks[0];
        let last = self.blocks[self.blocks.len() - 1];
        let span = last.start.wrapping_sub(first.start);
        if span + BLOCK_US < WINDOW_US {
            return false;
        }
        // Sent time advanced by (arrival span + how much the gap shrank).
        self.speed = 1.0 + (first.min_offset - last.min_offset) as f64 / span as f64;
        self.strikes = if self.speed >= LIMIT { self.strikes + 1 } else { 0 };
        true
    }
}
