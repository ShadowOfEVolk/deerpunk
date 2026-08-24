use std::collections::{HashMap, HashSet};

use deerpunk_engine_common::flecs_world::{FlecsWorld, ComponentId, Entity};
use deerpunk_engine_common::physics::PhysicsWorld;

use crate::schedule::Schedule;

pub struct ServerEngine {
    pub ecs_world: FlecsWorld,
    pub physics_world: PhysicsWorld,
    pub(crate) schedule: Schedule,
    pub(crate) component_names: HashMap<String, ComponentId>,
    pub(crate) component_sizes: HashMap<ComponentId, usize>,
    pub(crate) entities: Vec<Entity>,
    pub(crate) next_component_id: u64,
    pub(crate) initialized: bool,
    terrain_bodies: HashSet<u64>,
}

impl ServerEngine {
    pub fn new() -> Self {
        Self {
            ecs_world: FlecsWorld::new(),
            physics_world: PhysicsWorld::new(),
            schedule: Schedule::new(),
            component_names: HashMap::new(),
            component_sizes: HashMap::new(),
            entities: Vec::new(),
            next_component_id: 0,
            initialized: false,
            terrain_bodies: HashSet::new(),
        }
    }

    pub fn initialize(&mut self, gamedir: impl AsRef<std::path::Path>) -> Result<(), String> {
        use deerpunk_engine_common::{cmd::cmd, cvar::cvar, fs::fs};
        if self.initialized {
            return Err("Engine already initialized".to_string());
        }
        fs().initialize();
        fs().set_gamedir(gamedir);
        cmd().initialize();
        cvar().initialize();
        cmd().exec_config_file("configs/autoexec.cfg");

        self.register_component("Transform", std::mem::size_of::<deerpunk_engine_common::transform::Transform>(), std::mem::align_of::<deerpunk_engine_common::transform::Transform>());
        self.register_component("CameraComponent", std::mem::size_of::<deerpunk_engine_common::components::camera::CameraComponent>(), std::mem::align_of::<deerpunk_engine_common::components::camera::CameraComponent>());
        self.register_component("MeshRenderer", std::mem::size_of::<deerpunk_engine_common::components::mesh_renderer::MeshRenderer>(), std::mem::align_of::<deerpunk_engine_common::components::mesh_renderer::MeshRenderer>());
        self.register_component("Terrain", std::mem::size_of::<deerpunk_engine_common::components::terrain::Terrain>(), std::mem::align_of::<deerpunk_engine_common::components::terrain::Terrain>());
        self.register_component("TerrainCollider", std::mem::size_of::<deerpunk_engine_common::components::terrain_collider::TerrainCollider>(), std::mem::align_of::<deerpunk_engine_common::components::terrain_collider::TerrainCollider>());
        self.register_component("RigidBody", std::mem::size_of::<deerpunk_engine_common::physics::RigidBody>(), std::mem::align_of::<deerpunk_engine_common::physics::RigidBody>());
        self.register_component("RigidBodyHandle", std::mem::size_of::<deerpunk_engine_common::physics::RigidBodyHandle>(), std::mem::align_of::<deerpunk_engine_common::physics::RigidBodyHandle>());
        let pbd_id = self.register_component("PhysicsBodyDesc", std::mem::size_of::<deerpunk_engine_common::physics::PhysicsBodyDesc>(), std::mem::align_of::<deerpunk_engine_common::physics::PhysicsBodyDesc>());

        // Create OnAdd observer: PhysicsBodyDesc → auto-register rapier body
        self.ecs_world.observe_physics_body(pbd_id);

        self.schedule.add_system(|dt: f32| {
            unsafe {
                let se = &mut *(crate::engine_ctx() as *mut ServerEngine);
                se.ensure_terrain_colliders();
            }
            let world = unsafe { crate::field::ecs_world() };
            let physics = unsafe { crate::field::physics_world() };
            deerpunk_engine_common::physics::physics_sync_in(physics, world);
            deerpunk_engine_common::physics::physics_step(physics, dt);
            deerpunk_engine_common::physics::physics_sync_out(physics, world);
        });

        self.initialized = true;
        Ok(())
    }

    pub fn frame(&mut self, delta_time: f32) -> Result<(), String> {
        use deerpunk_engine_common::cmd::cmd;
        if !self.initialized {
            return Err("Engine not initialized".to_string());
        }
        cmd().frame();
        self.schedule.run(delta_time);
        Ok(())
    }

    pub fn shutdown(&mut self) {
        use deerpunk_engine_common::{cmd::cmd, cvar::cvar, fs::fs};
        if self.initialized {
            cmd().shutdown();
            cvar().shutdown();
            fs().shutdown();
            self.initialized = false;
        }
    }

    // ── Component registration ──────────────────────────────────

    pub fn register_component(&mut self, name: &str, size: usize, alignment: usize) -> ComponentId {
        if let Some(&id) = self.component_names.get(name) {
            return id;
        }
        let id = self.ecs_world.register_component(name, size, alignment);
        self.component_names.insert(name.to_string(), id);
        self.component_sizes.insert(id, size);
        id
    }

    pub fn component_size(&self, comp_id: ComponentId) -> usize {
        self.component_sizes.get(&comp_id).copied().unwrap_or(0)
    }

    pub fn resolve_component(&self, name: &str) -> Option<ComponentId> {
        self.component_names.get(name).copied()
    }

    pub fn resolve_component_name(&self, id: ComponentId) -> Option<&str> {
        self.component_names.iter().find(|(_, &v)| v == id).map(|(k, _)| k.as_str())
    }

    // ── Entity/component access ─────────────────────────────────

    pub fn has(&self, entity_id: u64, comp_id: u64) -> bool {
        self.ecs_world.has(entity_id, comp_id)
    }

    pub fn get_raw(&self, entity_id: u64, comp_id: u64) -> Option<*const std::ffi::c_void> {
        self.ecs_world.get_raw(entity_id, comp_id)
    }

    pub fn set_raw(&self, entity_id: u64, comp_id: u64, ptr: *const std::ffi::c_void, size: usize) {
        self.ecs_world.set_raw(entity_id, comp_id, ptr, size);
    }

    pub fn remove(&self, entity_id: u64, comp_id: u64) {
        self.ecs_world.remove(entity_id, comp_id);
    }

    pub fn spawn(&mut self) -> u64 {
        let entity = self.ecs_world.spawn();
        self.entities.push(entity);
        entity
    }

    pub fn spawn_bundle(&mut self, components: Vec<(u64, *const std::ffi::c_void, usize)>) -> u64 {
        let entity = self.ecs_world.spawn();
        for (comp_id, ptr, size) in components {
            if size == 0 {
                continue;
            }
            self.ecs_world.set_raw(entity, comp_id, ptr, size);
        }
        self.entities.push(entity);
        entity
    }

    pub fn destroy(&mut self, entity_id: u64) {
        self.ecs_world.delete(entity_id);
        self.entities.retain(|&e| e != entity_id);
    }

    pub fn clear(&mut self) {
        self.ecs_world = FlecsWorld::new();
        self.entities.clear();
    }

    pub fn query(&self, require: &[u64], exclude: &[u64]) -> Vec<u64> {
        use std::collections::HashSet;

        let require_set: HashSet<u64> = require.iter().copied().collect();
        let exclude_set: HashSet<u64> = exclude.iter().copied().collect();

        if require_set.is_empty() && exclude_set.is_empty() {
            return self.entities.clone();
        }

        self.entities.iter()
            .filter(|&&e| {
                require_set.iter().all(|&c| self.has(e, c))
                    && !exclude_set.iter().any(|&c| self.has(e, c))
            })
            .copied()
            .collect()
    }

    pub fn query_count(&self, require: &[u64], exclude: &[u64]) -> usize {
        self.query(require, exclude).len()
    }

    pub fn query_matches(&self, entity_id: u64, require: &[u64], exclude: &[u64]) -> bool {
        require.iter().all(|&c| self.has(entity_id, c))
            && !exclude.iter().any(|&c| self.has(entity_id, c))
    }

    pub fn ecs_world(&self) -> &FlecsWorld {
        &self.ecs_world
    }

    pub fn ecs_world_mut(&mut self) -> &mut FlecsWorld {
        &mut self.ecs_world
    }

    /// Creates heightfield physics bodies for entities with Terrain + TerrainCollider.
    pub fn ensure_terrain_colliders(&mut self) {
        let terrain_id = match self.resolve_component("Terrain") {
            Some(id) => id,
            None => return,
        };
        let tc_id = match self.resolve_component("TerrainCollider") {
            Some(id) => id,
            None => return,
        };

        let entities = self.query(&[terrain_id, tc_id], &[]);

        for &entity_id in &entities {
            if self.terrain_bodies.contains(&entity_id) {
                continue;
            }
            if self.physics_world.body_map.contains_key(&entity_id) {
                continue;
            }

            let t_ptr = match self.get_raw(entity_id, terrain_id) {
                Some(p) => p,
                None => continue,
            };

            let tc_ptr = match self.get_raw(entity_id, tc_id) {
                Some(p) => p,
                None => continue,
            };

            let (heights, rows, cols, cell_size, height_scale) = unsafe {
                let t = &*(t_ptr as *const deerpunk_engine_common::Terrain);
                (t.heights.clone(), t.rows, t.cols, t.cell_size, t.height_scale)
            };

            if heights.is_empty() || rows == 0 || cols == 0 {
                continue;
            }

            let (friction, restitution) = unsafe {
                let tc = &*(tc_ptr as *const deerpunk_engine_common::TerrainCollider);
                (tc.friction, tc.restitution)
            };

            let mut reversed_heights = Vec::with_capacity(heights.len());
            for row in (0..rows).rev() {
                reversed_heights.extend_from_slice(&heights[row * cols..(row + 1) * cols]);
            }

            let scale = [(cols as f32 - 1.0) * cell_size, height_scale, (rows as f32 - 1.0) * cell_size];

            let collider = deerpunk_engine_common::physics::ColliderDescriptor {
                shape: deerpunk_engine_common::physics::ColliderShape::HeightField {
                    heights: reversed_heights, rows, cols, scale,
                },
                body_type: deerpunk_engine_common::physics::BodyType::Static,
                category_bits: deerpunk_engine_common::physics::categories::TERRAIN,
                mask_bits: deerpunk_engine_common::physics::categories::TANK
                    | deerpunk_engine_common::physics::categories::PROJECTILE,
                friction,
                restitution,
                density: 0.0,
                is_sensor: false,
            };

            let half_x = (cols as f32 - 1.0) * cell_size * 0.5;
            let half_y = (rows as f32 - 1.0) * cell_size * 0.5;
            self.physics_world.create_body(
                entity_id, half_x, half_y, 0.0,
                [std::f32::consts::FRAC_PI_2, 0.0, 0.0],
                &collider,
            );
            self.terrain_bodies.insert(entity_id);

            println!("[server_engine] terrain collider: entity={:#x} {}x{} cell={} hs={}",
                entity_id, rows, cols, cell_size, height_scale);
        }
    }
}

impl Default for ServerEngine {
    fn default() -> Self {
        Self::new()
    }
}
