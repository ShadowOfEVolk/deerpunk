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

/// World eye + orientation for any [`CameraComponent`] entity (uses parent chain when linked).
pub fn camera_world_pose(
    world: &World,
    entity: u64,
    local: &TransformComponent,
    parent_id: ComponentId,
) -> (Vec3, Quat) {
    if world.get::<ParentComponent>(entity, parent_id).is_some() {
        world_pose(world, entity, local, parent_id)
    } else {
        (local.position, local.rotation)
    }
}

/// True if `cam_ent` is `root` or a descendant via [`ParentComponent`].
pub fn entity_in_subtree(world: &World, root: u64, cam_ent: u64, parent_id: ComponentId) -> bool {
    let mut e = cam_ent;
    loop {
        if e == root {
            return true;
        }
        let Some(link) = world.get::<ParentComponent>(e, parent_id) else {
            return false;
        };
        e = link.parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::CameraComponent;
    use crate::components::cam_pass;
    use crate::components::camera::{ColorLoad, DepthAttachment, DepthTargetId};
    use crate::world::World;

    fn setup_pawn_view_stack() -> (World, u64, u64, u64, ComponentId) {
        let world = World::new();
        let tid = world.component_init::<TransformComponent>();
        let pid = world.component_init::<ParentComponent>();
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

        let view = world.spawn();
        world.set(
            view,
            tid,
            TransformComponent {
                position: Vec3::new(0.0, 0.0, 1.6),
                rotation: Quat::from_rotation_x(-0.2),
                scale: Vec3::ONE,
            },
        );
        world.set(view, pid, ParentComponent { parent: pawn });
        world.set(view, cid, CameraComponent::default());

        let overlay = world.spawn();
        world.set(
            overlay,
            tid,
            TransformComponent {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        world.set(overlay, pid, ParentComponent { parent: view });
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

        (world, pawn, view, overlay, pid)
    }

    #[test]
    fn camera_world_pose_uses_parent_chain() {
        let (world, _pawn, view, _overlay, pid) = setup_pawn_view_stack();
        let tid = world.component_init::<TransformComponent>();
        let local = world.get::<TransformComponent>(view, tid).unwrap();
        let (pos, _rot) = camera_world_pose(&world, view, &local, pid);
        assert!((pos.x - 100.0).abs() < 1e-4);
        assert!((pos.y - 50.0).abs() < 1e-4);
        assert!((pos.z - 1.6).abs() < 1e-4);
    }

    #[test]
    fn overlay_child_shares_view_eye_position() {
        let (world, _pawn, view, overlay, pid) = setup_pawn_view_stack();
        let tid = world.component_init::<TransformComponent>();
        let view_local = world.get::<TransformComponent>(view, tid).unwrap();
        let overlay_local = world.get::<TransformComponent>(overlay, tid).unwrap();
        let (view_pos, _) = camera_world_pose(&world, view, &view_local, pid);
        let (overlay_pos, _) = camera_world_pose(&world, overlay, &overlay_local, pid);
        assert!((view_pos - overlay_pos).length() < 1e-4);
    }

    #[test]
    fn entity_in_subtree_matches_pawn_hierarchy() {
        let (world, pawn, view, overlay, pid) = setup_pawn_view_stack();
        assert!(entity_in_subtree(&world, pawn, view, pid));
        assert!(entity_in_subtree(&world, pawn, overlay, pid));
        assert!(!entity_in_subtree(&world, view, pawn, pid));
    }

    #[test]
    fn rootless_camera_uses_local_transform_only() {
        let world = World::new();
        let tid = world.component_init::<TransformComponent>();
        let pid = world.component_init::<ParentComponent>();
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
        let (pos, rot) = camera_world_pose(&world, cam, &local, pid);
        assert_eq!(pos, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(rot, Quat::IDENTITY);
    }
}
