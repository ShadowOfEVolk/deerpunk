use std::collections::HashMap;

use rapier3d::prelude::*;
use rapier3d::control::{DynamicRayCastVehicleController, WheelTuning};
use glam::Quat;

/// Обёртка над rapier3d::PhysicsWorld + vehicle controllers.
pub struct PhysicsWorld {
    pub inner: rapier3d::pipeline::PhysicsWorld,
    pub body_map: HashMap<u64, RigidBodyHandle>,
    pub tile_body_map: HashMap<(i32, i32), RigidBodyHandle>,
    /// ECS Entity (u64) → vehicle controller (для танков).
    pub vehicle_map: HashMap<u64, DynamicRayCastVehicleController>,
}

pub mod categories {
    pub const TERRAIN: u32 = 1 << 0;
    pub const TANK: u32 = 1 << 1;
    pub const PROJECTILE: u32 = 1 << 2;
    pub const SENSOR: u32 = 1 << 3;
}

impl PhysicsWorld {
    pub fn new() -> Self {
        let mut inner = rapier3d::pipeline::PhysicsWorld::new();
        inner.gravity = Vec3::new(0.0, 0.0, -9.81);

        // Оптимизация: меньше итераций solver'а.
        inner.integration_parameters.num_solver_iterations = 2;
        inner.integration_parameters.num_internal_pgs_iterations = 1;
        inner.integration_parameters.max_ccd_substeps = 0;

        Self {
            inner,
            body_map: HashMap::new(),
            tile_body_map: HashMap::new(),
            vehicle_map: HashMap::new(),
        }
    }

    pub fn create_body(
        &mut self,
        entity_bits: u64,
        x: f32, y: f32, z: f32,
        yaw: f32,
        collider: &ColliderDescriptor,
    ) -> RigidBodyHandle {
        let rb = match collider.body_type {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };

        let (handle, _) = self.inner.insert(
            rb.translation(Vec3::new(x, y, z))
                .rotation(Vec3::new(0.0, 0.0, yaw)),
            self.make_collider(collider),
        );

        self.body_map.insert(entity_bits, handle);
        handle
    }

    pub fn create_tile_body(
        &mut self, tx: i32, ty: i32, x: f32, y: f32,
        collider: &ColliderDescriptor,
    ) -> RigidBodyHandle {
        let handle = self.create_body(0, x, y, 0.0, 0.0, collider);
        self.tile_body_map.insert((tx, ty), handle);
        handle
    }

    pub fn destroy_body(&mut self, entity_bits: u64) {
        if let Some(handle) = self.body_map.remove(&entity_bits) {
            self.inner.remove_body(handle);
        }
    }

    pub fn destroy_tile_body(&mut self, tx: i32, ty: i32) {
        if let Some(handle) = self.tile_body_map.remove(&(tx, ty)) {
            self.inner.remove_body(handle);
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.inner.integration_parameters.dt = dt;
        self.inner.step();
    }

    pub fn last_profile(&self) -> RapierProfile {
        let c = &self.inner.physics_pipeline.counters;
        RapierProfile {
            broad_phase_ms: c.broad_phase_time_ms() as f32,
            narrow_phase_ms: c.narrow_phase_time_ms() as f32,
            solver_ms: c.solver_time_ms() as f32,
            total_ms: c.step_time_ms() as f32,
            ncontacts: c.cd.ncontact_pairs,
            nconstraints: c.solver.nconstraints,
        }
    }

    pub fn ray_cast_closest(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_dist: f32,
        category_mask: u32,
    ) -> Option<RayHit> {
        let ray = Ray::new(
            Vec3::new(origin[0], origin[1], origin[2]),
            Vec3::new(direction[0], direction[1], direction[2]).normalize(),
        );

        let filter = QueryFilter {
            groups: Some(InteractionGroups {
                memberships: Group::from(category_mask),
                filter: Group::ALL,
                test_mode: InteractionTestMode::default(),
            }),
            ..QueryFilter::default()
        };

        self.inner.cast_ray_and_get_normal(&ray, max_dist, true, filter)
            .map(|(_handle, hit)| {
                let point = ray.point_at(hit.time_of_impact);
                let n = hit.normal;
                RayHit {
                    point: [point.x, point.y, point.z],
                    normal: [n.x, n.y, n.z],
                    fraction: hit.time_of_impact,
                }
            })
    }

    pub fn body_position(&self, handle: RigidBodyHandle) -> Option<[f32; 3]> {
        self.inner.bodies.get(handle).map(|rb| {
            let p = rb.translation();
            [p.x, p.y, p.z]
        })
    }

    pub fn body_yaw(&self, handle: RigidBodyHandle) -> f32 {
        self.inner.bodies.get(handle)
            .map(|rb| {
                let q = *rb.rotation();
                2.0 * q.y.atan2(q.w)
            })
            .unwrap_or(0.0)
    }

    /// Читает euler angles из body.
    /// Возвращает (x, y, z) для Quat::from_euler(EulerRot::XYZ, x, y, z).
    pub fn body_rotation(&self, handle: RigidBodyHandle) -> Option<(f32, f32, f32)> {
        self.inner.bodies.get(handle).map(|rb| {
            let q = *rb.rotation();
            q.to_euler(glam::EulerRot::XYZ)
        })
    }

    pub fn set_body_transform(&mut self, handle: RigidBodyHandle, x: f32, y: f32, z: f32, yaw: f32) {
        if let Some(rb) = self.inner.bodies.get_mut(handle) {
            rb.set_position(
                Pose3::from_parts(
                    Vec3::new(x, y, z).into(),
                    Quat::from_rotation_z(yaw),
                ),
                true,
            );
        }
    }

    pub fn set_body_velocity(&mut self, handle: RigidBodyHandle, vx: f32, vy: f32, angular: f32) {
        if let Some(rb) = self.inner.bodies.get_mut(handle) {
            rb.set_linvel(Vec3::new(vx, vy, 0.0), true);
            rb.set_angvel(Vec3::new(0.0, 0.0, angular), true);
        }
    }

    pub fn apply_impulse(&mut self, handle: RigidBodyHandle, vx: f32, vy: f32) {
        if let Some(rb) = self.inner.bodies.get_mut(handle) {
            rb.apply_impulse(Vec3::new(vx, vy, 0.0), true);
        }
    }

    pub fn apply_impulse_3d(&mut self, handle: RigidBodyHandle, vx: f32, vy: f32, vz: f32) {
        if let Some(rb) = self.inner.bodies.get_mut(handle) {
            rb.apply_impulse(Vec3::new(vx, vy, vz), true);
        }
    }

    // ── Vehicle ──────────────────────────────────────────────────────

    /// Создаёт vehicle controller для танка (chassis body).
    /// Добавляет 4 колеса: 2 задних (привод), 2 передних (поворот).
    pub fn create_vehicle(&mut self, entity_bits: u64) {
        let chassis = *self.body_map.get(&entity_bits).expect("body not found");
        let mut vehicle = DynamicRayCastVehicleController::new(chassis);
        vehicle.index_up_axis = 2;      // Z-up
        vehicle.index_forward_axis = 1; // Y-forward

        let tuning = WheelTuning {
            suspension_stiffness: 20.0,
            suspension_compression: 4.4,
            suspension_damping: 2.3,
            max_suspension_travel: 0.3,
            friction_slip: 3.5,
            side_friction_stiffness: 1.0,
            max_suspension_force: 10000.0,
        };

        let half_l = 1.2; // половина длины танка
        let half_w = 0.8; // половина ширины танка
        let wheel_radius = 0.15;

        // Задние колёса (привод)
        vehicle.add_wheel(
            Vec3::new(-half_w, -half_l, -0.2),  // connection
            Vec3::new(0.0, 0.0, -1.0),          // suspension down
            Vec3::new(1.0, 0.0, 0.0),           // axle X
            0.15,                                // rest length
            wheel_radius,
            &tuning,
        );

        vehicle.add_wheel(
            Vec3::new(half_w, -half_l, -0.2),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 0.0),
            0.15,
            wheel_radius,
            &tuning,
        );

        // Передние колёса (поворот)
        vehicle.add_wheel(
            Vec3::new(-half_w, half_l, -0.2),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 0.0),
            0.15,
            wheel_radius,
            &tuning,
        );

        vehicle.add_wheel(
            Vec3::new(half_w, half_l, -0.2),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 0.0),
            0.15,
            wheel_radius,
            &tuning,
        );

        self.vehicle_map.insert(entity_bits, vehicle);
    }

    /// Удаляет vehicle controller.
    pub fn destroy_vehicle(&mut self, entity_bits: u64) {
        self.vehicle_map.remove(&entity_bits);
    }

    /// Устанавливает скорость привода (задние колёса).
    pub fn set_vehicle_drive(&mut self, entity_bits: u64, speed: f32) {
        if let Some(vehicle) = self.vehicle_map.get_mut(&entity_bits) {
            for i in 0..2.min(vehicle.wheels().len()) {
                vehicle.wheels_mut()[i].engine_force = speed;
            }
        }
    }

    /// Устанавливает тормоз.
    pub fn set_vehicle_brake(&mut self, entity_bits: u64, brake: f32) {
        if let Some(vehicle) = self.vehicle_map.get_mut(&entity_bits) {
            for wheel in vehicle.wheels_mut().iter_mut() {
                wheel.brake = brake;
            }
        }
    }

    /// Устанавливает угол поворота передних колёс.
    pub fn set_vehicle_steering(&mut self, entity_bits: u64, angle: f32) {
        if let Some(vehicle) = self.vehicle_map.get_mut(&entity_bits) {
            let n = vehicle.wheels().len();
            for i in 2..n {
                vehicle.wheels_mut()[i].steering = angle;
            }
        }
    }

    /// Обновляет все vehicle controllers (вызывать перед step).
    pub fn update_vehicles(&mut self, dt: f32) {
        let vehicle_keys: Vec<u64> = self.vehicle_map.keys().copied().collect();
        for entity_bits in vehicle_keys {
            if let Some(mut vehicle) = self.vehicle_map.remove(&entity_bits) {
                let qp = self.inner.broad_phase.as_query_pipeline_mut(
                    self.inner.narrow_phase.query_dispatcher(),
                    &mut self.inner.bodies,
                    &mut self.inner.colliders,
                    QueryFilter::default(),
                );
                vehicle.update_vehicle(dt, qp);
                self.vehicle_map.insert(entity_bits, vehicle);
            }
        }
    }

    fn make_collider(&self, desc: &ColliderDescriptor) -> ColliderBuilder {
        let shape = match &desc.shape {
            ColliderShape::Box(h) => SharedShape::cuboid(h[0], h[1], h[2]),
            ColliderShape::Ball(r) => SharedShape::ball(*r),
            ColliderShape::HalfSpace(normal) => {
                SharedShape::halfspace(
                    Vec3::new(normal[0], normal[1], normal[2]).normalize(),
                )
            }
            ColliderShape::HeightField { heights, rows, cols, scale } => {
                let array = rapier3d::geometry::Array2::new(*rows, *cols, heights.clone());
                SharedShape::heightfield(
                    array,
                    Vec3::new(scale[0], scale[1], scale[2]).into(),
                )
            }
        };

        let groups = InteractionGroups {
            memberships: Group::from(desc.category_bits),
            filter: Group::from(desc.mask_bits),
            test_mode: InteractionTestMode::default(),
        };

        let mut builder = ColliderBuilder::new(shape)
            .friction(desc.friction)
            .restitution(desc.restitution)
            .collision_groups(groups);

        if desc.is_sensor {
            builder = builder.sensor(true);
        }
        if desc.density > 0.0 {
            builder = builder.density(desc.density);
        }

        builder
    }
}

pub struct ColliderDescriptor {
    pub shape: ColliderShape,
    pub body_type: BodyType,
    pub category_bits: u32,
    pub mask_bits: u32,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    pub is_sensor: bool,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct RapierProfile {
    pub broad_phase_ms: f32,
    pub narrow_phase_ms: f32,
    pub solver_ms: f32,
    pub total_ms: f32,
    pub ncontacts: usize,
    pub nconstraints: usize,
}

pub enum ColliderShape {
    Box([f32; 3]),
    Ball(f32),
    HalfSpace([f32; 3]),
    HeightField {
        heights: Vec<f32>,
        rows: usize,
        cols: usize,
        scale: [f32; 3],
    },
}

pub enum BodyType {
    Static,
    Dynamic,
    Kinematic,
}

impl ColliderDescriptor {
    pub fn static_box(half_extents: [f32; 3]) -> Self {
        Self {
            shape: ColliderShape::Box(half_extents),
            body_type: BodyType::Static,
            category_bits: categories::TERRAIN,
            mask_bits: categories::TANK | categories::PROJECTILE,
            friction: 0.5, restitution: 0.0, density: 0.0, is_sensor: false,
        }
    }
    pub fn dynamic_box(half_extents: [f32; 3]) -> Self {
        Self {
            shape: ColliderShape::Box(half_extents),
            body_type: BodyType::Dynamic,
            category_bits: categories::TANK,
            mask_bits: categories::TERRAIN | categories::TANK | categories::SENSOR,
            friction: 0.3, restitution: 0.1, density: 1.0, is_sensor: false,
        }
    }
    pub fn sensor(half_extents: [f32; 3]) -> Self {
        Self {
            shape: ColliderShape::Box(half_extents),
            body_type: BodyType::Static,
            category_bits: categories::SENSOR,
            mask_bits: categories::TANK,
            friction: 0.0, restitution: 0.0, density: 0.0, is_sensor: true,
        }
    }
    pub fn static_ground(normal: [f32; 3]) -> Self {
        Self {
            shape: ColliderShape::HalfSpace(normal),
            body_type: BodyType::Static,
            category_bits: categories::TERRAIN,
            mask_bits: categories::TANK | categories::PROJECTILE,
            friction: 0.5, restitution: 0.0, density: 0.0, is_sensor: false,
        }
    }
    pub fn static_heightfield(
        heights: Vec<f32>, rows: usize, cols: usize, scale: [f32; 3],
    ) -> Self {
        Self {
            shape: ColliderShape::HeightField { heights, rows, cols, scale },
            body_type: BodyType::Static,
            category_bits: categories::TERRAIN,
            mask_bits: categories::TANK | categories::PROJECTILE,
            friction: 0.5, restitution: 0.0, density: 0.0, is_sensor: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sensor;

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub point: [f32; 3],
    pub normal: [f32; 3],
    pub fraction: f32,
}
