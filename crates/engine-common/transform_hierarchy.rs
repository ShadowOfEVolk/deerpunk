//! World pose from local transform + optional parent.

use glam::{Quat, Vec3};

use crate::components::{ParentComponent, TransformComponent};
use crate::world::{ComponentId, World};

/// One parent link: `world = parent ⊕ local` (Z-up engine convention).
pub fn world_pose(
    world: &World,
    entity: u64,
    local: &TransformComponent,
    parent_id: ComponentId,
) -> (Vec3, Quat) {
    let Some(link) = world.get::<ParentComponent>(entity, parent_id) else {
        return (local.position, local.rotation);
    };
    let tid = world.component_init::<TransformComponent>();
    let Some(parent_t) = world.get::<TransformComponent>(link.parent, tid) else {
        return (local.position, local.rotation);
    };
    let scale = local.scale;
    let scaled = Vec3::new(
        local.position.x * scale.x,
        local.position.y * scale.y,
        local.position.z * scale.z,
    );
    let world_pos = parent_t.position + parent_t.rotation * scaled;
    let world_rot = parent_t.rotation * local.rotation;
    (world_pos, world_rot)
}
