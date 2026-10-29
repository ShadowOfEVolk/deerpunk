//! World pose from local transform + parent chain.

use glam::{Quat, Vec3};

use crate::components::{ParentComponent, TransformComponent};
use crate::world::{ComponentId, World};

/// Full parent chain: `world = parent_n ⊕ … ⊕ local` (Z-up engine convention).
pub fn world_pose(
    world: &World,
    entity: u64,
    local: &TransformComponent,
    parent_id: ComponentId,
) -> (Vec3, Quat) {
    let tid = world.component_init::<TransformComponent>();
    let mut pos = local.position;
    let mut rot = local.rotation;
    let mut scale = local.scale;
    let mut current = entity;
    while let Some(link) = world.get::<ParentComponent>(current, parent_id) {
        let parent_ent = link.parent;
        let Some(parent_t) = world.get::<TransformComponent>(parent_ent, tid) else {
            break;
        };
        let scaled = pos * scale;
        pos = parent_t.position + parent_t.rotation * scaled;
        rot = parent_t.rotation * rot;
        scale = parent_t.scale * scale;
        current = parent_ent;
    }
    (pos, rot)
}
