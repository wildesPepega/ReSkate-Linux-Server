// Each owner's placed objects (Extension/Multiplayer/Session/object_state.h). A multipart
// replacement or delta becomes visible only after all parts validate.
use crate::protocol::{valid_network_object, valid_object_chunk, NetworkObject, ObjectChunk};
use crate::protocol::{MAX_OWNED_OBJECTS, OBJECT_CHUNK_ENTRIES};
use std::collections::{BTreeMap, BTreeSet};

type Layout = BTreeMap<u64, NetworkObject>;

struct Pending {
    change: ObjectChunk,
    layout: Layout,
    touched: BTreeSet<u64>,
    next: u16,
}

#[derive(PartialEq, Eq)]
pub enum ObjectResult {
    Ignored,
    Pending,
    Applied,
    Invalid,
}

#[derive(Default)]
pub struct ObjectState {
    objects: Layout,
    revision: u64,
    latest: ObjectChunk,
    pending: Option<Pending>,
}

impl ObjectState {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn objects(&self) -> &Layout {
        &self.objects
    }
    pub fn layout(&self) -> Vec<NetworkObject> {
        self.objects.values().cloned().collect()
    }

    pub fn replace(&mut self, objects: &[NetworkObject]) {
        if objects.len() > MAX_OWNED_OBJECTS {
            panic!("Too many owned objects");
        }
        let mut next = Layout::new();
        for object in objects {
            if !valid_network_object(object) || next.insert(object.id, object.clone()).is_some() {
                panic!("Invalid owned object");
            }
        }
        if self.revision != 0 && next == self.objects {
            return;
        }
        if self.revision == u64::MAX {
            panic!("Object revision exhausted");
        }
        self.latest = ObjectChunk { base: self.revision, ..Default::default() };
        self.revision += 1;
        self.latest.revision = self.revision;
        for (id, object) in &next {
            if self.objects.get(id) != Some(object) {
                self.latest.objects.push(object.clone());
            }
        }
        for id in self.objects.keys() {
            if !next.contains_key(id) {
                self.latest.removed.push(*id);
            }
        }
        self.objects = next;
    }

    pub fn updates(&self, since: u64) -> Vec<ObjectChunk> {
        if self.revision == 0 || since == self.revision {
            return Vec::new();
        }
        let change = if since != 0 && since == self.latest.base {
            self.latest.clone()
        } else {
            ObjectChunk { revision: self.revision, objects: self.layout(), ..Default::default() }
        };
        let count = change.objects.len() + change.removed.len();
        let parts = ((count + OBJECT_CHUNK_ENTRIES - 1) / OBJECT_CHUNK_ENTRIES).max(1);
        let mut result = Vec::with_capacity(parts);
        for part in 0..parts {
            let mut chunk = ObjectChunk {
                base: change.base,
                revision: self.revision,
                part: part as u16,
                parts: parts as u16,
                ..Default::default()
            };
            for index in part * OBJECT_CHUNK_ENTRIES..count.min((part + 1) * OBJECT_CHUNK_ENTRIES) {
                if index < change.objects.len() {
                    chunk.objects.push(change.objects[index].clone());
                } else {
                    chunk.removed.push(change.removed[index - change.objects.len()]);
                }
            }
            result.push(chunk);
        }
        result
    }

    pub fn receive(&mut self, chunk: &ObjectChunk) -> ObjectResult {
        if !valid_object_chunk(chunk) {
            return ObjectResult::Invalid;
        }
        if chunk.revision <= self.revision {
            return ObjectResult::Ignored;
        }
        if chunk.part == 0 {
            if chunk.base != 0 && chunk.base != self.revision {
                return ObjectResult::Invalid;
            }
            let mut next = Pending {
                change: ObjectChunk { base: chunk.base, revision: chunk.revision, parts: chunk.parts, ..Default::default() },
                layout: Layout::new(),
                touched: BTreeSet::new(),
                next: 0,
            };
            if chunk.base != 0 {
                next.layout = self.objects.clone();
            }
            self.pending = Some(next);
        }
        let Some(next) = self.pending.as_mut() else {
            return ObjectResult::Invalid;
        };
        if next.next != chunk.part || next.change.base != chunk.base || next.change.revision != chunk.revision || next.change.parts != chunk.parts {
            return ObjectResult::Invalid;
        }
        for object in &chunk.objects {
            if !next.touched.insert(object.id) {
                self.pending = None;
                return ObjectResult::Invalid;
            }
            next.layout.insert(object.id, object.clone());
            next.change.objects.push(object.clone());
        }
        for &id in &chunk.removed {
            if !next.touched.insert(id) {
                self.pending = None;
                return ObjectResult::Invalid;
            }
            next.layout.remove(&id);
            next.change.removed.push(id);
        }
        if next.touched.len() > MAX_OWNED_OBJECTS * 2 {
            self.pending = None;
            return ObjectResult::Invalid;
        }
        next.next += 1;
        if next.next != chunk.parts {
            return ObjectResult::Pending;
        }
        if next.layout.len() > MAX_OWNED_OBJECTS {
            self.pending = None;
            return ObjectResult::Invalid;
        }
        let done = self.pending.take().unwrap();
        self.objects = done.layout;
        self.latest = done.change;
        self.revision = chunk.revision;
        ObjectResult::Applied
    }
}
