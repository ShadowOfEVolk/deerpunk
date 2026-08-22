use std::any::Any;
use std::collections::HashMap;

use hecs::World;

use deerpunk_engine_common::physics::PhysicsWorld;

use crate::schedule::Schedule;
use super::api::{ComponentEntry, ComponentId, EntityId, entity_from_hecs, entity_to_hecs};

pub struct ServerEngine {
    pub ecs_world: World,
    pub physics_world: PhysicsWorld,
    pub(crate) schedule: Schedule,
    pub(crate) components: HashMap<ComponentId, ComponentEntry>,
    pub(crate) entities: Vec<EntityId>,
    pub(crate) next_component_id: u64,
    pub(crate) initialized: bool,
}

impl ServerEngine {
    pub fn new() -> Self {
        Self {
            ecs_world: World::new(),
            physics_world: PhysicsWorld::new(),
            schedule: Schedule::new(),
            components: HashMap::new(),
            entities: Vec::new(),
            next_component_id: 0,
            initialized: false,
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
        self.initialized = true;
        println!("ServerEngine initialized");
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
            println!("ServerEngine shutdown");
        }
    }

    // ── Component registration ──────────────────────────────────

    pub fn register_component(
        &mut self,
        _name: &str,
        has: impl Fn(&World, hecs::Entity) -> bool + Send + Sync + 'static,
        get_boxed: impl Fn(&World, hecs::Entity) -> Option<Box<dyn Any>> + Send + Sync + 'static,
        set_boxed: impl Fn(&mut World, hecs::Entity, Box<dyn Any>) + Send + Sync + 'static,
        remove: impl Fn(&mut World, hecs::Entity) + Send + Sync + 'static,
    ) -> ComponentId {
        let id = self.next_component_id;
        self.next_component_id += 1;
        self.components.insert(id, ComponentEntry {
            has: std::sync::Arc::new(has),
            get_boxed: std::sync::Arc::new(get_boxed),
            set_boxed: std::sync::Arc::new(set_boxed),
            remove: std::sync::Arc::new(remove),
        });
        id
    }

    pub fn register_component_with_fns(
        &mut self,
        _name: &str,
        has_fn: super::vtable::CompHasFn,
        get_fn: super::vtable::CompGetFn,
        set_fn: super::vtable::CompSetFn,
        remove_fn: super::vtable::CompRemoveFn,
    ) -> u64 {
        let id = self.next_component_id;
        self.next_component_id += 1;
        self.components.insert(id, ComponentEntry {
            has: std::sync::Arc::new(move |world, entity| {
                unsafe { has_fn(world as *const World as *mut std::ffi::c_void, entity.to_bits().get()) }
            }),
            get_boxed: std::sync::Arc::new(move |world, entity| {
                let ptr = unsafe { get_fn(world as *const World as *mut std::ffi::c_void, entity.to_bits().get()) };
                if ptr.is_null() { None } else { unsafe { Some(*Box::from_raw(ptr as *mut Box<dyn Any>)) } }
            }),
            set_boxed: std::sync::Arc::new(move |world, entity, data| {
                let bx = Box::new(data);
                unsafe { set_fn(world as *mut World as *mut std::ffi::c_void, entity.to_bits().get(), Box::into_raw(bx) as *mut std::ffi::c_void) }
            }),
            remove: std::sync::Arc::new(move |world, entity| {
                unsafe { remove_fn(world as *mut World as *mut std::ffi::c_void, entity.to_bits().get()) }
            }),
        });
        id
    }

    pub fn register_component_with_entry(&mut self, _name: &str, entry: ComponentEntry) -> u64 {
        let id = self.next_component_id;
        self.next_component_id += 1;
        self.components.insert(id, entry);
        id
    }

    // ── Entity/component access ─────────────────────────────────

    pub fn has(&self, entity_id: u64, comp_id: u64) -> bool {
        let entity = entity_to_hecs(entity_id);
        if let Some(entry) = self.components.get(&comp_id) {
            (entry.has)(&self.ecs_world, entity)
        } else {
            false
        }
    }

    pub fn get_boxed_raw(&self, entity_id: u64, comp_id: u64) -> Option<Box<dyn Any>> {
        let entity = entity_to_hecs(entity_id);
        let entry = self.components.get(&comp_id)?;
        (entry.get_boxed)(&self.ecs_world, entity)
    }

    pub fn set_boxed_raw(&mut self, entity_id: u64, comp_id: u64, data: Box<dyn Any>) {
        let entity = entity_to_hecs(entity_id);
        if let Some(entry) = self.components.get(&comp_id) {
            (entry.set_boxed)(&mut self.ecs_world, entity, data);
        }
    }

    pub fn remove(&mut self, entity_id: u64, comp_id: u64) {
        let entity = entity_to_hecs(entity_id);
        if let Some(entry) = self.components.get(&comp_id) {
            (entry.remove)(&mut self.ecs_world, entity);
        }
    }

    pub fn spawn(&mut self) -> u64 {
        let entity = self.ecs_world.spawn(());
        let id = entity_from_hecs(entity);
        self.entities.push(id);
        id
    }

    pub fn spawn_bundle(&mut self, components: Vec<(u64, Box<dyn Any>)>) -> u64 {
        let entity = self.ecs_world.spawn(());
        for (comp_id, data) in components {
            if let Some(entry) = self.components.get(&comp_id) {
                (entry.set_boxed)(&mut self.ecs_world, entity, data);
            }
        }
        let id = entity_from_hecs(entity);
        self.entities.push(id);
        id
    }

    pub fn destroy(&mut self, entity_id: u64) {
        let entity = entity_to_hecs(entity_id);
        let _ = self.ecs_world.despawn(entity);
        self.entities.retain(|&e| e != entity_id);
    }

    pub fn clear(&mut self) {
        self.ecs_world.clear();
        self.entities.clear();
    }

    pub fn query(&self, require: &[u64], exclude: &[u64]) -> Vec<u64> {
        use std::collections::HashSet;

        let require_set: HashSet<u64> = require.iter().copied().collect();
        let exclude_set: HashSet<u64> = exclude.iter().copied().collect();

        if require_set.is_empty() {
            return self.entities.iter()
                .filter(|&&e| !exclude_set.iter().any(|&c| self.has(e, c)))
                .copied()
                .collect();
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
}

impl Default for ServerEngine {
    fn default() -> Self {
        Self::new()
    }
}
