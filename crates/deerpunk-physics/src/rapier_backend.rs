//! Rapier implementation of [`super::backend::PhysicsBackend`].

use glam::Quat;
use rapier3d::control::{CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::{ShapeCastOptions, ShapeCastStatus};
use rapier3d::prelude::*;
use std::collections::HashMap;
use std::ffi::c_void;

use super::abi::{
    decode_shape, Collider, ColliderPart, PartsHandle, ShapeGeom, ShapeHandle, BODY_TYPE_DYNAMIC,
    BODY_TYPE_KINEMATIC, PARTS_HANDLE_INVALID, SHAPE_HANDLE_INVALID, VOXEL_VOLUME_SIZE,
};
use super::backend::{CapsuleCastHit, PhysicsBackend, RayCastHit};
use super::character::CharacterController;
use super::world::{categories, BodyType, PhysicsWorld};
use super::{
    Collider as PodCollider, PhysBodyHandle, RigidBody, TransformView, Velocity,
    SHAPE_DESC_KIND_BALL, SHAPE_DESC_KIND_BOX, SHAPE_DESC_KIND_CAPSULE,
};
use deerpunk_engine_world::{ComponentDescriptor, World};

/// Player capsules: collide with world, not with other players.
fn player_collision_groups() -> InteractionGroups {
    InteractionGroups {
        memberships: Group::from(categories::PLAYER),
        filter: Group::from(!categories::PLAYER),
        test_mode: InteractionTestMode::default(),
    }
}

fn player_query_filter(exclude_body: Option<RigidBodyHandle>) -> QueryFilter<'static> {
    let mut f = QueryFilter::default().groups(player_collision_groups());
    if let Some(rb) = exclude_body {
        f = f.exclude_rigid_body(rb);
    }
    f
}

/// High bit that tags voxel-volume shape handles. `shapes` (Rapier geometry)
/// and `voxel_shapes` (DDA volumes) are separate `Arena`s with independent
/// counters, so their raw indices overlap; the tag keeps the two handle spaces
/// disjoint and tells them apart at every use site.
const VOXEL_HANDLE_FLAG: u32 = 1 << 31;

fn tag_voxel_shape_handle(handle: ShapeHandle) -> ShapeHandle {
    handle | VOXEL_HANDLE_FLAG
}

fn shape_arena_index(handle: ShapeHandle) -> ShapeHandle {
    handle & !VOXEL_HANDLE_FLAG
}

struct Arena<T> {
    slots: Vec<Option<T>>,
    free: Vec<u32>,
}

impl<T> Arena<T> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn insert(&mut self, value: T) -> u32 {
        if let Some(i) = self.free.pop() {
            self.slots[i as usize] = Some(value);
            i + 1
        } else {
            self.slots.push(Some(value));
            self.slots.len() as u32
        }
    }

    fn get(&self, handle: u32) -> Option<&T> {
        if handle == 0 {
            return None;
        }
        self.slots.get((handle - 1) as usize)?.as_ref()
    }

    fn remove(&mut self, handle: u32) -> Option<T> {
        if handle == 0 {
            return None;
        }
        let i = (handle - 1) as usize;
        let v = self.slots.get_mut(i)?.take()?;
        self.free.push(handle - 1);
        Some(v)
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
    }
}

struct CharacterEntry {
    body: RigidBodyHandle,
    collider: ColliderHandle,
    shape: SharedShape,
    /// Half-height + radius (debug / telemetry). Shape is feet-origin (see upsert).
    center_offset: f32,
    step_height: f32,
    max_slope_deg: f32,
    grounded: bool,
}

/// Rapier backend + shape/parts arenas + kinematic characters.
pub struct RapierPhysicsBackend {
    world: PhysicsWorld,
    shapes: Arena<ShapeGeom>,
    parts: Arena<Vec<ColliderPart>>,
    characters: HashMap<u64, CharacterEntry>,
    /// Voxel volume shapes (host-side; not Rapier colliders). Indexed by the
    /// same `ShapeHandle` arena as `shapes` so `create_shape`/`destroy_shape`
    /// stay uniform. A voxel-volume shape handle is present in exactly one of
    /// `shapes` or `voxel_shapes`.
    voxel_shapes: Arena<super::voxel_query::VoxelVolume>,
    /// Entities whose `Collider` is a single voxel-volume part: they have no
    /// bridge body; `cast_ray`/`cast_capsule` query the DDA volumes directly.
    voxel_entities: HashMap<u64, Vec<ShapeHandle>>,
    /// Static Rapier contact bodies for voxel-only entities (parry `Voxels`
    /// shape built from the same bitset the DDA queries sweep). Deliberately
    /// NOT in `body_map`: prepare/writeback key off `body_map` and voxel
    /// entities must stay excluded from that bridge.
    voxel_contact_bodies: HashMap<u64, RigidBodyHandle>,
    /// Flecs world + component ids for observers (ecs_get_id fallback).
    transform_id: u64,
    collider_id: u64,
    char_id: u64,
    rigid_body_id: u64,
    pod_collider_id: u64,
    velocity_id: u64,
    phys_handle_id: u64,
    /// Server ECS world (for Flecs sync systems invoked from prepare/writeback).
    pub(crate) world_view: *const deerpunk_engine_world::World,
    sys_sync_in: u64,
    sys_rigid_body_prepare: u64,
    sys_writeback_characters: u64,
    sys_writeback_dynamics: u64,
    sys_rigid_body_writeback: u64,
    /// Leftover server time for the fixed-timestep physics loop (`advance`).
    step_accumulator: f32,
    /// Last velocity value applied from the `Velocity` component, per entity.
    /// Change detection: the component is only pushed into Rapier when it
    /// differs from this, never compared against the solver-owned body linvel
    /// (which changes legitimately every substep — comparing against it made
    /// prepare resurrect stale writeback values and fight the contacts).
    applied_linvel: HashMap<u64, [f32; 3]>,
    attached: bool,
    /// Contact skin (`target_distance`) for swept capsule casts. Smaller for
    /// voxel (1-unit) scale, larger for HL scale. Set via cvar `ph_capsule_skin`.
    capsule_skin: f32,
}

// Flecs world pointer is only touched on the server sim thread.
unsafe impl Send for RapierPhysicsBackend {}

impl RapierPhysicsBackend {
    pub fn new() -> Self {
        Self {
            world: PhysicsWorld::new(),
            shapes: Arena::new(),
            parts: Arena::new(),
            voxel_shapes: Arena::new(),
            voxel_entities: HashMap::new(),
            voxel_contact_bodies: HashMap::new(),
            characters: HashMap::new(),
            transform_id: 0,
            collider_id: 0,
            char_id: 0,
            rigid_body_id: 0,
            pod_collider_id: 0,
            velocity_id: 0,
            phys_handle_id: 0,
            world_view: std::ptr::null(),
            sys_sync_in: 0,
            sys_rigid_body_prepare: 0,
            sys_writeback_characters: 0,
            sys_writeback_dynamics: 0,
            sys_rigid_body_writeback: 0,
            step_accumulator: 0.0,
            applied_linvel: HashMap::new(),
            attached: false,
            capsule_skin: 0.5,
        }
    }

    fn shared_shape(geom: &ShapeGeom) -> Option<SharedShape> {
        match geom {
            ShapeGeom::Box { half_extents } => Some(SharedShape::cuboid(
                half_extents[0],
                half_extents[1],
                half_extents[2],
            )),
            ShapeGeom::Ball { radius } => Some(SharedShape::ball(*radius)),
            ShapeGeom::Convex { vertices } => {
                if vertices.len() < 4 {
                    log::warn!("[physics] convex needs ≥4 verts, got {}", vertices.len());
                    return None;
                }
                let pts: Vec<Vec3> = vertices
                    .iter()
                    .map(|v| Vec3::new(v[0], v[1], v[2]))
                    .collect();
                match SharedShape::convex_hull(&pts) {
                    Some(s) => Some(s),
                    None => {
                        log::warn!("[physics] convex_hull failed ({} verts)", pts.len());
                        None
                    }
                }
            }
            ShapeGeom::HalfSpace { outward_normal } => {
                let n = Vec3::from_array(*outward_normal);
                let n = if n.length_squared() < 1.0e-8 {
                    Vec3::Z
                } else {
                    n.normalize()
                };
                Some(SharedShape::halfspace(n))
            }
            ShapeGeom::TriMesh { vertices, indices } => {
                if vertices.len() < 3 || indices.len() < 3 || indices.len() % 3 != 0 {
                    log::warn!(
                        "[physics] trimesh needs ≥1 tri ({} verts, {} indices)",
                        vertices.len(),
                        indices.len()
                    );
                    return None;
                }
                let pts: Vec<rapier3d::math::Vector> = vertices
                    .iter()
                    .map(|v| rapier3d::math::Vector::new(v[0], v[1], v[2]))
                    .collect();
                let tris: Vec<[u32; 3]> = indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| [c[0], c[1], c[2]])
                    .collect();
                match SharedShape::trimesh(pts, tris) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        log::warn!("[physics] trimesh build failed: {:?}", e);
                        None
                    }
                }
            }
            ShapeGeom::VoxelVolume { .. } => {
                // Voxel volumes are not Rapier shapes; they live in `voxel_shapes`
                // and are queried by DDA in `cast_ray`/`cast_capsule`.
                None
            }
        }
    }

    /// True if `parts_handle` is a single voxel-volume part (voxel-only path:
    /// DDA queries + static `Voxels` contact body, no bridge body).
    fn is_voxel_only(&self, parts_handle: PartsHandle) -> bool {
        let Some(parts) = self.parts.get(parts_handle) else {
            return false;
        };
        if parts.len() != 1 {
            return false;
        }
        parts[0].shape & VOXEL_HANDLE_FLAG != 0
    }

    /// parry `Voxels` shape for a [`super::voxel_query::VoxelVolume`], with
    /// EXACTLY the mapping `voxel_query` DDA queries use:
    ///
    /// - DDA (`voxel_query`): cell `(x,y,z)` occupies the world AABB
    ///   `[origin + (x,y,z), origin + (x+1,y+1,z+1))`, and bit
    ///   `i = x + y*S + z*S*S` (LSB-first in `solid[i >> 3]`) marks it solid.
    /// - parry (`Voxels::new`): the voxel at grid coord `(x,y,z)` spans the
    ///   shape-local AABB `[(x,y,z) * cell_size, ((x,y,z)+1) * cell_size)`.
    ///
    /// With `cell_size = 1` and the body posed at `volume.origin` with identity
    /// rotation (see `replace_voxel_contact_body`), the two grids coincide
    /// cell-for-cell in world space, so DDA ray hits and contact geometry
    /// describe the same surface. The entity `TransformComponent` and the
    /// `ColliderPart` local pose are deliberately ignored — the DDA path
    /// ignores them too (it only reads `volume.origin`).
    fn voxels_shape(vol: &super::voxel_query::VoxelVolume) -> SharedShape {
        let mut cells = Vec::with_capacity(
            VOXEL_VOLUME_SIZE * VOXEL_VOLUME_SIZE * VOXEL_VOLUME_SIZE / 4,
        );
        for z in 0..VOXEL_VOLUME_SIZE {
            for y in 0..VOXEL_VOLUME_SIZE {
                for x in 0..VOXEL_VOLUME_SIZE {
                    if vol.is_solid(x, y, z) {
                        cells.push(IVector::new(x as i32, y as i32, z as i32));
                    }
                }
            }
        }
        SharedShape::new(Voxels::new(Vector::ONE, &cells))
    }

    /// Rapier compares sleep thresholds against velocity in meters
    /// (`v / IntegrationParameters::length_unit`), while this engine runs all
    /// velocities and gravity in raw world units (length_unit=64 exists only to
    /// scale solver epsilons). The default normalized threshold (0.05) therefore
    /// equals 3.2 units/s and force-sleeps bodies mid-fall (~0.5 s after spawn).
    /// Translate the desired engine-units threshold into the normalized scale.
    fn apply_units_sleep_threshold(&mut self, handle: RigidBodyHandle) {
        const SLEEP_THRESHOLD_UNITS: f32 = 0.2; // units/s, well below walk speed
        let lu = self.world.inner.integration_parameters.length_unit;
        if lu > 0.0 {
            if let Some(body) = self.world.inner.bodies.get_mut(handle) {
                body.activation_mut().normalized_linear_threshold =
                    SLEEP_THRESHOLD_UNITS / lu;
            }
        }
    }

    /// Destroy + recreate the static contact body for a voxel-only entity.
    /// Called on every `PhysCollider` OnSet, so a chunk rebuild (block change)
    /// replaces the contact geometry from the fresh bitset — the same
    /// destroy-then-insert pattern `insert_body` uses for other bodies.
    fn replace_voxel_contact_body(&mut self, entity: u64, collider: &Collider) {
        self.destroy_voxel_contact_body(entity);
        let Some(vol) = self
            .voxel_entities
            .get(&entity)
            .and_then(|handles| handles.first().copied())
            .and_then(|h| self.voxel_shapes.get(shape_arena_index(h)))
            .cloned()
        else {
            return;
        };
        let solid_cells: u32 = vol.solid.iter().map(|b| b.count_ones()).sum();
        // All-empty chunk: nothing to collide with; keep it contact-free.
        if solid_cells == 0 {
            return;
        }
        // Body pose = volume origin (identity rotation): pins the parry grid
        // exactly onto the DDA grid (see `voxels_shape`).
        let origin = Vec3::new(
            vol.origin[0] as f32,
            vol.origin[1] as f32,
            vol.origin[2] as f32,
        );
        let rb = RigidBodyBuilder::fixed().translation(origin);
        let groups = InteractionGroups {
            memberships: Group::from(collider.category_bits),
            filter: Group::from(collider.mask_bits),
            test_mode: InteractionTestMode::default(),
        };
        let cb = ColliderBuilder::new(Self::voxels_shape(&vol))
            .friction(collider.friction)
            .restitution(collider.restitution)
            .collision_groups(groups);
        let (handle, _) = self.world.inner.insert(rb, cb);
        self.voxel_contact_bodies.insert(entity, handle);
        log::info!(
            "[physics] voxel contact body entity={entity:#x} origin={:?} cells={solid_cells} bodies={}",
            vol.origin,
            self.world.inner.bodies.len()
        );
    }

    /// Destroy the voxel contact body of `entity` (no-op when absent).
    fn destroy_voxel_contact_body(&mut self, entity: u64) {
        if let Some(handle) = self.voxel_contact_bodies.remove(&entity) {
            self.world.inner.remove_body(handle);
        }
    }

    fn part_pose(part: &ColliderPart) -> Pose {
        let rot = Quat::from_xyzw(
            part.local_rot[0],
            part.local_rot[1],
            part.local_rot[2],
            part.local_rot[3],
        );
        Pose::from_parts(Vec3::from_array(part.local_pos), rot)
    }

    fn build_shared_shape(&self, parts_handle: PartsHandle) -> Option<SharedShape> {
        if parts_handle == PARTS_HANDLE_INVALID {
            return None;
        }
        let parts = self.parts.get(parts_handle)?;
        if parts.is_empty() {
            return None;
        }
        if parts.len() == 1 {
            let p = &parts[0];
            let geom = self.shapes.get(p.shape)?;
            let shape = Self::shared_shape(geom)?;
            if p.local_pos == [0.0; 3] && p.local_rot == [0.0, 0.0, 0.0, 1.0] {
                return Some(shape);
            }
            return Some(SharedShape::compound(vec![(Self::part_pose(p), shape)]));
        }
        let mut compound = Vec::with_capacity(parts.len());
        for p in parts {
            let Some(geom) = self.shapes.get(p.shape) else {
                log::warn!("[physics] missing shape handle {}", p.shape);
                continue;
            };
            let Some(shape) = Self::shared_shape(geom) else {
                continue;
            };
            compound.push((Self::part_pose(p), shape));
        }
        if compound.is_empty() {
            None
        } else {
            Some(SharedShape::compound(compound))
        }
    }

    fn body_type(t: u32) -> BodyType {
        match t {
            BODY_TYPE_DYNAMIC => BodyType::Dynamic,
            BODY_TYPE_KINEMATIC => BodyType::Kinematic,
            _ => BodyType::Static,
        }
    }

    fn insert_body(
        &mut self,
        entity: u64,
        position: [f32; 3],
        rotation_xyzw: [f32; 4],
        collider: &Collider,
        shape: SharedShape,
    ) {
        self.world.destroy_body(entity);

        let rot = Quat::from_xyzw(
            rotation_xyzw[0],
            rotation_xyzw[1],
            rotation_xyzw[2],
            rotation_xyzw[3],
        );
        let pose = Pose::from_parts(Vec3::from_array(position), rot);

        let rb = match Self::body_type(collider.body_type) {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        }
        .pose(pose);

        let groups = InteractionGroups {
            memberships: Group::from(collider.category_bits),
            filter: Group::from(collider.mask_bits),
            test_mode: InteractionTestMode::default(),
        };

        let mut cb = ColliderBuilder::new(shape)
            .friction(collider.friction)
            .restitution(collider.restitution)
            .collision_groups(groups);
        if collider.is_sensor() {
            cb = cb.sensor(true);
        }
        if collider.density > 0.0 {
            cb = cb.density(collider.density);
        }

        let (handle, _) = self.world.inner.insert(rb, cb);
        self.world.body_map.insert(entity, handle);
        self.apply_units_sleep_threshold(handle);
        // Only real Rapier bodies land here (voxel volumes and characters take
        // other paths), so this stays one line per dynamic entity, not spam.
        log::info!(
            "[physics] dynamic body entity={entity:#x} handle={handle:?} bodies={}",
            self.world.inner.bodies.len()
        );
    }

    fn upsert_character(&mut self, entity: u64, position: [f32; 3], cc: &CharacterController) {
        self.remove_character(entity);

        let radius = cc.radius.max(0.1);
        let half_height = cc.half_height.max(0.1);
        let center_offset = half_height + radius;
        // Centered capsule (KCC-stable). Body/ECS = feet; collider + move_shape use +Z offset.
        let shape = SharedShape::capsule_z(half_height, radius);
        let feet = Vec3::from_array(position);
        let pose = Pose::from_parts(feet, Quat::IDENTITY);

        let rb = RigidBodyBuilder::kinematic_position_based().pose(pose);
        let cb = ColliderBuilder::new(shape.clone())
            .position(Pose::from_parts(
                Vec3::new(0.0, 0.0, center_offset),
                Quat::IDENTITY,
            ))
            .friction(0.0)
            .restitution(0.0)
            .collision_groups(player_collision_groups())
            .solver_groups(player_collision_groups())
            .active_events(ActiveEvents::COLLISION_EVENTS);

        let (body, collider) = self.world.inner.insert(rb, cb);
        self.world.body_map.insert(entity, body);
        self.characters.insert(
            entity,
            CharacterEntry {
                body,
                collider,
                shape,
                center_offset,
                step_height: cc.step_height.max(0.0),
                max_slope_deg: cc.max_slope_deg.max(1.0),
                grounded: false,
            },
        );
        log::info!(
            "[physics] character entity={entity:#x} feet={:?} center_z={:.1} (bodies={})",
            position,
            position[2] + center_offset,
            self.world.inner.bodies.len()
        );
    }

    fn remove_character(&mut self, entity: u64) {
        if let Some(entry) = self.characters.remove(&entity) {
            self.world.inner.remove_body(entry.body);
            self.world.body_map.remove(&entity);
            let _ = entry.collider;
        }
    }

    /// Same shape as Rapier docs / bevy_rapier examples, Z-up + HL-ish absolutes.
    fn kcc_for(entry: &CharacterEntry) -> KinematicCharacterController {
        KinematicCharacterController {
            up: Vec3::Z,
            offset: CharacterLength::Absolute(0.5),
            slide: true,
            autostep: Some(rapier3d::control::CharacterAutostep {
                max_height: CharacterLength::Absolute(entry.step_height.clamp(1.0, 18.0)),
                min_width: CharacterLength::Absolute(2.0),
                include_dynamic_bodies: false,
            }),
            snap_to_ground: Some(CharacterLength::Absolute(8.0)),
            max_slope_climb_angle: entry.max_slope_deg.to_radians(),
            min_slope_slide_angle: (entry.max_slope_deg * 0.5).to_radians(),
            ..Default::default()
        }
    }

    /// Rebuild BVH leaves for every collider so `query_pipeline` sees OnSet bodies
    /// even before / between physics steps.
    fn entity_for_collider(&self, ch: ColliderHandle) -> u64 {
        let colliders = &self.world.inner.colliders;
        let Some(collider) = colliders.get(ch) else {
            return 0;
        };
        let Some(rb) = collider.parent() else {
            return 0;
        };
        for (entity, handle) in &self.world.body_map {
            if *handle == rb {
                return *entity;
            }
        }
        // Voxel contact bodies live outside `body_map` (they must stay out of
        // the prepare/writeback bridge); resolve them here so Rapier-side
        // ray/capsule hits attribute the same chunk entity as the DDA path.
        for (entity, handle) in &self.voxel_contact_bodies {
            if *handle == rb {
                return *entity;
            }
        }
        0
    }
    /// Per-entity sync (Flecs system `PhysicsSyncIn`).
    pub(crate) fn integrate_character(&mut self, world: &World, entity: u64, dt: f32) {
        let Some(entry) = self.characters.get(&entity) else {
            return;
        };
        let cc_id = world.component_init::<CharacterController>();
        let t_ptr = world.get_raw(entity, self.transform_id);
        if t_ptr.is_null() {
            return;
        }
        let Some(cc) = world.get::<CharacterController>(entity, cc_id) else {
            return;
        };
        let t = unsafe { &*(t_ptr as *const TransformView) };
        let body = entry.body;
        let shape = entry.shape.clone();
        let center_offset = entry.center_offset.max(cc.center_offset());
        let controller = Self::kcc_for(entry);
        let noclip = cc.noclip();
        let manual = cc.manual_move();

        let Some(rb) = self.world.inner.bodies.get_mut(body) else {
            return;
        };
        let feet = t.position;
        let rot = if manual { t.rotation } else { Quat::IDENTITY };
        rb.set_position(Pose::from_parts(feet, rot), true);

        if manual {
            if let Some(entry) = self.characters.get_mut(&entity) {
                entry.grounded = cc.grounded != 0;
                entry.center_offset = center_offset;
                entry.step_height = cc.step_height.max(0.0);
                entry.max_slope_deg = cc.max_slope_deg.max(1.0);
            }
            return;
        }

        let desired = Vec3::from_array(cc.velocity) * dt;
        let (new_feet, grounded) = if noclip {
            (feet + desired, false)
        } else {
            let mut center = feet + Vec3::new(0.0, 0.0, center_offset);
            let filter = player_query_filter(Some(body));
            let queries = self.world.inner.query_pipeline_with_filter(filter);
            let movement = controller.move_shape(
                dt,
                &queries,
                shape.as_ref(),
                &Pose::from_parts(center, Quat::IDENTITY),
                desired,
                |_| {},
            );
            center += movement.translation;
            let grounded = movement.grounded && cc.velocity[2] <= 0.0;
            (center - Vec3::new(0.0, 0.0, center_offset), grounded)
        };

        if let Some(rb) = self.world.inner.bodies.get_mut(body) {
            rb.set_position(Pose::from_parts(new_feet, Quat::IDENTITY), true);
        }
        if let Some(entry) = self.characters.get_mut(&entity) {
            entry.grounded = grounded;
            entry.center_offset = center_offset;
            entry.step_height = cc.step_height.max(0.0);
            entry.max_slope_deg = cc.max_slope_deg.max(1.0);
        }
    }

    pub(crate) fn writeback_character(&mut self, world: &World, entity: u64) {
        let Some(entry) = self.characters.get(&entity) else {
            return;
        };
        let cc_id = world.component_init::<CharacterController>();
        let ptr = world.get_raw(entity, cc_id);
        let manual = if !ptr.is_null() {
            unsafe { (*(ptr as *const CharacterController)).manual_move() }
        } else {
            false
        };
        if manual {
            return;
        }
        let Some(rb) = self.world.inner.bodies.get(entry.body) else {
            return;
        };
        let feet = rb.translation();
        if let Some(mut t) = world.get::<TransformView>(entity, self.transform_id) {
            t.position = feet;
            world.set(entity, self.transform_id, t);
        }
        if !ptr.is_null() {
            let cc = unsafe { &mut *(ptr as *mut CharacterController) };
            cc.grounded = u32::from(entry.grounded);
            if entry.grounded && !cc.noclip() && cc.velocity[2] < 0.0 {
                cc.velocity[2] = cc.velocity[2].clamp(-80.0, 0.0);
            }
        }
    }

    pub(crate) fn writeback_dynamic(&mut self, world: &World, entity: u64) {
        if self.characters.contains_key(&entity) || self.voxel_entities.contains_key(&entity) {
            return;
        }
        let Some(&handle) = self.world.body_map.get(&entity) else {
            return;
        };
        let Some(rb) = self.world.inner.bodies.get(handle) else {
            return;
        };
        if !rb.is_dynamic() {
            return;
        }
        if let Some(mut t) = world.get::<TransformView>(entity, self.transform_id) {
            t.position = rb.translation();
            t.rotation = *rb.rotation();
            world.set(entity, self.transform_id, t);
        }
    }

    /// Bridge prepare pass for POD `RigidBody` entities (manual system, query
    /// `"RigidBody, TransformComponent"`). Prefab appliers `World::set` components in
    /// arbitrary order, so `RigidBody` may land before its `TransformComponent`;
    /// instead of an OnSet observer, this runs every prepare and:
    /// 1. creates any missing Rapier body (idempotent via body_map /
    ///    character / voxel checks) from Transform + `Collider` + `Velocity`,
    /// 2. teleports static/kinematic bodies to Transform (Transform authority),
    /// 3. writes `Velocity.linear` → body linvel for dynamic bodies (wakes).
    pub(crate) fn prepare_rigid_body(&mut self, world: &World, entity: u64) {
        let Some(rb_desc) = world.get::<RigidBody>(entity, self.rigid_body_id) else {
            return;
        };
        let t_ptr = world.get_raw(entity, self.transform_id);
        if t_ptr.is_null() {
            return;
        }
        let t = unsafe { &*(t_ptr as *const TransformView) };
        if self.world.body_map.contains_key(&entity)
            || self.characters.contains_key(&entity)
            || self.voxel_entities.contains_key(&entity)
            || self.voxel_contact_bodies.contains_key(&entity)
        {
            // Body already exists: static/kinematic teleports to Transform,
            // dynamic gets its Velocity component copied in (set_linvel wakes).
            let Some(&handle) = self.world.body_map.get(&entity) else {
                return;
            };
            let Some(body) = self.world.inner.bodies.get_mut(handle) else {
                return;
            };
            match rb_desc.body_type {
                BODY_TYPE_DYNAMIC => {
                    // Apply the Velocity component only when gameplay CHANGED it
                    // since the last application (change detection against the
                    // last-applied value, not the solver-owned body linvel).
                    if let Some(v) = world.get::<Velocity>(entity, self.velocity_id) {
                        if self.applied_linvel.get(&entity) != Some(&v.linear) {
                            body.set_linvel(Vec3::from_array(v.linear), true);
                            self.applied_linvel.insert(entity, v.linear);
                        }
                    }
                }
                _ => {
                    // Static/kinematic: Transform is authority, but skip the
                    // teleport when the body already matches (keeps fixed
                    // geometry out of the broad-phase update path).
                    let moved = (t.position - body.translation()).length_squared() > 1e-12
                        || 1.0 - body.rotation().dot(t.rotation) > 1e-6;
                    if moved {
                        body.set_position(Pose::from_parts(t.position, t.rotation), true);
                    }
                }
            }
            return;
        }
        // Fresh creation: shape comes from the POD `Collider` if present.
        let pod_collider = world.get::<PodCollider>(entity, self.pod_collider_id);
        let initial_vel = world
            .get::<Velocity>(entity, self.velocity_id)
            .map(|v| v.linear);
        let shape = pod_collider.as_ref().and_then(pod_shared_shape);
        let rot = t.rotation.to_array();
        let handle = self.insert_rigid_body(
            entity,
            t.position.to_array(),
            [rot[0], rot[1], rot[2], rot[3]],
            &rb_desc,
            pod_collider.as_ref(),
            shape,
        );
        world.set(
            entity,
            self.phys_handle_id,
            PhysBodyHandle(PhysBodyHandle::encode(handle)),
        );
        // Initial linear velocity from `Velocity` if present (dynamic only).
        if rb_desc.body_type == BODY_TYPE_DYNAMIC {
            if let Some(linvel) = initial_vel {
                if let Some(body) = self.world.inner.bodies.get_mut(handle) {
                    body.set_linvel(Vec3::from_array(linvel), true);
                }
                self.applied_linvel.insert(entity, linvel);
            }
        }
    }

    /// Bridge writeback pass for POD `RigidBody` entities (manual system, query
    /// `"RigidBody, TransformComponent"`). Dynamic bodies copy pose → `TransformView`
    /// and body linvel → `Velocity`.
    pub(crate) fn writeback_rigid_body(&mut self, world: &World, entity: u64) {
        let Some(rb_desc) = world.get::<RigidBody>(entity, self.rigid_body_id) else {
            return;
        };
        if rb_desc.body_type != BODY_TYPE_DYNAMIC {
            return;
        }
        let Some(&handle) = self.world.body_map.get(&entity) else {
            return;
        };
        let Some(body) = self.world.inner.bodies.get(handle) else {
            return;
        };
        let linvel = body.linvel();
        let angvel = body.angvel();
        if let Some(mut t) = world.get::<TransformView>(entity, self.transform_id) {
            // Skip the write when the body did not move (bit-identical while
            // asleep): avoids per-frame component churn for resting bodies.
            if t.position != body.translation() || t.rotation != *body.rotation() {
                t.position = body.translation();
                t.rotation = *body.rotation();
                world.set(entity, self.transform_id, t);
            }
        }
        if let Some(mut v) = world.get::<Velocity>(entity, self.velocity_id) {
            let lin = [linvel.x, linvel.y, linvel.z];
            let ang = [angvel.x, angvel.y, angvel.z];
            if v.linear != lin || v.angular != ang {
                v.linear = lin;
                v.angular = ang;
                world.set(entity, self.velocity_id, v);
            }
        }
        // The component now mirrors the solver-owned linvel; record it as the
        // last-applied value so prepare does not treat the writeback as a
        // gameplay change and re-apply it over the next substeps.
        self.applied_linvel
            .insert(entity, [linvel.x, linvel.y, linvel.z]);
    }

    /// Insert a Rapier rigid body from the bridge's POD descriptor path
    /// (`RigidBody` + optional `Collider` component). Returns the encoded
    /// Rapier handle for `PhysBodyHandle`.
    fn insert_rigid_body(
        &mut self,
        entity: u64,
        position: [f32; 3],
        rotation_xyzw: [f32; 4],
        rb_desc: &RigidBody,
        collider: Option<&PodCollider>,
        shape: Option<SharedShape>,
    ) -> rapier3d::prelude::RigidBodyHandle {
        self.world.destroy_body(entity);
        let rot = Quat::from_xyzw(
            rotation_xyzw[0],
            rotation_xyzw[1],
            rotation_xyzw[2],
            rotation_xyzw[3],
        );
        let pose = Pose::from_parts(Vec3::from_array(position), rot);
        let rb = match rb_desc.body_type {
            BODY_TYPE_DYNAMIC => RigidBodyBuilder::dynamic(),
            BODY_TYPE_KINEMATIC => RigidBodyBuilder::kinematic_position_based(),
            _ => RigidBodyBuilder::fixed(),
        }
        .pose(pose)
        .gravity_scale(rb_desc.gravity_scale)
        .linear_damping(rb_desc.linear_damping)
        .angular_damping(rb_desc.angular_damping)
        .can_sleep(rb_desc.can_sleep != 0);
        let rb = if rb_desc.ccd_enabled != 0 {
            rb.ccd_enabled(true)
        } else {
            rb
        };
        let Some(shape) = shape else {
            let handle = self.world.inner.insert_body(rb);
            self.world.body_map.insert(entity, handle);
            self.apply_units_sleep_threshold(handle);
            log::info!(
                "[physics] bridge body entity={entity:#x} handle={handle:?} (no collider)"
            );
            return handle;
        };
        let Some(c) = collider else {
            let handle = self.world.inner.insert_body(rb);
            self.world.body_map.insert(entity, handle);
            self.apply_units_sleep_threshold(handle);
            log::info!("[physics] bridge body entity={entity:#x} handle={handle:?} (no collider)");
            return handle;
        };
        let groups = InteractionGroups {
            memberships: Group::from(c.category_bits),
            filter: Group::from(c.mask_bits),
            test_mode: InteractionTestMode::default(),
        };
        let mut cb = ColliderBuilder::new(shape)
            .friction(c.friction)
            .restitution(c.restitution)
            .collision_groups(groups);
        if c.sensor != 0 {
            cb = cb.sensor(true);
        }
        if c.density > 0.0 {
            cb = cb.density(c.density);
        }
        let (handle, _) = self.world.inner.insert(rb, cb);
        self.world.body_map.insert(entity, handle);
        self.apply_units_sleep_threshold(handle);
        log::info!("[physics] bridge body entity={entity:#x} handle={handle:?}");
        handle
    }

    pub(crate) fn set_sync_system_ids(
        &mut self,
        rigid_body_prepare: u64,
        sync_in: u64,
        writeback_characters: u64,
        writeback_dynamics: u64,
        rigid_body_writeback: u64,
    ) {
        self.sys_rigid_body_prepare = rigid_body_prepare;
        self.sys_sync_in = sync_in;
        self.sys_writeback_characters = writeback_characters;
        self.sys_writeback_dynamics = writeback_dynamics;
        self.sys_rigid_body_writeback = rigid_body_writeback;
    }

    /// Sweep all registered voxel volumes with a ray; return the closest hit.
    fn cast_ray_volumes(
        &self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_dist: f32,
    ) -> Option<RayCastHit> {
        let mut best: Option<RayCastHit> = None;
        for (entity, handles) in &self.voxel_entities {
            for &h in handles {
                let Some(vol) = self.voxel_shapes.get(shape_arena_index(h)) else {
                    continue;
                };
                if let Some(hit) =
                    super::voxel_query::cast_ray_volume(vol, origin, dir, max_dist, *entity)
                {
                    let better = match &best {
                        None => true,
                        Some(b) => hit.fraction < b.fraction,
                    };
                    if better {
                        best = Some(hit);
                    }
                }
            }
        }
        best
    }

    /// Sweep all registered voxel volumes with a swept AABB (capsule approx);
    /// return the closest hit.
    fn cast_capsule_volumes(
        &self,
        center: [f32; 3],
        half_height: f32,
        radius: f32,
        dir: [f32; 3],
        max_dist: f32,
    ) -> Option<CapsuleCastHit> {
        let mut best: Option<CapsuleCastHit> = None;
        for (entity, handles) in &self.voxel_entities {
            for &h in handles {
                let Some(vol) = self.voxel_shapes.get(shape_arena_index(h)) else {
                    continue;
                };
                if let Some(hit) = super::voxel_query::cast_capsule_volume(
                    vol,
                    center,
                    half_height,
                    radius,
                    dir,
                    max_dist,
                    *entity,
                ) {
                    let better = match &best {
                        None => true,
                        Some(b) => hit.toi < b.toi,
                    };
                    if better {
                        best = Some(hit);
                    }
                }
            }
        }
        best
    }
}

/// Merge Rapier + voxel ray hits: keep the closer fraction.
fn merge_ray(a: Option<RayCastHit>, b: Option<RayCastHit>) -> Option<RayCastHit> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if x.fraction <= y.fraction { x } else { y }),
        (x, y) => x.or(y),
    }
}

/// Merge Rapier + voxel capsule hits: keep the smaller toi; `start_solid` wins.
fn merge_capsule(a: Option<CapsuleCastHit>, b: Option<CapsuleCastHit>) -> Option<CapsuleCastHit> {
    match (a, b) {
        (Some(x), Some(y)) => {
            if x.start_solid || y.start_solid {
                let mut z = if x.toi <= y.toi { x } else { y };
                z.start_solid = true;
                Some(z)
            } else {
                Some(if x.toi <= y.toi { x } else { y })
            }
        }
        (x, y) => x.or(y),
    }
}

impl Default for RapierPhysicsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsBackend for RapierPhysicsBackend {
    fn init(&mut self) -> bool {
        true
    }

    fn shutdown(&mut self) {
        let ents: Vec<u64> = self.characters.keys().copied().collect();
        for e in ents {
            self.remove_character(e);
        }
        self.shapes.clear();
        self.voxel_shapes.clear();
        self.voxel_entities.clear();
        self.voxel_contact_bodies.clear();
        self.applied_linvel.clear();
        self.parts.clear();
        self.world.body_map.clear();
        self.world.tile_body_map.clear();
        self.world.vehicle_map.clear();
        self.attached = false;
    }

    fn create_shape(&mut self, abi: &[u8]) -> ShapeHandle {
        match decode_shape(abi) {
            Some(ShapeGeom::VoxelVolume { origin, solid }) => tag_voxel_shape_handle(
                self.voxel_shapes
                    .insert(super::voxel_query::VoxelVolume { origin, solid }),
            ),
            Some(geom) => self.shapes.insert(geom),
            None => {
                log::warn!("[physics] create_shape: bad ABI ({} bytes)", abi.len());
                SHAPE_HANDLE_INVALID
            }
        }
    }

    fn destroy_shape(&mut self, handle: ShapeHandle) {
        if handle & VOXEL_HANDLE_FLAG != 0 {
            let _ = self.voxel_shapes.remove(shape_arena_index(handle));
        } else {
            let _ = self.shapes.remove(handle);
        }
    }

    fn create_parts(&mut self, parts: &[ColliderPart]) -> PartsHandle {
        if parts.is_empty() {
            return PARTS_HANDLE_INVALID;
        }
        self.parts.insert(parts.to_vec())
    }

    fn destroy_parts(&mut self, handle: PartsHandle) {
        let _ = self.parts.remove(handle);
    }

    fn debug_shape(&self, handle: ShapeHandle) -> Option<ShapeGeom> {
        self.shapes.get(handle).cloned()
    }

    fn debug_parts(&self, handle: PartsHandle) -> Option<Vec<ColliderPart>> {
        self.parts.get(handle).cloned()
    }

    fn cast_capsule(
        &mut self,
        center: [f32; 3],
        half_height: f32,
        radius: f32,
        dir: [f32; 3],
        max_dist: f32,
        exclude_entity: u64,
    ) -> Option<CapsuleCastHit> {
        let half_height = half_height.max(0.1);
        let radius = radius.max(0.1);
        let max_dist = max_dist.max(0.0);
        if max_dist < 1e-8 {
            return None;
        }
        let mut d = Vec3::from_array(dir);
        let dlen = d.length();
        if dlen < 1e-8 {
            return None;
        }
        d /= dlen;


        let shape = SharedShape::capsule_z(half_height, radius);
        let pose = Pose::from_parts(Vec3::from_array(center), Quat::IDENTITY);
        let exclude = if exclude_entity != 0 {
            self.world.body_map.get(&exclude_entity).copied()
        } else {
            None
        };
        // Same groups as character colliders: hit terrain/etc., skip other players.
        let filter = player_query_filter(exclude);
        let queries = self.world.inner.query_pipeline_with_filter(filter);
        // Contact skin so slide casts don't leave the capsule embedded
        // (HL DIST_EPSILON / Rapier KCC offset). stop_at_penetration=false lets
        // shallow overlaps resolve instead of toi=0 forever.
        //
        // The skin is configurable via cvar `ph_capsule_skin` (default 0.5).
        // HL scale: 0.5 is a tiny fraction of a ~72-unit player. Voxel scale
        // (1 block = 1 unit): 0.5 is half a block — far too large, it makes
        // every ground trace report `start_solid` and floats the player ~1
        // unit above the floor. A voxel game sets a small skin (e.g. 0.02).
        let skin = self.capsule_skin.max(0.0);
        let opts = ShapeCastOptions {
            max_time_of_impact: max_dist,
            target_distance: skin,
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: true,
        };
        // QueryPipeline: normal1/witness1 = hit collider, world space.
        let rapier_hit = queries
            .cast_shape(&pose, d, shape.as_ref(), opts)
            .map(|(ch, hit)| {
                let toi = hit.time_of_impact;
                let start_solid =
                    matches!(hit.status, ShapeCastStatus::PenetratingOrWithinTargetDist);
                let n = hit.normal1;
                let point = hit.witness1;
                CapsuleCastHit {
                    toi,
                    normal: [n.x, n.y, n.z],
                    point: [point.x, point.y, point.z],
                    start_solid,
                    entity: self.entity_for_collider(ch),
                }
            });
        // Merge with voxel-volume hits (DDA, no Rapier).
        let voxel_hit = self.cast_capsule_volumes(center, half_height, radius, dir, max_dist);
        merge_capsule(rapier_hit, voxel_hit)
    }

    fn cast_ray(
        &mut self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_dist: f32,
        exclude_entity: u64,
    ) -> Option<RayCastHit> {
        let max_dist = max_dist.max(0.0);
        if max_dist < 1e-8 {
            return None;
        }
        let mut d = Vec3::from_array(dir);
        let dlen = d.length();
        if dlen < 1e-8 {
            return None;
        }
        d /= dlen;


        let exclude = if exclude_entity != 0 {
            self.world.body_map.get(&exclude_entity).copied()
        } else {
            None
        };
        let filter = player_query_filter(exclude);
        let queries = self.world.inner.query_pipeline_with_filter(filter);
        let ray = Ray::new(Vec3::from_array(origin), d);
        let rapier_hit = queries
            .cast_ray_and_get_normal(&ray, max_dist, true)
            .map(|(ch, hit)| {
                let toi = hit.time_of_impact;
                let n = hit.normal;
                let point = ray.point_at(toi);
                RayCastHit {
                    fraction: if max_dist > 0.0 { toi / max_dist } else { 1.0 },
                    normal: [n.x, n.y, n.z],
                    point: [point.x, point.y, point.z],
                    entity: self.entity_for_collider(ch),
                }
            });
        // Merge with voxel-volume hits (DDA, no Rapier).
        let voxel_hit = self.cast_ray_volumes(origin, dir, max_dist);
        merge_ray(rapier_hit, voxel_hit)
    }

    fn upsert_body(
        &mut self,
        entity: u64,
        position: [f32; 3],
        rotation_xyzw: [f32; 4],
        collider: &Collider,
    ) {
        // Voxel-volume-only colliders: registered for the DDA sweeps in
        // `cast_ray`/`cast_capsule` AND backed by a real static Rapier body
        // (parry `Voxels`, see `voxel_contact_bodies`) so dynamic bodies get
        // contact physics against terrain. The contact body is rebuilt on every
        // OnSet (chunk bitset rebuild on block change).
        if self.is_voxel_only(collider.parts) {
            self.world.destroy_body(entity);
            let parts = self.parts.get(collider.parts).cloned().unwrap_or_default();
            let handles: Vec<ShapeHandle> = parts.iter().map(|p| p.shape).collect();
            self.voxel_entities.insert(entity, handles);
            self.replace_voxel_contact_body(entity, collider);
            return;
        }
        // Mixed Rapier + voxel parts aren't supported; drop the voxel handles
        // and any contact body left over from a previous voxel-only upsert.
        self.voxel_entities.remove(&entity);
        self.destroy_voxel_contact_body(entity);
        let Some(shape) = self.build_shared_shape(collider.parts) else {
            log::warn!(
                "[physics] upsert_body entity={entity:#x}: empty/invalid parts={}",
                collider.parts
            );
            self.world.destroy_body(entity);
            return;
        };
        self.insert_body(entity, position, rotation_xyzw, collider, shape);
    }

    fn remove_body(&mut self, entity: u64) {
        self.destroy_voxel_contact_body(entity);
        if self.characters.contains_key(&entity) {
            self.remove_character(entity);
        } else if self.voxel_entities.remove(&entity).is_some() {
            // voxel-only body: DDA registration removed; the static contact
            // body was destroyed above.
        } else {
            self.world.destroy_body(entity);
        }
    }

    fn set_body_transform(&mut self, entity: u64, position: [f32; 3], rotation_xyzw: [f32; 4]) {
        // Voxel volumes are static (chunk-local); ignore ECS transform writes.
        if self.voxel_entities.contains_key(&entity) {
            return;
        }
        let Some(&handle) = self.world.body_map.get(&entity) else {
            return;
        };
        let Some(rb) = self.world.inner.bodies.get_mut(handle) else {
            return;
        };
        let rot = Quat::from_xyzw(
            rotation_xyzw[0],
            rotation_xyzw[1],
            rotation_xyzw[2],
            rotation_xyzw[3],
        );
        rb.set_position(Pose::from_parts(Vec3::from_array(position), rot), true);
    }

    fn set_body_linvel(&mut self, entity: u64, linvel: [f32; 3]) {
        if self.voxel_entities.contains_key(&entity) || self.characters.contains_key(&entity) {
            return;
        }
        let Some(&handle) = self.world.body_map.get(&entity) else {
            return;
        };
        let Some(rb) = self.world.inner.bodies.get_mut(handle) else {
            return;
        };
        if !rb.is_dynamic() {
            return;
        }
        rb.set_linvel(Vec3::from_array(linvel), true);
    }

    fn prepare(&mut self, world: &World, dt: f32) {
        // Create missing bridge bodies first so sync/step see them
        // (`PhysicsRigidBodyPrepare`, query "RigidBody, TransformComponent").
        if self.sys_rigid_body_prepare != 0 {
            world.run_system(self.sys_rigid_body_prepare, 0.0);
        }
        if self.sys_sync_in != 0 {
            world.run_system(self.sys_sync_in, dt.max(0.0));
        }
    }

    fn step(&mut self, dt: f32) {
        self.world.step(dt);
    }

    fn writeback(&mut self, world: &World) {
        if self.sys_writeback_characters != 0 {
            world.run_system(self.sys_writeback_characters, 0.0);
        }
        if self.sys_writeback_dynamics != 0 {
            world.run_system(self.sys_writeback_dynamics, 0.0);
        }
        if self.sys_rigid_body_writeback != 0 {
            world.run_system(self.sys_rigid_body_writeback, 0.0);
        }
    }

    /// Fixed-timestep loop decoupled from the server tick rate: physics runs
    /// in 1/60 s steps (the server ticks at 20 TPS → 3 substeps per tick), so
    /// solver quality and CCD behave like a real-time simulation instead of
    /// integrating 50 ms chunks. Writeback runs once per server frame with the
    /// final state. The accumulator is clamped so a hitch cannot trigger a
    /// spiral of death (leftover time beyond MAX_SUBSTEPS is dropped).
    fn advance(&mut self, world: &World, dt: f32) {
        const FIXED_DT: f32 = 1.0 / 60.0;
        const MAX_SUBSTEPS: u32 = 5;

        self.step_accumulator += dt;
        let max_acc = FIXED_DT * MAX_SUBSTEPS as f32;
        if self.step_accumulator > max_acc {
            self.step_accumulator = max_acc;
        }
        while self.step_accumulator >= FIXED_DT {
            self.prepare(world, FIXED_DT);
            self.step(FIXED_DT);
            self.step_accumulator -= FIXED_DT;
        }
        self.writeback(world);
    }

    fn attach_ecs(&mut self, world: &World) {
        if self.attached {
            return;
        }
        self.world_view = world as *const World;
        // Legacy ABI collider: flecs component renamed "Collider" → "PhysCollider"
        // (Rust type `abi::Collider` is unchanged). Registered by raw descriptor
        // so the flecs name does not follow the Rust type name.
        self.collider_id = world.component_init_raw(&ComponentDescriptor {
            name: "PhysCollider".to_string(),
            size: std::mem::size_of::<Collider>(),
            align: std::mem::align_of::<Collider>(),
            prefab_apply_user: std::ptr::null_mut(),
            prefab_apply: None,
            inspect_schema_cbor: Vec::new(),
            inspect_dump_user: std::ptr::null_mut(),
            inspect_dump_native: None,
            inspect_dump_ffi: None,
            layout_meta: Vec::new(),
        });
        self.char_id = world.component_init::<CharacterController>();
        // POD bridge components (the prefab-facing contract types).
        self.rigid_body_id = world.component_init::<RigidBody>();
        self.pod_collider_id = world.component_init::<PodCollider>();
        self.velocity_id = world.component_init::<Velocity>();
        self.phys_handle_id = world.component_init::<PhysBodyHandle>();
        // Transform: layout-compatible raw registration (no engine-common dep).
        self.transform_id = world.component_init_raw(&ComponentDescriptor {
            name: "TransformComponent".to_string(),
            size: std::mem::size_of::<TransformView>(),
            align: std::mem::align_of::<TransformView>(),
            prefab_apply_user: std::ptr::null_mut(),
            prefab_apply: None,
            inspect_schema_cbor: Vec::new(),
            inspect_dump_user: std::ptr::null_mut(),
            inspect_dump_native: None,
            inspect_dump_ffi: None,
            layout_meta: Vec::new(),
        });

        let self_ptr = self as *mut RapierPhysicsBackend as *mut c_void;
        let on_collider_set = observer_collider_set as deerpunk_engine_world::EcsCallback;
        let on_collider_rm = observer_collider_remove as deerpunk_engine_world::EcsCallback;
        let on_char_set = observer_character_set as deerpunk_engine_world::EcsCallback;
        let on_char_rm = observer_character_remove as deerpunk_engine_world::EcsCallback;
        let on_rigid_body_rm = observer_rigid_body_remove as deerpunk_engine_world::EcsCallback;
        // OnSet/OnRemove on the legacy ABI collider (flecs name "PhysCollider").
        if world
            .register_observer(
                "PhysCollider",
                &[deerpunk_engine_world::WatchEvent::OnSet],
                self_ptr,
                on_collider_set,
            )
            .is_err()
        {
            log::error!("[physics] OnSet PhysCollider observer failed");
        }
        if world
            .register_observer(
                "PhysCollider",
                &[deerpunk_engine_world::WatchEvent::OnRemove],
                self_ptr,
                on_collider_rm,
            )
            .is_err()
        {
            log::error!("[physics] OnRemove PhysCollider observer failed");
        }
        // OnAdd only — writeback must not recreate the kinematic capsule every tick.
        if world
            .register_observer(
                "CharacterController",
                &[deerpunk_engine_world::WatchEvent::OnAdd],
                self_ptr,
                on_char_set,
            )
            .is_err()
        {
            log::error!("[physics] OnAdd CharacterController observer failed");
        }
        if world
            .register_observer(
                "CharacterController",
                &[deerpunk_engine_world::WatchEvent::OnRemove],
                self_ptr,
                on_char_rm,
            )
            .is_err()
        {
            log::error!("[physics] OnRemove CharacterController observer failed");
        }
        // Removal observers are safe: OnRemove on "RigidBody" destroys the
        // body and clears `PhysBodyHandle`. No OnSet observer — prefab appliers
        // set components in arbitrary order, so body creation happens in the
        // PhysicsRigidBodyPrepare pass (see ecs_sync.rs / prepare).
        if world
            .register_observer(
                "RigidBody",
                &[deerpunk_engine_world::WatchEvent::OnRemove],
                self_ptr,
                on_rigid_body_rm,
            )
            .is_err()
        {
            log::error!("[physics] OnRemove RigidBody observer failed");
        }

        self.attached = true;
        super::ecs_sync::register_sync_systems(self, world);
        log::info!(
            "[physics] PhysCollider/CharacterController observers, OnRemove RigidBody cleanup, PhysicsRigidBodyPrepare/PhysicsSyncIn/Writeback systems; prepare→step→writeback"
        );
    }

    fn set_capsule_skin(&mut self, skin: f32) {
        self.capsule_skin = skin;
    }
}

extern "C-unwind" fn observer_collider_set(
    entities: *const deerpunk_engine_world::EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        let Some(c) = world.get::<Collider>(e, backend.collider_id) else {
            continue;
        };
        let t_ptr = world.get_raw(e, backend.transform_id);
        if t_ptr.is_null() {
            log::warn!("[physics] PhysCollider OnSet entity={e:#x}: no Transform");
            continue;
        }
        let t = unsafe { &*(t_ptr as *const TransformView) };
        let rot = t.rotation.to_array();
        backend.upsert_body(
            e,
            t.position.to_array(),
            [rot[0], rot[1], rot[2], rot[3]],
            &c,
        );
    }
}

extern "C-unwind" fn observer_collider_remove(
    entities: *const deerpunk_engine_world::EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        backend.remove_body(e);
    }
}

extern "C-unwind" fn observer_character_set(
    entities: *const deerpunk_engine_world::EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        let Some(c) = world.get::<CharacterController>(e, backend.char_id) else {
            continue;
        };
        let t_ptr = world.get_raw(e, backend.transform_id);
        if t_ptr.is_null() {
            log::warn!("[physics] CharacterController OnAdd entity={e:#x}: no Transform");
            continue;
        }
        let t = unsafe { &*(t_ptr as *const TransformView) };
        backend.upsert_character(e, t.position.to_array(), &c);
        log::info!(
            "[physics] CharacterController OnAdd entity={e:#x} feet={:?}",
            t.position
        );
    }
}

extern "C-unwind" fn observer_character_remove(
    entities: *const deerpunk_engine_world::EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        backend.remove_character(e);
    }
}

/// Rapier geometry straight from the POD [`PodCollider`] shapes array (no
/// shape/parts arena round-trip: the descriptor stores the params inline).
fn pod_shared_shape(c: &PodCollider) -> Option<SharedShape> {
    if c.shape_count == 0 {
        return None;
    }
    let mut parts: Vec<(Pose, SharedShape)> = Vec::new();
    for desc in c.shapes.iter().take(c.shape_count as usize) {
        let shape = match desc.kind {
            SHAPE_DESC_KIND_BOX => Some(SharedShape::cuboid(
                desc.params[0],
                desc.params[1],
                desc.params[2],
            )),
            SHAPE_DESC_KIND_BALL => Some(SharedShape::ball(desc.params[0])),
            SHAPE_DESC_KIND_CAPSULE => Some(SharedShape::capsule_z(
                desc.params[0],
                desc.params[1],
            )),
            other => {
                log::warn!("[physics] unknown POD collider shape kind {other}");
                None
            }
        };
        if let Some(shape) = shape {
            parts.push((
                Pose::from_parts(Vec3::from_array(desc.local_pos), Quat::IDENTITY),
                shape,
            ));
        }
    }
    match parts.len() {
        0 => None,
        1 => {
            let (pose, shape) = parts.pop().unwrap();
            if pose.translation == Vec3::ZERO && pose.rotation == Quat::IDENTITY {
                Some(shape)
            } else {
                Some(SharedShape::compound(vec![(pose, shape)]))
            }
        }
        _ => Some(SharedShape::compound(parts)),
    }
}

extern "C-unwind" fn observer_rigid_body_remove(
    entities: *const deerpunk_engine_world::EntityId,
    count: u32,
    _dt: f32,
    ctx: *mut c_void,
) {
    if entities.is_null() || ctx.is_null() || count == 0 {
        return;
    }
    let backend = unsafe { &mut *(ctx as *mut RapierPhysicsBackend) };
    if backend.world_view.is_null() {
        return;
    }
    let world = unsafe { &*backend.world_view };
    for i in 0..count as usize {
        let e = unsafe { *entities.add(i) };
        backend.remove_body(e);
        backend.applied_linvel.remove(&e);
        // PhysBodyHandle is removed together with the body.
        world.remove(e, backend.phys_handle_id);
    }
}
