//! World pose from local transform + native flecs `(ChildOf, _)` parent chain.

use glam::{Quat, Vec3};

use crate::components::{TransformComponent, WorldTransformComponent};
use crate::world::{ComponentId, QueryId, World};

/// Full parent chain: `world = parent_n ⊕ … ⊕ local` (Z-up engine convention).
pub fn world_pose(world: &World, entity: u64, local: &TransformComponent) -> (Vec3, Quat) {
    let tid = world.component_init::<TransformComponent>();
    world_pose_with_ids(world, entity, local, tid)
}

/// Like [`world_pose`] but reuses the pre-resolved transform id (hot extract paths).
pub fn world_pose_with_ids(
    world: &World,
    entity: u64,
    local: &TransformComponent,
    transform_id: ComponentId,
) -> (Vec3, Quat) {
    let mut pos = local.position;
    let mut rot = local.rotation;
    let mut scale = local.scale;
    let mut current = entity;
    while let Some(parent_ent) = world.get_parent(current) {
        let Some(parent_t) = world.get::<TransformComponent>(parent_ent, transform_id) else {
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

/// Rows skipped because [`WorldTransformComponent`] already matched computed world pose.
#[derive(Clone, Copy, Debug, Default)]
pub struct SyncWorldTransformStats {
    pub unchanged: u32,
}

/// Propagate local [`TransformComponent`] (+ `(ChildOf, _)` parents) into [`WorldTransformComponent`].
/// Call once per frame **before** render extract (not the renderer's job to walk parents).
pub fn sync_world_transforms(
    world: &mut World,
    q_roots: QueryId,
    q_parented: QueryId,
) -> SyncWorldTransformStats {
    let transform_id = world.component_init::<TransformComponent>();
    let world_id = world.component_init::<WorldTransformComponent>();

    let mut unchanged = 0u32;

    if let Some(batches) = world.query_batches(q_roots) {
        for batch in batches {
            let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
            let transforms = unsafe {
                std::slice::from_raw_parts(
                    batch.columns[0] as *const TransformComponent,
                    batch.count,
                )
            };
            for i in 0..batch.count {
                let ent = entities[i];
                let local = &transforms[i];
                let world_t = WorldTransformComponent::from_transform(local);
                if world.get::<WorldTransformComponent>(ent, world_id) == Some(world_t) {
                    unchanged += 1;
                    continue;
                }
                world.set(ent, world_id, world_t);
            }
        }
    }

    if let Some(batches) = world.query_batches(q_parented) {
        for batch in batches {
            let entities = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
            let transforms = unsafe {
                std::slice::from_raw_parts(
                    batch.columns[0] as *const TransformComponent,
                    batch.count,
                )
            };
            for i in 0..batch.count {
                let ent = entities[i];
                let local = &transforms[i];
                let (pos, rot) = world_pose_with_ids(world, ent, local, transform_id);
                let world_t = WorldTransformComponent {
                    position: pos,
                    rotation: rot,
                    scale: local.scale,
                };
                if world.get::<WorldTransformComponent>(ent, world_id) == Some(world_t) {
                    unchanged += 1;
                    continue;
                }
                world.set(ent, world_id, world_t);
            }
        }
    }

    SyncWorldTransformStats { unchanged }
}

/// World eye + orientation for any [`crate::components::CameraComponent`] entity (uses parent chain when linked).
pub fn camera_world_pose(world: &World, entity: u64, local: &TransformComponent) -> (Vec3, Quat) {
    if world.get_parent(entity).is_some() {
        world_pose(world, entity, local)
    } else {
        (local.position, local.rotation)
    }
}

/// True if `cam_ent` is `root` or a descendant via the native `(ChildOf, _)` chain.
pub fn entity_in_subtree(world: &World, root: u64, cam_ent: u64) -> bool {
    let mut e = cam_ent;
    loop {
        if e == root {
            return true;
        }
        let Some(parent) = world.get_parent(e) else {
            return false;
        };
        e = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::cam_pass;
    use crate::components::camera::{ColorLoad, DepthAttachment, DepthTargetId};
    use crate::components::CameraComponent;
    use crate::world::World;

    fn setup_pawn_view_stack() -> (World, u64, u64, u64) {
        let world = World::new();
        let tid = world.component_init::<TransformComponent>();
        let cid = world.component_init::<CameraComponent>();

        let pawn = world.spawn();
        world.set(
            pawn,
            tid,
            TransformComponent {
                position: Vec3::new(100.0, 50.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );

        let view = world.spawn_child(pawn);
        world.set(
            view,
            tid,
            TransformComponent {
                position: Vec3::new(0.0, 0.0, 1.6),
                rotation: Quat::from_rotation_x(-0.2),
                scale: Vec3::ONE,
            },
        );
        world.set(view, cid, CameraComponent::default());

        let overlay = world.spawn_child(view);
        world.set(
            overlay,
            tid,
            TransformComponent {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        world.set(
            overlay,
            cid,
            CameraComponent {
                layer_mask: crate::components::render_layer::RENDER_LAYER_DEFAULT << 1,
                order: 1,
                pass_mask: cam_pass::DEFAULT_OVERLAY,
                color_load: ColorLoad::Load,
                depth: DepthAttachment::owned(DepthTargetId(1)),
                ..CameraComponent::default()
            },
        );

        (world, pawn, view, overlay)
    }

    #[test]
    fn camera_world_pose_uses_parent_chain() {
        let (world, _pawn, view, _overlay) = setup_pawn_view_stack();
        let tid = world.component_init::<TransformComponent>();
        let local = world.get::<TransformComponent>(view, tid).unwrap();
        let (pos, _rot) = camera_world_pose(&world, view, &local);
        assert!((pos.x - 100.0).abs() < 1e-4);
        assert!((pos.y - 50.0).abs() < 1e-4);
        assert!((pos.z - 1.6).abs() < 1e-4);
    }

    #[test]
    fn overlay_child_shares_view_eye_position() {
        let (world, _pawn, view, overlay) = setup_pawn_view_stack();
        let tid = world.component_init::<TransformComponent>();
        let view_local = world.get::<TransformComponent>(view, tid).unwrap();
        let overlay_local = world.get::<TransformComponent>(overlay, tid).unwrap();
        let (view_pos, _) = camera_world_pose(&world, view, &view_local);
        let (overlay_pos, _) = camera_world_pose(&world, overlay, &overlay_local);
        assert!((view_pos - overlay_pos).length() < 1e-4);
    }

    #[test]
    fn entity_in_subtree_matches_pawn_hierarchy() {
        let (world, pawn, view, overlay) = setup_pawn_view_stack();
        assert!(entity_in_subtree(&world, pawn, view));
        assert!(entity_in_subtree(&world, pawn, overlay));
        assert!(!entity_in_subtree(&world, view, pawn));
    }

    #[test]
    fn rootless_camera_uses_local_transform_only() {
        let world = World::new();
        let tid = world.component_init::<TransformComponent>();
        let cam = world.spawn();
        world.set(
            cam,
            tid,
            TransformComponent {
                position: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let local = world.get::<TransformComponent>(cam, tid).unwrap();
        let (pos, rot) = camera_world_pose(&world, cam, &local);
        assert_eq!(pos, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(rot, Quat::IDENTITY);
    }
}
