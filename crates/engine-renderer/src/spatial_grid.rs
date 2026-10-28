//! Uniform spatial hash — persistent broad-phase for frustum culling.

use std::collections::{HashMap, HashSet};

use crate::aabb::Aabb;

type CellKey = (i32, i32, i32);

fn cell_range(aabb: &Aabb, inv_cell: f32) -> (CellKey, CellKey) {
    let min = aabb.min * inv_cell;
    let max = aabb.max * inv_cell;
    (
        (
            min.x.floor() as i32,
            min.y.floor() as i32,
            min.z.floor() as i32,
        ),
        (
            max.x.floor() as i32,
            max.y.floor() as i32,
            max.z.floor() as i32,
        ),
    )
}

/// Stable-key spatial index (e.g. ECS entity id). Updated incrementally; queried per frame.
#[derive(Clone, Debug, Default)]
pub struct PersistentSpatialIndex {
    cell_size: f32,
    inv_cell: f32,
    bounds: HashMap<u64, Aabb>,
    cells: HashMap<CellKey, Vec<u64>>,
    query_scratch: Vec<u64>,
    /// Last successful [`sync`] fingerprint — skip rebuild when unchanged.
    sync_fingerprint: u64,
}

impl PersistentSpatialIndex {
    pub fn new(cell_size: f32) -> Self {
        let cell_size = cell_size.max(1.0);
        Self {
            cell_size,
            inv_cell: 1.0 / cell_size,
            bounds: HashMap::new(),
            cells: HashMap::new(),
            query_scratch: Vec::new(),
            sync_fingerprint: 0,
        }
    }

    pub fn set_cell_size(&mut self, cell_size: f32) {
        let cell_size = cell_size.max(1.0);
        if (self.cell_size - cell_size).abs() > f32::EPSILON {
            self.cell_size = cell_size;
            self.inv_cell = 1.0 / cell_size;
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.bounds.clear();
        self.cells.clear();
        self.query_scratch.clear();
        self.sync_fingerprint = 0;
    }

    fn fingerprint<I>(objects: I) -> u64
    where
        I: IntoIterator<Item = (u64, Aabb)>,
    {
        let mut h = 0u64;
        let mut n = 0u64;
        for (id, b) in objects {
            n += 1;
            h = h.wrapping_mul(31).wrapping_add(id);
            h ^= b.min.x.to_bits() as u64;
            h ^= b.min.y.to_bits() as u64;
            h ^= b.min.z.to_bits() as u64;
            h ^= b.max.x.to_bits() as u64;
            h ^= b.max.y.to_bits() as u64;
            h ^= b.max.z.to_bits() as u64;
        }
        h.wrapping_mul(31).wrapping_add(n)
    }

    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    /// Insert or move `id` when bounds changed; skip work when unchanged.
    pub fn upsert(&mut self, id: u64, bounds: Aabb) {
        if !bounds.is_valid() {
            self.remove(id);
            return;
        }
        if let Some(prev) = self.bounds.get(&id) {
            if *prev == bounds {
                return;
            }
            let prev = *prev;
            self.remove_from_cells(id, &prev);
        }
        self.bounds.insert(id, bounds);
        self.insert_into_cells(id, &bounds);
    }

    pub fn remove(&mut self, id: u64) {
        if let Some(prev) = self.bounds.remove(&id) {
            self.remove_from_cells(id, &prev);
        }
    }

    /// Drop entries not in `alive`; upsert the rest.
    pub fn sync_slice(&mut self, objects: &[(u64, Aabb)], cell_size: f32) {
        self.set_cell_size(cell_size);
        let fp = Self::fingerprint(objects.iter().copied());
        if fp == self.sync_fingerprint {
            return;
        }
        self.sync_fingerprint = fp;
        let mut alive = HashSet::new();
        for &(id, bounds) in objects {
            alive.insert(id);
            self.upsert(id, bounds);
        }
        let stale: Vec<u64> = self
            .bounds
            .keys()
            .filter(|id| !alive.contains(id))
            .copied()
            .collect();
        for id in stale {
            self.remove(id);
        }
    }

    /// Object ids whose AABB overlaps `region` (broad phase before frustum test).
    pub fn query_region_ids(&mut self, region: &Aabb) -> &[u64] {
        self.query_scratch.clear();
        if !region.is_valid() {
            return &self.query_scratch;
        }
        // Do not walk empty grid cells inside a huge frustum AABB (O(cell³)); test
        // registered objects directly — typical streaming scenes have hundreds, not millions.
        for (&id, bounds) in &self.bounds {
            if bounds.intersects(region) {
                self.query_scratch.push(id);
            }
        }
        &self.query_scratch
    }

    fn insert_into_cells(&mut self, id: u64, bounds: &Aabb) {
        let (c0, c1) = cell_range(bounds, self.inv_cell);
        for cz in c0.2..=c1.2 {
            for cy in c0.1..=c1.1 {
                for cx in c0.0..=c1.0 {
                    self.cells.entry((cx, cy, cz)).or_default().push(id);
                }
            }
        }
    }

    fn remove_from_cells(&mut self, id: u64, bounds: &Aabb) {
        let (c0, c1) = cell_range(bounds, self.inv_cell);
        for cz in c0.2..=c1.2 {
            for cy in c0.1..=c1.1 {
                for cx in c0.0..=c1.0 {
                    if let Some(list) = self.cells.get_mut(&(cx, cy, cz)) {
                        list.retain(|x| *x != id);
                    }
                }
            }
        }
    }
}
