//! Client view camera child entity (local offset + pitch/roll on transform).

use deerpunk_engine_common::components::{
    CameraComponent, ParentComponent, TransformComponent, ViewCameraTag,
};
use deerpunk_engine_common::physics::CHAR_EYE_HEIGHT;
use deerpunk_engine_common::transform_hierarchy::world_pose;
use deerpunk_engine_common::world::{ComponentId, World};
use crate::client_engine::LocalViewStamp;
use glam::{EulerRot, Quat, Vec3};

const DEFAULT_FOV_Y: f32 = 75.0_f32.to_radians();

pub fn find_view_camera_child(world: &World, pawn: u64) -> Option<u64> {
    let parent_id = world.component_init::<ParentComponent>();
    let tag_id = world.component_init::<ViewCameraTag>();
    let mut found = None;
    world.for_each_with_component(tag_id, |entity| {
        if found.is_some() {
            return;
        }
        let Some(link) = world.get::<ParentComponent>(entity, parent_id) else {
            return;
        };
        if link.parent == pawn {
            found = Some(entity);
        }
    });
    found
}

/// Spawn `ViewCameraTag` + `CameraComponent` child if the pawn has no view camera yet.
pub fn ensure_view_camera_child(world: &World, pawn: u64, eye_height: f32) -> u64 {
    if let Some(c) = find_view_camera_child(world, pawn) {
        return c;
    }
    let tid = world.component_init::<TransformComponent>();
    let cid = world.component_init::<CameraComponent>();
    let parent_id = world.component_init::<ParentComponent>();
    let tag_id = world.component_init::<ViewCameraTag>();

    let child = world.spawn();
    let eye = if eye_height > 0.0 {
        eye_height
    } else {
        CHAR_EYE_HEIGHT
    };
    world.set(
        child,
        tid,
        TransformComponent {
            position: Vec3::new(0.0, 0.0, eye),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.set(
        child,
        cid,
        CameraComponent {
            fov_y: DEFAULT_FOV_Y,
            near: 0.05,
            far: 512.0,
        },
    );
    world.set(child, parent_id, ParentComponent { parent: pawn });
    world.set(child, tag_id, ViewCameraTag::marker());
    child
}

pub fn camera_entity_for_view(world: &World, view_pawn: Option<u64>, cam_entity: u64) -> bool {
    let Some(pawn) = view_pawn else {
        return true;
    };
    let parent_id = world.component_init::<ParentComponent>();
    world
        .get::<ParentComponent>(cam_entity, parent_id)
        .is_some_and(|p| p.parent == pawn)
}

/// World eye position + view rotation for any camera entity.
pub fn resolve_camera_world(
    world: &World,
    entity: u64,
    local: &TransformComponent,
    _cam: &CameraComponent,
    parent_id: ComponentId,
) -> (Vec3, Quat) {
    if world.has(entity, world.component_init::<ViewCameraTag>())
        || world.get::<ParentComponent>(entity, parent_id).is_some()
    {
        return world_pose(world, entity, local, parent_id);
    }
    (local.position, local.rotation)
}

pub fn camera_world_at_entity(world: &World, entity: u64) -> Option<(Vec3, Quat)> {
    let tid = world.component_init::<TransformComponent>();
    let cid = world.component_init::<CameraComponent>();
    let parent_id = world.component_init::<ParentComponent>();
    let local = world.get::<TransformComponent>(entity, tid)?;
    let cam = world.get::<CameraComponent>(entity, cid)?;
    Some(resolve_camera_world(world, entity, &local, &cam, parent_id))
}

/// Read client view pose from ECS (pawn yaw + view-camera child).
pub fn read_local_view_stamp(world: &World, pawn: u64) -> Option<LocalViewStamp> {
    let cam = find_view_camera_child(world, pawn)?;
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
