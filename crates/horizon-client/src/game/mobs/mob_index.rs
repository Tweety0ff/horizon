pub(super) use horizon_gameplay::mob_index::*;

use horizon_client::fast_hash::FxHashMap;
use std::cell::RefCell;

/// Validated ID-to-position cache for an entity list. Every hit is checked
/// against the live list; a miss rebuilds the table after a reorder/removal.
#[derive(Default)]
pub(super) struct IdIndex {
    slots: RefCell<FxHashMap<u32, u32>>,
}
impl IdIndex {
    pub(super) fn find<T>(&self, items: &[T], id: u32, id_of: impl Fn(&T) -> u32) -> Option<usize> {
        let mut slots = self.slots.borrow_mut();
        if let Some(&at) = slots.get(&id)
            && items.get(at as usize).is_some_and(|item| id_of(item) == id)
        {
            return Some(at as usize);
        }
        let at = items.iter().position(|item| id_of(item) == id)?;
        slots.clear();
        for (i, item) in items.iter().enumerate() {
            slots.entry(id_of(item)).or_insert(i as u32);
        }
        Some(at)
    }
}

/// ID caches for entity lists frequently addressed by network packets.
#[derive(Default)]
pub(super) struct EntityIndices {
    pub(super) dropped: IdIndex,
    pub(super) boats: IdIndex,
    pub(super) orbs: IdIndex,
    pub(super) falling: IdIndex,
    pub(super) tnts: IdIndex,
    pub(super) carts: IdIndex,
    pub(super) projectiles: IdIndex,
    pub(super) tridents: IdIndex,
    pub(super) rockets: IdIndex,
    pub(super) armor_stands: IdIndex,
    pub(super) remote_hooks: IdIndex,
}

#[cfg(test)]
mod id_index_tests {
    use super::IdIndex;

    #[derive(Clone, Copy)]
    struct Row(u32);

    #[test]
    fn cached_lookup_recovers_after_reorder_remove_and_reinsert() {
        let index = IdIndex::default();
        let mut rows = vec![Row(9), Row(3), Row(7)];
        let id = |row: &Row| row.0;

        assert_eq!(index.find(&rows, 3, id), Some(1));
        rows.swap(1, 2);
        assert_eq!(index.find(&rows, 3, id), Some(2));
        rows.remove(2);
        assert_eq!(index.find(&rows, 3, id), None);
        rows.push(Row(3));
        assert_eq!(index.find(&rows, 3, id), Some(2));
    }
}
