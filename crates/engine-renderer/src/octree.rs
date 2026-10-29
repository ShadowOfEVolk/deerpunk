//! Persistent loose octree for frustum broad-phase (entity-keyed, rebuilt on geom change).

use std::collections::{HashMap, HashSet};

use crate::aabb::Aabb;
use crate::frustum::Frustum;

const MAX_DEPTH: u8 = 8;
const MAX_OBJECTS_PER_LEAF: usize = 8;

#[derive(Clone, Debug)]
struct OctreeNode {
    bounds: Aabb,
    children: [Option<Box<OctreeNode>>; 8],
    objects: Vec<u64>,
}

impl OctreeNode {
    fn empty(bounds: Aabb) -> Self {
        Self {
            bounds,
            children: Default::default(),
            objects: Vec::new(),
        }
    }

    fn is_leaf(&self) -> bool {
        self.children.iter().all(|c| c.is_none())
    }
}

/// Stable-key octree; [`Self::sync_slice`] rebuilds when geometry fingerprint changes.
#[derive(Debug, Default)]
pub struct PersistentOctree {
    root: Option<OctreeNode>,
    objects: HashMap<u64, Aabb>,
    query_scratch: Vec<u64>,
    sync_fingerprint: u64,
}

impl PersistentOctree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn clear(&mut self) {
        self.root = None;
        self.objects.clear();
        self.query_scratch.clear();
        self.sync_fingerprint = 0;
    }

    fn fingerprint(objects: &[(u64, Aabb)]) -> u64 {
        let mut h = 0u64;
        let mut n = 0u64;
        for (id, b) in objects {
            n += 1;
            h = h.wrapping_mul(31).wrapping_add(*id);
            if b.is_valid() {
                h ^= b.min.x.to_bits() as u64;
                h ^= b.min.y.to_bits() as u64;
                h ^= b.min.z.to_bits() as u64;
                h ^= b.max.x.to_bits() as u64;
                h ^= b.max.y.to_bits() as u64;
                h ^= b.max.z.to_bits() as u64;
            }
        }
        h.wrapping_mul(31).wrapping_add(n)
    }

    /// Drop stale entries; rebuild tree when fingerprint changes.
    pub fn sync_slice(&mut self, objects: &[(u64, Aabb)]) {
        let fp = Self::fingerprint(objects);
        if fp == self.sync_fingerprint {
            return;
        }
        self.sync_fingerprint = fp;
        self.objects.clear();
        let mut alive = HashSet::new();
        for &(id, bounds) in objects {
            if bounds.is_valid() {
                alive.insert(id);
                self.objects.insert(id, bounds);
            }
        }
        self.root = None;
        if self.objects.is_empty() {
            return;
        }
        let world = Self::merge_bounds(self.objects.values().copied());
        let entries: Vec<(u64, Aabb)> = self.objects.iter().map(|(&id, &b)| (id, b)).collect();
        self.root = Some(Self::build_node(&entries, world, 0));
    }

    fn merge_bounds(iter: impl Iterator<Item = Aabb>) -> Aabb {
        let mut out = Aabb::EMPTY;
        for b in iter {
            out = out.merge(b);
        }
        out
    }

    fn build_node(entries: &[(u64, Aabb)], bounds: Aabb, depth: u8) -> OctreeNode {
        if entries.is_empty() {
            return OctreeNode::empty(bounds);
        }
        if depth >= MAX_DEPTH || entries.len() <= MAX_OBJECTS_PER_LEAF {
            let mut node = OctreeNode::empty(bounds);
            node.objects = entries.iter().map(|(id, _)| *id).collect();
            return node;
        }

        let center = bounds.center();
        let mut buckets: [Vec<(u64, Aabb)>; 8] = Default::default();
        let mut stragglers = Vec::new();

        for &(id, aabb) in entries {
            let c = aabb.center();
            let mut mask = 0u8;
            if c.x >= center.x {
                mask |= 1;
            }
            if c.y >= center.y {
                mask |= 2;
            }
            if c.z >= center.z {
                mask |= 4;
            }
            let child_bounds = Self::child_bounds(bounds, mask);
            if aabb.min.x >= child_bounds.min.x
                && aabb.min.y >= child_bounds.min.y
                && aabb.min.z >= child_bounds.min.z
                && aabb.max.x <= child_bounds.max.x
                && aabb.max.y <= child_bounds.max.y
                && aabb.max.z <= child_bounds.max.z
            {
                buckets[mask as usize].push((id, aabb));
            } else {
                stragglers.push((id, aabb));
            }
        }

        let mut node = OctreeNode::empty(bounds);
        node.objects = stragglers.iter().map(|(id, _)| *id).collect();
        for mask in 0..8u8 {
            let list = &buckets[mask as usize];
            if list.is_empty() {
                continue;
            }
            let child_bounds = Self::child_bounds(bounds, mask);
            node.children[mask as usize] =
                Some(Box::new(Self::build_node(list, child_bounds, depth + 1)));
        }
        if node.is_leaf() && node.objects.is_empty() && entries.len() <= MAX_OBJECTS_PER_LEAF {
            node.objects = entries.iter().map(|(id, _)| *id).collect();
        }
        node
    }

    fn child_bounds(parent: Aabb, mask: u8) -> Aabb {
        let c = parent.center();
        let min = parent.min;
        let max = parent.max;
        Aabb::from_min_max(
            glam::Vec3::new(
                if mask & 1 != 0 { c.x } else { min.x },
                if mask & 2 != 0 { c.y } else { min.y },
                if mask & 4 != 0 { c.z } else { min.z },
            ),
            glam::Vec3::new(
                if mask & 1 != 0 { max.x } else { c.x },
                if mask & 2 != 0 { max.y } else { c.y },
                if mask & 4 != 0 { max.z } else { c.z },
            ),
        )
    }

    /// Object ids whose bounds may intersect `frustum` (narrow phase still required).
    pub fn query_frustum_ids(&mut self, frustum: &Frustum) -> &[u64] {
        self.query_scratch.clear();
        if let Some(root) = self.root.as_ref() {
            Self::query_node(root, frustum, &self.objects, &mut self.query_scratch);
        }
        &self.query_scratch
    }

    fn query_node(
        node: &OctreeNode,
        frustum: &Frustum,
        objects: &HashMap<u64, Aabb>,
        out: &mut Vec<u64>,
    ) {
        if !frustum.intersects_aabb(&node.bounds) {
            return;
        }
        for &id in &node.objects {
            if let Some(b) = objects.get(&id) {
                if frustum.intersects_aabb(b) {
                    out.push(id);
                }
            }
        }
        for child in node.children.iter().flatten() {
            Self::query_node(child, frustum, objects, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    #[test]
    fn octree_frustum_finds_visible_cluster() {
        let mut tree = PersistentOctree::new();
        let mut objs = Vec::new();
        for i in 0..100i32 {
            let x = (i % 10) as f32 * 40.0;
            let z = (i / 10) as f32 * 40.0;
            let b = Aabb::from_min_max(
                glam::Vec3::new(x, 0.0, z),
                glam::Vec3::new(x + 16.0, 16.0, z + 16.0),
            );
            objs.push((i as u64 + 1, b));
        }
        tree.sync_slice(&objs);
        let aspect = 16.0 / 9.0;
        let view = Mat4::look_at_rh(
            glam::Vec3::new(80.0, 20.0, 80.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let proj = Mat4::perspective_rh(60.0_f32.to_radians(), aspect, 0.1, 500.0);
        let vp = proj * view;
        let frustum = Frustum::from_view_proj(vp);
        let hits = tree.query_frustum_ids(&frustum);
        assert!(hits.len() < 100);
        assert!(!hits.is_empty());
    }
}
