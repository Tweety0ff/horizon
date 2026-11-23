use super::{Aabb, ExperienceOrb, ItemEntity, ItemStack, Vec3};
use crate::join_seq::JoinSeq;

/// Dropped items by block cell, for `ItemEntity.mergeWithNeighbours`'s
/// search around an item. Holds indexes into the tick's item list.
#[derive(Default)]
pub struct ItemCells {
    cells: crate::fast_hash::FxHashMap<(i32, i32, i32), Vec<u32>>,
    spare: Vec<Vec<u32>>,
}
impl ItemCells {
    pub fn new(items: &[ItemEntity]) -> Self {
        let mut cells = Self::default();
        cells.rebuild(items);
        cells
    }
    /// Rebuilds per-tick buckets while retaining the previous tick's allocations.
    pub fn rebuild(&mut self, items: &[ItemEntity]) {
        self.clear();
        self.cells.reserve(items.len());
        for (i, item) in items.iter().enumerate() {
            self.add(i, item.position);
        }
    }
    fn clear(&mut self) {
        self.spare.reserve(self.cells.len());
        for (_, mut bucket) in self.cells.drain() {
            bucket.clear();
            self.spare.push(bucket);
        }
    }
    fn cell(p: Vec3) -> (i32, i32, i32) {
        (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    }
    fn add(&mut self, i: usize, at: Vec3) {
        let key = Self::cell(at);
        if !self.cells.contains_key(&key) {
            self.cells.insert(key, self.spare.pop().unwrap_or_default());
        }
        self.cells.get_mut(&key).expect("item cell inserted").push(i as u32);
    }
    pub fn remove(&mut self, i: usize, at: Vec3) {
        let key = Self::cell(at);
        let empty = self.cells.get_mut(&key).is_some_and(|list| {
            list.retain(|&j| j as usize != i);
            list.is_empty()
        });
        if empty && let Some(mut list) = self.cells.remove(&key) {
            list.clear();
            self.spare.push(list);
        }
    }
    pub fn moved(&mut self, i: usize, from: Vec3, to: Vec3) {
        if Self::cell(from) != Self::cell(to) {
            self.remove(i, from);
            self.add(i, to);
        }
    }
    /// The items whose cells could hold a box meeting `area`, in the order
    /// they were added (vanilla lists a section's entities in the order they
    /// entered it).
    fn near(&self, area: Aabb, out: &mut Vec<u32>) {
        out.clear();
        // An item's box reaches 0.125 sideways and 0.25 up from its position.
        let (min, max) = (area.min_position(), area.max_position());
        let (lo, hi) = (
            Self::cell(Vec3::new(min.x - 0.125, min.y - 0.25, min.z - 0.125)),
            Self::cell(Vec3::new(max.x + 0.125, max.y, max.z + 0.125)),
        );
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    if let Some(list) = self.cells.get(&(x, y, z)) {
                        out.extend_from_slice(list);
                    }
                }
            }
        }
        out.sort_unstable();
    }
}
/// `ItemEntity.mergeWithNeighbours` for `items[i]`. When `age_is_ticked`, it
/// has just moved in its tick (its `age` already counts this tick; vanilla's
/// doesn't yet). Teleport arrivals have not ticked. Each
/// mergeable item whose box meets this one's grown by 0.5 sideways takes this
/// stack or gives it its own, the smaller into the larger
/// (`ItemEntity.tryToMerge`), until this item is used up. An emptied item is
/// marked in `removed`.
pub fn merge_with_neighbours<I>(
    items: &mut I,
    removed: &mut [bool],
    cells: &mut ItemCells,
    i: usize,
    age_is_ticked: bool,
    max_stack: &dyn Fn(&ItemStack) -> u32,
) where
    I: AsRef<[ItemEntity]> + std::ops::IndexMut<usize, Output = ItemEntity> + ?Sized,
{
    let mut near = Vec::new();
    merge_with_neighbours_scratch(items, removed, cells, i, age_is_ticked, &mut near, max_stack);
}

/// Variant for tick loops that retain their candidate vector between calls.
pub(crate) fn merge_with_neighbours_scratch<I>(
    items: &mut I,
    removed: &mut [bool],
    cells: &mut ItemCells,
    i: usize,
    age_is_ticked: bool,
    near: &mut Vec<u32>,
    max_stack: &dyn Fn(&ItemStack) -> u32,
) where
    I: AsRef<[ItemEntity]> + std::ops::IndexMut<usize, Output = ItemEntity> + ?Sized,
{
    let age = |items: &[ItemEntity], j: usize| {
        if age_is_ticked && j == i && items[j].age != i16::MIN as i32 {
            items[j].age.wrapping_sub(1)
        } else {
            items[j].age
        }
    };
    if removed[i] || !items[i].mergeable(age(items.as_ref(), i), max_stack(&items[i].stack)) {
        return;
    }
    let area = items[i].bounds().inflate(Vec3::new(0.5, 0.0, 0.5));
    cells.near(area, near);
    for &j in near.iter() {
        let j = j as usize;
        if j == i
            || removed[j]
            || !items[j].bounds().intersects(area)
            || !items[j].mergeable(age(items.as_ref(), j), max_stack(&items[j].stack))
        {
            continue;
        }
        // ItemEntity.areMergable: both fit one stack of the same item and components.
        let (this, other) = (&items[i].stack, &items[j].stack);
        if items[i].target != items[j].target
            || this.count + other.count > max_stack(other)
            || !this.same_item_same_components(other)
        {
            continue;
        }
        let (to, from) = if other.count < this.count {
            (i, j)
        } else {
            (j, i)
        };
        // ItemEntity.merge(toStack, fromStack, 64).
        // A component limit above 64 permits an overstacked target. Vanilla's
        // signed room then moves the excess back into the source stack.
        let room =
            i64::from(max_stack(&items[to].stack).min(64)) - i64::from(items[to].stack.count);
        let moved = room.min(i64::from(items[from].stack.count));
        items[to].stack.count = (i64::from(items[to].stack.count) + moved) as u32;
        items[from].stack.count = (i64::from(items[from].stack.count) - moved) as u32;
        let (delay, from_age, to_age) = (
            items[from].pickup_delay,
            age(items.as_ref(), from),
            age(items.as_ref(), to),
        );
        let to_item = &mut items[to];
        to_item.pickup_delay = to_item.pickup_delay.max(delay);
        // Only the tick caller has already counted this item's next age increment.
        to_item.age = to_age
            .min(from_age)
            .wrapping_add(i32::from(age_is_ticked && to == i));
        if items[from].stack.count == 0 {
            removed[from] = true;
            cells.remove(from, items[from].position);
        }
        if removed[i] {
            return;
        }
    }
}


/// Experience orbs grouped by their bottom-centre position. Exact AABB checks
/// remain in the caller; cells only reduce the number of candidate entities.
#[derive(Default)]
pub struct OrbCells {
    cells: crate::fast_hash::FxHashMap<(i32, i32, i32), Vec<JoinSeq>>,
    spare: Vec<Vec<JoinSeq>>,
}
impl OrbCells {
    const SIDE: f64 = 2.0;
    fn cell(at: Vec3) -> (i32, i32, i32) {
        ((at.x / Self::SIDE).floor() as i32, (at.y / Self::SIDE).floor() as i32, (at.z / Self::SIDE).floor() as i32)
    }
    pub fn rebuild(&mut self, orbs: &[ExperienceOrb]) {
        self.spare.reserve(self.cells.len());
        for (_, mut bucket) in self.cells.drain() {
            bucket.clear();
            self.spare.push(bucket);
        }
        self.cells.reserve(orbs.len());
        for orb in orbs {
            self.insert(orb.join, orb.position);
        }
    }
    pub fn insert(&mut self, join: JoinSeq, at: Vec3) {
        let key = Self::cell(at);
        if !self.cells.contains_key(&key) {
            self.cells.insert(key, self.spare.pop().unwrap_or_default());
        }
        self.cells.get_mut(&key).expect("orb cell inserted").push(join);
    }
    pub fn remove(&mut self, join: JoinSeq, at: Vec3) {
        let key = Self::cell(at);
        let empty = self.cells.get_mut(&key).is_some_and(|list| {
            list.retain(|&entry| entry != join);
            list.is_empty()
        });
        if empty && let Some(mut bucket) = self.cells.remove(&key) {
            bucket.clear();
            self.spare.push(bucket);
        }
    }
    pub fn moved(&mut self, join: JoinSeq, from: Vec3, to: Vec3) {
        if Self::cell(from) != Self::cell(to) {
            self.remove(join, from);
            self.insert(join, to);
        }
    }
    /// Candidate orbs whose boxes may intersect `area`; caller tests exact bounds.
    pub fn near(&self, area: Aabb, out: &mut Vec<JoinSeq>) {
        out.clear();
        let (min, max) = (area.min_position(), area.max_position());
        let lo = Self::cell(Vec3::new(min.x - 0.25, min.y - 0.5, min.z - 0.25));
        let hi = Self::cell(Vec3::new(max.x + 0.25, max.y, max.z + 0.25));
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    if let Some(bucket) = self.cells.get(&(x, y, z)) {
                        out.extend_from_slice(bucket);
                    }
                }
            }
        }
        out.sort_unstable();
    }
}

#[cfg(test)]
mod reusable_cells_tests {
    use super::*;

    #[test]
    fn item_cells_reuse_bucket_allocations_after_rebuild() {
        let mut cells = ItemCells::default();
        cells.add(0, Vec3::new(0.0, 0.0, 0.0));
        let capacity = cells.cells.values().next().unwrap().capacity();
        cells.clear();
        cells.add(1, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(cells.cells.values().next().unwrap().capacity(), capacity);
    }

    #[test]
    fn orb_cells_reuse_buckets_and_follow_moved_orbs() {
        let mut orbs = vec![
            ExperienceOrb::new(1, Vec3::new(0.2, 64.0, 0.2), Vec3::ZERO, 1),
            ExperienceOrb::new(2, Vec3::new(0.4, 64.0, 0.2), Vec3::ZERO, 1),
        ];
        orbs[0].join = JoinSeq::next();
        orbs[1].join = JoinSeq::next();

        let mut cells = OrbCells::default();
        cells.rebuild(&orbs);
        let mut found = Vec::new();
        let area = orbs[0].bounds().inflate(Vec3::new(0.5, 0.5, 0.5));
        cells.near(area, &mut found);
        assert_eq!(found, vec![orbs[0].join, orbs[1].join]);

        let old = orbs[0].position;
        let moved = Vec3::new(4.2, 64.0, 0.2);
        cells.moved(orbs[0].join, old, moved);
        cells.near(area, &mut found);
        assert_eq!(found, vec![orbs[1].join]);

        let moved_area = ExperienceOrb::box_at(moved).inflate(Vec3::new(0.5, 0.5, 0.5));
        cells.near(moved_area, &mut found);
        assert_eq!(found, vec![orbs[0].join]);
    }
}

