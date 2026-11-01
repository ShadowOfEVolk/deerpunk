//! Generic camera helpers (no FPS-specific tags). Games pick cameras via [`CameraComponent::layer_mask`] + hierarchy.

use deerpunk_engine_common::components::{CameraComponent, ParentComponent, TransformComponent};
use deerpunk_engine_common::components::render_layer::{render_layer_visible, RENDER_LAYER_WORLD};
use deerpunk_engine_common::physics::CHAR_EYE_HEIGHT;
use deerpunk_engine_common::transform_hierarchy::{camera_world_pose, entity_in_subtree};
use deerpunk_engine_common::world::World;
use glam::{EulerRot, Quat, Vec3};

use crate::client_engine::LocalViewStamp;

pub fn camera_entity_in_view(
    world: &World,
    view_root: Option<u64>,
    cam_entity: u64,
) -> bool {
    let Some(root) = view_root else {
        return true;
    };
    let parent_id = world.component_init::<ParentComponent>();
    entity_in_subtree(world, root, cam_entity, parent_id)
}

/// Primary world-layer camera under `parent` (lowest entity id wins).
pub fn find_child_camera_with_layer(
    world: &World,
    parent: u64,
    camera_layer_mask: u32,
) -> Option<u64> {
    let parent_id = world.component_init::<ParentComponent>();
    let cid = world.component_init::<CameraComponent>();
    let mut keep = None;
    world.for_each_with_component(cid, |entity| {
        let Some(link) = world.get::<ParentComponent>(entity, parent_id) else {
            return;
        };
        if link.parent != parent {
            return;
        }
        let Some(cam) = world.get::<CameraComponent>(entity, cid) else {
            return;
        };
        if !render_layer_visible(camera_layer_mask, cam.layer_mask) {
            return;
        }
        keep = Some(keep.map_or(entity, |b: u64| b.min(entity)));
    });
    keep
}

pub fn camera_world_at_entity(world: &World, entity: u64) -> Option<(Vec3, Quat)> {
    let tid = world.component_init::<TransformComponent>();
    let cid = world.component_init::<CameraComponent>();
    let parent_id = world.component_init::<ParentComponent>();
    let local = world.get::<TransformComponent>(entity, tid)?;
    let _cam = world.get::<CameraComponent>(entity, cid)?;
    Some(camera_world_pose(world, entity, &local, parent_id))
}

pub fn read_local_view_stamp(world: &World, pawn: u64) -> Option<LocalViewStamp> {
    let cam = find_child_camera_with_layer(world, pawn, RENDER_LAYER_WORLD)?;
    let tid = world.component_init::<TransformComponent>();
    let cid = world.component_init::<CameraComponent>();
    let local = world.get::<TransformComponent>(cam, tid)?;
    let cam_lens = world.get::<CameraComponent>(cam, cid)?;
    let body = world.get::<TransformComponent>(pawn, tid)?;
    let (_, _, z) = body.rotation.to_euler(EulerRot::ZYX);
    let yaw = -z;
    let (pitch, roll, _) = local.rotation.to_euler(EulerRot::ZYX);
    Some(LocalViewStamp {
        yaw,
        pitch,
        roll,
        eye_height: local.position.z,
        fov_y: cam_lens.fov_y,
    })
}

/// Demo / playback: spawn a child camera if none exists (world layer only).
pub fn ensure_child_world_camera(world: &World, parent: u64, eye_height: f32) -> u64 {
    if let Some(c) = find_child_camera_with_layer(world, parent, RENDER_LAYER_WORLD) {
        return c;
    }
    let tid = world.component_init::<TransformComponent>();
    let cid = world.component_init::<CameraComponent>();
    let parent_id = world.component_init::<ParentComponent>();
    let eye = if eye_height > 0.0 {
        eye_height
    } else {
        CHAR_EYE_HEIGHT
    };
    let child = world.spawn();
    world.set(
        child,
        tid,
        TransformComponent {
            position: Vec3::new(0.0, 0.0, eye),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.set(child, cid, CameraComponent::default());
    world.set(child, parent_id, ParentComponent { parent });
    child
}

#[cfg(test)]
mod tests {
    use super::*;
    use deerpunk_engine_common::components::render_layer::RENDER_LAYER_WORLD;
    /// Overlay layer bit (game-defined convention; bit 1).
    const OVERLAY_LAYER: u32 = 1 << 1;

    fn spawn_pawn_with_cameras(world: &World) -> (u64, u64, u64) {
        let tid = world.component_init::<TransformComponent>();
        let pid = world.component_init::<ParentComponent>();
        let cid = world.component_init::<CameraComponent>();

        let pawn = world.spawn();
        world.set(
            pawn,
            tid,
            TransformComponent {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );

        let world_cam = world.spawn();
        world.set(
            world_cam,
            tid,
            TransformComponent {
                position: Vec3::new(0.0, 0.0, 1.6),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        world.set(world_cam, pid, ParentComponent { parent: pawn });
        world.set(
            world_cam,
            cid,
            CameraComponent {
                layer_mask: RENDER_LAYER_WORLD,
                order: 0,
                ..CameraComponent::default()
            },
        );

        let vm_cam = world.spawn();
        world.set(
            vm_cam,
            tid,
            TransformComponent {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        world.set(vm_cam, pid, ParentComponent { parent: world_cam });
        world.set(
            vm_cam,
            cid,
            CameraComponent {
                layer_mask: OVERLAY_LAYER,
                order: 1,
                pass_mask: 0,
                ..CameraComponent::default()
            },
        );

        (pawn, world_cam, vm_cam)
    }

    #[test]
    fn find_child_camera_picks_lowest_entity_with_layer() {
        let world = World::new();
        let (pawn, world_cam, _) = spawn_pawn_with_cameras(&world);
        let found = find_child_camera_with_layer(&world, pawn, RENDER_LAYER_WORLD);
        assert_eq!(found, Some(world_cam));
    }

    #[test]
    fn camera_entity_in_view_requires_pawn_subtree() {
        let world = World::new();
        let (pawn, world_cam, vm_cam) = spawn_pawn_with_cameras(&world);
        assert!(camera_entity_in_view(&world, Some(pawn), world_cam));
        assert!(camera_entity_in_view(&world, Some(pawn), vm_cam));
        let other = world.spawn();
        assert!(!camera_entity_in_view(&world, Some(pawn), other));
    }

    #[test]
    fn ensure_child_world_camera_is_idempotent() {
        let world = World::new();
        let pawn = world.spawn();
        let a = ensure_child_world_camera(&world, pawn, 1.7);
        let b = ensure_child_world_camera(&world, pawn, 1.7);
        assert_eq!(a, b);
    }
}
