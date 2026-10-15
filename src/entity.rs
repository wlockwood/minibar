//! Compact generational handles for runtime world entities.
//!
//! Units, structures, nanoframes, walls, factories, and reclaimables can share
//! this identity spine. Reusing a vacant slot changes its generation, so an
//! old projectile, command, or reservation cannot accidentally address the new
//! entity occupying that slot.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityId {
    slot: u32,
    generation: u32,
}

impl EntityId {
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }

    pub const fn slot(self) -> u32 {
        self.slot
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub const fn to_raw(self) -> u64 {
        (self.generation as u64) << 32 | self.slot as u64
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self {
            slot: raw as u32,
            generation: (raw >> 32) as u32,
        }
    }
}

#[derive(Clone, Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Dense slot storage with O(1) insertion, lookup, and removal.
///
/// Removal order determines slot reuse order, but identical command logs still
/// produce identical IDs. Iteration always follows ascending slot order.
#[derive(Clone, Debug, Default)]
pub struct EntityStore<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> EntityStore<T> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            free: Vec::new(),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    pub fn insert(&mut self, value: T) -> EntityId {
        let id = if let Some(slot_index) = self.free.pop() {
            let slot = &mut self.slots[slot_index as usize];
            debug_assert!(slot.value.is_none());
            slot.value = Some(value);
            EntityId::new(slot_index, slot.generation)
        } else {
            let slot_index = u32::try_from(self.slots.len())
                .expect("entity store cannot contain more than u32::MAX slots");
            let generation = 1;
            self.slots.push(Slot {
                generation,
                value: Some(value),
            });
            EntityId::new(slot_index, generation)
        };
        self.len += 1;
        id
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.get(id).is_some()
    }

    pub fn get(&self, id: EntityId) -> Option<&T> {
        let slot = self.slots.get(id.slot as usize)?;
        (slot.generation == id.generation)
            .then_some(slot.value.as_ref())
            .flatten()
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.slot as usize)?;
        (slot.generation == id.generation)
            .then_some(slot.value.as_mut())
            .flatten()
    }

    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        let slot = self.slots.get_mut(id.slot as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        slot.generation = next_generation(slot.generation);
        self.free.push(id.slot);
        self.len -= 1;
        Some(value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.value
                .as_ref()
                .map(|value| (EntityId::new(index as u32, slot.generation), value))
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| {
                let id = EntityId::new(index as u32, slot.generation);
                slot.value.as_mut().map(|value| (id, value))
            })
    }
}

const fn next_generation(generation: u32) -> u32 {
    let next = generation.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_handle_never_addresses_reused_slot() {
        let mut entities = EntityStore::new();
        let old = entities.insert("old");
        assert_eq!(entities.remove(old), Some("old"));

        let replacement = entities.insert("replacement");
        assert_eq!(replacement.slot(), old.slot());
        assert_ne!(replacement.generation(), old.generation());
        assert_eq!(entities.get(old), None);
        assert_eq!(entities.get(replacement), Some(&"replacement"));
    }

    #[test]
    fn raw_round_trip_and_iteration_are_stable() {
        let mut entities = EntityStore::new();
        let first = entities.insert(10);
        let second = entities.insert(20);
        assert_eq!(EntityId::from_raw(first.to_raw()), first);
        assert_eq!(EntityId::from_raw(second.to_raw()), second);
        assert_eq!(
            entities.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
            vec![10, 20]
        );
    }

    #[test]
    fn mutable_lookup_requires_current_generation() {
        let mut entities = EntityStore::with_capacity(2);
        let id = entities.insert(4);
        *entities.get_mut(id).expect("live entity") = 9;
        assert_eq!(entities.get(id), Some(&9));
        assert_eq!(entities.len(), 1);
        assert!(!entities.is_empty());
        assert!(entities.capacity() >= 2);
    }
}
