//! Rapier implementation of [`super::backend::PhysicsBackend`].

use std::collections::HashMap;
use std::ffi::{c_void, CString};

use flecs_ecs_sys as sys;
use glam::Quat;
use rapier3d::control::{CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::{ShapeCastOptions, ShapeCastStatus};
use rapier3d::prelude::*;

use super::abi::{
    decode_shape, Collider, ColliderPart, PartsHandle, ShapeGeom, ShapeHandle, BODY_TYPE_DYNAMIC,
    BODY_TYPE_KINEMATIC, PARTS_HANDLE_INVALID, SHAPE_HANDLE_INVALID,
};
use super::backend::{CapsuleCastHit, PhysicsBackend};
use super::character::CharacterController;
use super::world::{BodyType, PhysicsWorld};
use crate::components::TransformComponent;
use crate::world::World;

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
    /// Flecs world + component ids for observers (ecs_get_id fallback).
    flecs: *mut sys::ecs_world_t,
    transform_id: u64,
    collider_id: u64,
    char_id: u64,
    _expr_collider_set: Option<CString>,
    _expr_collider_rm: Option<CString>,
    _expr_char_set: Option<CString>,
    _expr_char_rm: Option<CString>,
    attached: bool,
}

// Flecs world pointer is only touched on the server sim thread.
unsafe impl Send for RapierPhysicsBackend {}

impl RapierPhysicsBackend {
    pub fn new() -> Self {
        Self {
            world: PhysicsWorld::new(),
            shapes: Arena::new(),
            parts: Arena::new(),
            characters: HashMap::new(),
            flecs: std::ptr::null_mut(),
            transform_id: 0,
            collider_id: 0,
            char_id: 0,
            _expr_collider_set: None,
            _expr_collider_rm: None,
            _expr_char_set: None,
            _expr_char_rm: None,
            attached: false,
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

        let rb = match Self::body_type(collider.body_type) {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };

        let rot = Quat::from_xyzw(
            rotation_xyzw[0],
            rotation_xyzw[1],
            rotation_xyzw[2],
            rotation_xyzw[3],
        );
        let pose = Pose::from_parts(Vec3::from_array(position), rot);

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

        let (handle, _) = self.world.inner.insert(rb.position(pose), cb);
        self.world.body_map.insert(entity, handle);
        log::info!(
            "[physics] collider entity={entity:#x} pos={:?} bodies={}",
            position,
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
    fn sync_query_bvh(&mut self) {
        let handles: Vec<ColliderHandle> =
            self.world.inner.colliders.iter().map(|(h, _)| h).collect();
        if handles.is_empty() {
            return;
        }
        let mut events = Vec::new();
        self.world.inner.broad_phase.update(
            &self.world.inner.integration_parameters,
            &self.world.inner.colliders,
            &self.world.inner.bodies,
            &handles,
            &[],
            &mut events,
        );
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
        self.parts.clear();
        self.world.body_map.clear();
        self.world.tile_body_map.clear();
        self.world.vehicle_map.clear();
        self.attached = false;
    }

    fn create_shape(&mut self, abi: &[u8]) -> ShapeHandle {
        match decode_shape(abi) {
            Some(geom) => self.shapes.insert(geom),
            None => {
                log::warn!("[physics] create_shape: bad ABI ({} bytes)", abi.len());
                SHAPE_HANDLE_INVALID
            }
        }
    }

    fn destroy_shape(&mut self, handle: ShapeHandle) {
        let _ = self.shapes.remove(handle);
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

        self.sync_query_bvh();

        let shape = SharedShape::capsule_z(half_height, radius);
        let pose = Pose::from_parts(Vec3::from_array(center), Quat::IDENTITY);
        let exclude = if exclude_entity != 0 {
            self.world.body_map.get(&exclude_entity).copied()
        } else {
            None
        };
        let filter = if let Some(rb) = exclude {
            QueryFilter::default().exclude_rigid_body(rb)
        } else {
            QueryFilter::default()
        };
        let queries = self.world.inner.query_pipeline_with_filter(filter);
        // Keep a contact skin so slide casts don't leave the capsule embedded
        // (HL DIST_EPSILON / Rapier KCC offset). stop_at_penetration=false lets
        // shallow overlaps resolve instead of toi=0 forever.
        const SKIN: f32 = 0.5;
        let opts = ShapeCastOptions {
            max_time_of_impact: max_dist,
            target_distance: SKIN,
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: true,
        };
        // QueryPipeline: normal1/witness1 = hit collider, world space.
        let (_handle, hit) = queries.cast_shape(&pose, d, shape.as_ref(), opts)?;
        let toi = hit.time_of_impact;
        let start_solid = matches!(
            hit.status,
            ShapeCastStatus::PenetratingOrWithinTargetDist
        );
        let n = hit.normal1;
        let point = hit.witness1;
        Some(CapsuleCastHit {
            toi,
            normal: [n.x, n.y, n.z],
            point: [point.x, point.y, point.z],
            start_solid,
        })
    }

    fn upsert_body(
        &mut self,
        entity: u64,
        position: [f32; 3],
        rotation_xyzw: [f32; 4],
        collider: &Collider,
    ) {
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
        if self.characters.contains_key(&entity) {
            self.remove_character(entity);
        } else {
            self.world.destroy_body(entity);
        }
    }

    fn set_body_transform(&mut self, entity: u64, position: [f32; 3], rotation_xyzw: [f32; 4]) {
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

    fn prepare(&mut self, world: &World, dt: f32) {
        // Refresh BVH so OnSet colliders are visible to queries this tick.
        self.sync_query_bvh();

        let cc_id = world.component_init::<CharacterController>();
        let t_id = world.component_init::<TransformComponent>();
        let entities: Vec<u64> = self.characters.keys().copied().collect();
        let dt = dt.max(0.0);

        for entity in entities {
            let Some(cc) = world.get::<CharacterController>(entity, cc_id) else {
                continue;
            };
            let Some(t) = world.get::<TransformComponent>(entity, t_id) else {
                continue;
            };
            let Some(entry) = self.characters.get(&entity) else {
                continue;
            };
            let body = entry.body;
            let shape = entry.shape.clone();
            let center_offset = entry.center_offset.max(cc.center_offset());
            let controller = Self::kcc_for(entry);
            let noclip = cc.noclip();
            let manual = cc.manual_move();

            let Some(rb) = self.world.inner.bodies.get_mut(body) else {
                continue;
            };
            // Body / ECS = feet. Teleport if ECS moved far (spawn / noclip / manual).
            let mut feet = rb.translation();
            if (feet - t.position).length_squared() > 1.0 || manual {
                feet = t.position;
            }
            rb.set_position(Pose::from_parts(feet, Quat::IDENTITY), true);

            // Gamelib owns slide integration — keep collider in sync, skip KCC.
            if manual {
                if let Some(entry) = self.characters.get_mut(&entity) {
                    entry.grounded = cc.grounded != 0;
                    entry.center_offset = center_offset;
                    entry.step_height = cc.step_height.max(0.0);
                    entry.max_slope_deg = cc.max_slope_deg.max(1.0);
                }
                continue;
            }

            let desired = Vec3::from_array(cc.velocity) * dt;
            let (new_feet, grounded) = if noclip {
                (feet + desired, false)
            } else {
                // Rapier docs: move_shape(dt, queries, shape, character_pos, desired)
                // character_pos = capsule *center* (not feet).
                let mut center = feet + Vec3::new(0.0, 0.0, center_offset);
                let filter = QueryFilter::default().exclude_rigid_body(body);
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
    }

    fn step(&mut self, dt: f32) {
        self.world.step(dt);
    }

    fn writeback(&mut self, world: &World) {
        let cc_id = world.component_init::<CharacterController>();
        let t_id = world.component_init::<TransformComponent>();

        for (&entity, entry) in &self.characters {
            let Some(rb) = self.world.inner.bodies.get(entry.body) else {
                continue;
            };
            let ptr = world.get_raw(entity, cc_id);
            let manual = if !ptr.is_null() {
                unsafe { (*(ptr as *const CharacterController)).manual_move() }
            } else {
                false
            };
            // Manual: gamelib already wrote Transform + grounded; don't stomp.
            if manual {
                continue;
            }
            // A: body = feet = ECS Transform.
            let feet = rb.translation();
            if let Some(mut t) = world.get::<TransformComponent>(entity, t_id) {
                t.position = feet;
                world.set(entity, t_id, t);
            }
            // Patch in place — no world.set (keeps OnAdd-only create path clean).
            if !ptr.is_null() {
                let cc = unsafe { &mut *(ptr as *mut CharacterController) };
                cc.grounded = u32::from(entry.grounded);
                // Only kill downward stick/fall — never clamp away jump (+Z).
                if entry.grounded && !cc.noclip() && cc.velocity[2] < 0.0 {
                    cc.velocity[2] = cc.velocity[2].clamp(-80.0, 0.0);
                }
            }
        }

        // Rigid bodies (non-character) → Transform.
        for (&entity, &handle) in &self.world.body_map {
            if self.characters.contains_key(&entity) {
                continue;
            }
            let Some(rb) = self.world.inner.bodies.get(handle) else {
                continue;
            };
            if !rb.is_dynamic() {
                continue;
            }
            if let Some(mut t) = world.get::<TransformComponent>(entity, t_id) {
                t.position = rb.translation();
                t.rotation = *rb.rotation();
                world.set(entity, t_id, t);
            }
        }
    }

    fn attach_ecs(&mut self, world: &World) {
        if self.attached {
            return;
        }
        self.flecs = world.as_ptr();
        self.collider_id = world.component_init::<Collider>();
        self.char_id = world.component_init::<CharacterController>();
        self.transform_id = world.component_init::<TransformComponent>();

        // Single-component queries — Transform fetched via ecs_get_id (field 1 was often null).
        let expr_set = CString::new("Collider").unwrap();
        let expr_remove = CString::new("Collider").unwrap();
        let expr_char_set = CString::new("CharacterController").unwrap();
        let expr_char_rm = CString::new("CharacterController").unwrap();

        let self_ptr = self as *mut RapierPhysicsBackend as *mut c_void;

        unsafe {
            let mut desc: sys::ecs_observer_desc_t = std::mem::zeroed();
            desc.query.expr = expr_set.as_ptr();
            desc.events[0] = sys::EcsOnSet;
            desc.callback = Some(observer_collider_set);
            desc.ctx = self_ptr;
            if sys::ecs_observer_init(world.as_ptr(), &desc) == 0 {
                log::error!("[physics] OnSet Collider observer failed");
            }

            let mut desc_rm: sys::ecs_observer_desc_t = std::mem::zeroed();
            desc_rm.query.expr = expr_remove.as_ptr();
            desc_rm.events[0] = sys::EcsOnRemove;
            desc_rm.callback = Some(observer_collider_remove);
            desc_rm.ctx = self_ptr;
            if sys::ecs_observer_init(world.as_ptr(), &desc_rm) == 0 {
                log::error!("[physics] OnRemove Collider observer failed");
            }

            // OnAdd only — writeback must not recreate the kinematic capsule every tick.
            let mut desc_c: sys::ecs_observer_desc_t = std::mem::zeroed();
            desc_c.query.expr = expr_char_set.as_ptr();
            desc_c.events[0] = sys::EcsOnAdd;
            desc_c.callback = Some(observer_character_set);
            desc_c.ctx = self_ptr;
            if sys::ecs_observer_init(world.as_ptr(), &desc_c) == 0 {
                log::error!("[physics] OnAdd CharacterController observer failed");
            }

            let mut desc_crm: sys::ecs_observer_desc_t = std::mem::zeroed();
            desc_crm.query.expr = expr_char_rm.as_ptr();
            desc_crm.events[0] = sys::EcsOnRemove;
            desc_crm.callback = Some(observer_character_remove);
            desc_crm.ctx = self_ptr;
            if sys::ecs_observer_init(world.as_ptr(), &desc_crm) == 0 {
                log::error!("[physics] OnRemove CharacterController observer failed");
            }
        }

        self._expr_collider_set = Some(expr_set);
        self._expr_collider_rm = Some(expr_remove);
        self._expr_char_set = Some(expr_char_set);
        self._expr_char_rm = Some(expr_char_rm);
        self.attached = true;
        log::info!(
            "[physics] observers: Collider(OnSet) + CharacterController(OnAdd); step→prepare→writeback"
        );
    }
}

unsafe extern "C-unwind" fn observer_collider_set(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let backend = &mut *(it_ref.ctx as *mut RapierPhysicsBackend);
        let colliders =
            sys::ecs_field_w_size(it, std::mem::size_of::<Collider>(), 0) as *const Collider;
        if colliders.is_null() || backend.flecs.is_null() {
            return;
        }
        for i in 0..it_ref.count as isize {
            let e = *it_ref.entities.add(i as usize);
            let c = &*colliders.add(i as usize);
            let t_ptr = sys::ecs_get_id(backend.flecs, e, backend.transform_id)
                as *const TransformComponent;
            if t_ptr.is_null() {
                log::warn!("[physics] Collider OnSet entity={e:#x}: no Transform");
                continue;
            }
            let t = &*t_ptr;
            let rot = t.rotation.to_array();
            backend.upsert_body(
                e,
                t.position.to_array(),
                [rot[0], rot[1], rot[2], rot[3]],
                c,
            );
        }
    }
}

unsafe extern "C-unwind" fn observer_collider_remove(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let backend = &mut *(it_ref.ctx as *mut RapierPhysicsBackend);
        for i in 0..it_ref.count as isize {
            let e = *it_ref.entities.add(i as usize);
            backend.remove_body(e);
        }
    }
}

unsafe extern "C-unwind" fn observer_character_set(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let backend = &mut *(it_ref.ctx as *mut RapierPhysicsBackend);
        let chars = sys::ecs_field_w_size(it, std::mem::size_of::<CharacterController>(), 0)
            as *const CharacterController;
        if chars.is_null() || backend.flecs.is_null() {
            return;
        }
        for i in 0..it_ref.count as isize {
            let e = *it_ref.entities.add(i as usize);
            let c = &*chars.add(i as usize);
            let t_ptr = sys::ecs_get_id(backend.flecs, e, backend.transform_id)
                as *const TransformComponent;
            if t_ptr.is_null() {
                log::warn!("[physics] CharacterController OnAdd entity={e:#x}: no Transform");
                continue;
            }
            let t = &*t_ptr;
            backend.upsert_character(e, t.position.to_array(), c);
            log::info!(
                "[physics] CharacterController OnAdd entity={e:#x} feet={:?}",
                t.position
            );
        }
    }
}

unsafe extern "C-unwind" fn observer_character_remove(it: *mut sys::ecs_iter_t) {
    unsafe {
        if it.is_null() {
            return;
        }
        let it_ref = &*it;
        if it_ref.ctx.is_null() || it_ref.count <= 0 {
            return;
        }
        let backend = &mut *(it_ref.ctx as *mut RapierPhysicsBackend);
        for i in 0..it_ref.count as isize {
            let e = *it_ref.entities.add(i as usize);
            backend.remove_character(e);
        }
    }
}
