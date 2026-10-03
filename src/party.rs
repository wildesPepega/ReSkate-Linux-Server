// Parties on a dedicated server (Extension/Multiplayer/Session/party_book.{h,cpp}). The server
// owns this book; every roster carries each player's party. A party always has at least two
// members: one left alone is dissolved.
use std::collections::BTreeMap;

pub const INVITE_LIFETIME_US: u64 = 60_000_000;
const MAX_INVITES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PartyResult {
    Ok,
    SelfTarget,
    SameParty,
    Full,
    NotLeader,
    NotMember,
    NoInvite,
    Closed,
    NoParty,
    Busy,
    Renewed,
}

#[derive(Clone, Default)]
pub struct Party {
    pub leader: u64,
    pub members: Vec<u64>,
    pub open: bool,
}

#[derive(Clone, Copy)]
pub struct Invite {
    pub from: u64,
    pub to: u64,
    pub expires: u64,
}

pub struct PartyBook {
    parties: BTreeMap<u32, Party>,
    member_of: BTreeMap<u64, u32>,
    invites: Vec<Invite>,
    withdrawn: Vec<Invite>,
    next: u32,
    revision: u64,
    limit: usize,
}

impl PartyBook {
    pub fn new(limit: usize) -> Self {
        PartyBook {
            parties: BTreeMap::new(),
            member_of: BTreeMap::new(),
            invites: Vec::new(),
            withdrawn: Vec::new(),
            next: 1,
            revision: 1,
            limit: limit.max(2),
        }
    }
    pub fn party_of(&self, player: u64) -> u32 {
        self.member_of.get(&player).copied().unwrap_or(0)
    }
    pub fn party(&self, id: u32) -> Option<&Party> {
        self.parties.get(&id)
    }
    pub fn parties(&self) -> &BTreeMap<u32, Party> {
        &self.parties
    }
    pub fn invites(&self) -> &[Invite] {
        &self.invites
    }
    pub fn limit(&self) -> usize {
        self.limit
    }
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(2);
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    fn changed(&mut self) {
        self.revision += 1;
    }
    fn add(&mut self, id: u32, player: u64) {
        self.parties.entry(id).or_default().members.push(player);
        self.member_of.insert(player, id);
    }
    // Takes a player out of their party, handing the lead on and dissolving a party left with one.
    fn take_out(&mut self, player: u64) {
        let id = self.party_of(player);
        let Some(p) = self.parties.get_mut(&id) else { return };
        self.member_of.remove(&player);
        p.members.retain(|&m| m != player);
        if p.members.len() < 2 {
            let members = p.members.clone();
            for member in members {
                self.member_of.remove(&member);
            }
            self.parties.remove(&id);
        } else if p.leader == player {
            p.leader = p.members[0]; // the longest-standing member leads next
        }
        self.changed();
    }
    fn stale(&self, i: &Invite) -> bool {
        let party = self.party_of(i.from);
        let p = self.parties.get(&party);
        (party != 0 && party == self.party_of(i.to)) || p.is_some_and(|p| p.members.len() >= self.limit)
    }
    fn prune_invites(&mut self) {
        let invites = std::mem::take(&mut self.invites);
        for i in invites {
            if self.stale(&i) {
                self.withdrawn.push(i);
            } else {
                self.invites.push(i);
            }
        }
    }

    pub fn invite(&mut self, from: u64, to: u64, now: u64) -> PartyResult {
        if from == to {
            return PartyResult::SelfTarget;
        }
        let party = self.party_of(from);
        if party != 0 && party == self.party_of(to) {
            return PartyResult::SameParty;
        }
        if self.parties.get(&party).is_some_and(|p| p.members.len() >= self.limit) {
            return PartyResult::Full;
        }
        if let Some(found) = self.invites.iter_mut().find(|i| i.to == to && i.from == from) {
            found.expires = now + INVITE_LIFETIME_US;
            return PartyResult::Renewed;
        }
        if self.invites.iter().filter(|i| i.to == to).count() >= MAX_INVITES {
            return PartyResult::Busy;
        }
        self.invites.push(Invite { from, to, expires: now + INVITE_LIFETIME_US });
        PartyResult::Ok
    }

    pub fn accept(&mut self, to: u64, from: u64, now: u64) -> PartyResult {
        let Some(found) = self.invites.iter().position(|i| i.to == to && i.from == from && now < i.expires) else {
            return PartyResult::NoInvite;
        };
        self.invites.remove(found);
        let mut party = self.party_of(from);
        if party != 0 && party == self.party_of(to) {
            return PartyResult::SameParty;
        }
        if self.parties.get(&party).is_some_and(|p| p.members.len() >= self.limit) {
            return PartyResult::Full;
        }
        self.take_out(to);
        if party == 0 {
            party = self.next;
            self.next = self.next.wrapping_add(1);
            if self.next == 0 {
                self.next = 1;
            }
            self.parties.insert(party, Party { leader: from, members: Vec::new(), open: false });
            self.add(party, from);
        }
        self.add(party, to);
        self.changed();
        self.prune_invites();
        PartyResult::Ok
    }

    pub fn decline(&mut self, to: u64, from: u64) -> PartyResult {
        let before = self.invites.len();
        self.invites.retain(|i| !(i.to == to && i.from == from));
        if self.invites.len() == before {
            PartyResult::NoInvite
        } else {
            PartyResult::Ok
        }
    }

    pub fn join(&mut self, who: u64, target: u64, now: u64) -> PartyResult {
        if who == target {
            return PartyResult::SelfTarget;
        }
        let party = self.party_of(target);
        if party == 0 {
            return PartyResult::NoParty;
        }
        if party == self.party_of(who) {
            return PartyResult::SameParty;
        }
        // An invite from anyone in that party lets the player in, as does an open party.
        if let Some(invite) = self.invites.iter().find(|i| i.to == who && self.party_of(i.from) == party && now < i.expires) {
            let from = invite.from;
            return self.accept(who, from, now);
        }
        let p = &self.parties[&party];
        if !p.open {
            return PartyResult::Closed;
        }
        if p.members.len() >= self.limit {
            return PartyResult::Full;
        }
        self.take_out(who);
        self.add(party, who);
        self.changed();
        self.prune_invites();
        PartyResult::Ok
    }

    pub fn leave(&mut self, who: u64) -> PartyResult {
        if self.party_of(who) == 0 {
            return PartyResult::NotMember;
        }
        self.take_out(who);
        PartyResult::Ok
    }

    pub fn kick(&mut self, leader: u64, target: u64) -> PartyResult {
        let party = self.party_of(leader);
        let Some(p) = self.parties.get(&party) else { return PartyResult::NotMember };
        if p.leader != leader {
            return PartyResult::NotLeader;
        }
        if target == leader {
            return PartyResult::SelfTarget;
        }
        if self.party_of(target) != party {
            return PartyResult::NotMember;
        }
        self.take_out(target);
        PartyResult::Ok
    }

    pub fn promote(&mut self, leader: u64, target: u64) -> PartyResult {
        let party = self.party_of(leader);
        let target_party = self.party_of(target);
        let Some(p) = self.parties.get_mut(&party) else { return PartyResult::NotMember };
        if p.leader != leader {
            return PartyResult::NotLeader;
        }
        if target == leader {
            return PartyResult::SelfTarget;
        }
        if target_party != party {
            return PartyResult::NotMember;
        }
        p.leader = target;
        self.changed();
        PartyResult::Ok
    }

    pub fn set_open(&mut self, leader: u64, open: bool) -> PartyResult {
        let party = self.party_of(leader);
        let Some(p) = self.parties.get_mut(&party) else { return PartyResult::NotMember };
        if p.leader != leader {
            return PartyResult::NotLeader;
        }
        if p.open != open {
            p.open = open;
            self.changed();
        }
        PartyResult::Ok
    }

    // A player left the server: out of their party, and every invite from or to them dropped.
    pub fn remove(&mut self, player: u64) {
        self.take_out(player);
        let invites = std::mem::take(&mut self.invites);
        for i in invites {
            if i.to == player {
                continue;
            }
            if i.from == player {
                self.withdrawn.push(i);
                continue;
            }
            self.invites.push(i);
        }
    }

    // Drops lapsed invites; returns them so their holders can be told.
    pub fn expire(&mut self, now: u64) -> Vec<Invite> {
        let lapsed: Vec<Invite> = self.invites.iter().filter(|i| now >= i.expires).copied().collect();
        self.invites.retain(|i| now < i.expires);
        lapsed
    }

    // The invites dropped since the last call because they can no longer be accepted.
    pub fn take_withdrawn(&mut self) -> Vec<Invite> {
        self.prune_invites();
        std::mem::take(&mut self.withdrawn)
    }
}
