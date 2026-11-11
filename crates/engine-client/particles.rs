//! Engine-side particle runtime: effect catalog, emission instances, one
//! shared dense particle pool packed once per frame.
//!
//! Design notes:
//! - `ParticleEmitter` components (engine-common) carry per-entity stream state.
//! - Bursts go through [`ClientEngine::spawn_particles_burst`] (also exposed as
//!   `v1:particles:spawn_burst`); the payload is plain serializable data so the
//!   wire layer can encode it verbatim.
//! - All live particles live in one shared [`ParticlePool`]. Each tick drains
//!   queued bursts + stream emitters into it, integrates once, and packs a
//!   [`ParticleFrame`] that the renderer draws as GPU billboards — no ECS mesh
//!   entities, no per-texture meshes or materials. Textures are uploaded on
//!   first use and handed to `Renderer::register_particle_texture`.
//! - Effects are loaded from `<gamedir>/particles/*.json` at init; unknown
//!   affector types are hard load errors.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use deerpunk_engine_common::components::{ParticleEmitter, TransformComponent};
use deerpunk_engine_common::particle_gpu::ParticleFrame;
use deerpunk_engine_common::resource::ResourceManager;
use deerpunk_engine_common::world::World;
use deerpunk_engine_common::MeshTexture;
use deerpunk_engine_particles::{
    AffectorRegistry, CustomAffector, EffectDef, ParticlePool, ParticleSim, SpawnPayload,
    WorldCollider,
};
use glam::Vec3;

use deerpunk_engine_renderer::material_store::{GpuTextureResolver, TextureResolver};

/// Shared-pool capacity default (`r_particles_max`), in particles.
const DEFAULT_POOL_CAPACITY: usize = 1 << 20;

/// One live effect instance: emission state only. Particles live in the
/// shared pool on [`ParticleFx`].
pub(crate) struct EffectInstance {
    pub name: String,
    /// Texture the renderer must have registered (`None` = procedural sprite).
    pub tex_uri: Option<Arc<str>>,
    pub sim: ParticleSim,
}

/// Burst queued from the spawn API; drained on the next tick.
struct PendingBurst {
    effect: String,
    pos: [f32; 3],
    base_vel: [f32; 3],
    payload: SpawnPayload,
}

/// Engine particle backend state, owned by `ClientEngine`.
pub struct ParticleFx {
    registry: AffectorRegistry,
    /// Game-optional world query hook for `collide_world`.
    collider: Option<Arc<dyn WorldCollider>>,
    catalog: HashMap<String, EffectDef>,
    /// Emission instances (one per effect name in use).
    instances: Vec<EffectInstance>,
    /// The one dense pool; packed into the active frame slot every tick.
    pool: ParticlePool,
    /// Double-buffered packed frames. Each surface-camera snapshot shares
    /// the freshly packed `Arc`; the second slot lets the pool write in
    /// place while the renderer may still hold the previous frame.
    frames: [Arc<ParticleFrame>; 2],
    flip: usize,
    /// Uploaded particle texture slots, deduped by URI.
    uploaded: HashMap<String, MeshTexture>,
    /// URIs already handed to `Renderer::register_particle_texture`.
    registered: HashSet<String>,
    pending_bursts: Vec<PendingBurst>,
    time: f32,
    /// Set once the catalog has been loaded (init gate).
    loaded: bool,
}

impl ParticleFx {
    pub fn new() -> Self {
        let registry = AffectorRegistry::new();
        let mut pool = ParticlePool::new(DEFAULT_POOL_CAPACITY);
        pool.set_registry(registry.clone());
        Self {
            registry,
            collider: None,
            catalog: HashMap::new(),
            instances: Vec::new(),
            pool,
            frames: [
                Arc::new(ParticleFrame::default()),
                Arc::new(ParticleFrame::default()),
            ],
            flip: 0,
            uploaded: HashMap::new(),
            registered: HashSet::new(),
            pending_bursts: Vec::new(),
            time: 0.0,
            loaded: false,
        }
    }

    /// Register a game custom affector (must happen before catalog load).
    pub fn register_custom_affector(
        &mut self,
        type_name: impl Into<String>,
        f: Arc<dyn CustomAffector>,
    ) {
        self.registry.register_custom(type_name, f);
        self.pool.set_registry(self.registry.clone());
    }

    /// Register the world query hook used by `collide_world` affectors.
    pub fn set_world_collider(&mut self, collider: Arc<dyn WorldCollider>) {
        self.collider = Some(collider);
        self.pool.set_collider(self.collider.clone());
    }

    /// Load `<gamedir>/particles/*.json`. Per-file failures are hard errors
    /// (logged); the good files still load.
    pub fn load_catalog(&mut self, gamedir: &Path) {
        let dir = gamedir.join("particles");
        let has_collider = self.collider.is_some();
        match EffectDef::load_catalog(&dir, &self.registry, has_collider) {
            Ok((defs, errors)) => {
                for e in &errors {
                    log::error!("[particles] catalog: {e}");
                }
                for (name, def) in defs {
                    log::info!("[particles] loaded effect '{}'", name);
                    self.catalog.insert(name, def);
                }
            }
            Err(e) => {
                if !dir.exists() {
                    log::debug!("[particles] no {} — particles disabled", dir.display());
                } else {
                    log::error!("[particles] catalog load: {e}");
                }
            }
        }
        self.loaded = true;
    }

    /// Queue a burst. Cheap: pool work happens on the next tick.
    pub fn spawn_burst(
        &mut self,
        effect: &str,
        pos: [f32; 3],
        base_vel: [f32; 3],
        payload: &SpawnPayload,
    ) {
        if self.catalog.is_empty() {
            log::warn!("[particles] spawn_burst '{effect}': catalog empty");
            return;
        }
        if !self.catalog.contains_key(effect) {
            log::error!("[particles] spawn_burst: unknown effect '{effect}'");
            return;
        }
        self.pending_bursts.push(PendingBurst {
            effect: effect.to_string(),
            pos,
            base_vel,
            payload: payload.clone(),
        });
    }

    /// Live particle count + distinct textures in the last packed frame
    /// (`r_speeds` overlay).
    pub fn stats(&self) -> (usize, usize) {
        (self.pool.stats().0, self.frames[self.flip ^ 1].ranges.len())
    }

    /// The frame packed by the last [`tick`] — a cheap `Arc` clone per
    /// surface camera (no per-camera copy of instance data).
    pub(crate) fn packed_frame(&self) -> Arc<ParticleFrame> {
        Arc::clone(&self.frames[self.flip ^ 1])
    }

    /// Clamp the shared pool capacity (`r_particles_max`). Applied by
    /// recreating the pool (live particles drop); the collider hook and the
    /// custom-affector registry are carried over.
    pub fn set_capacity(&mut self, capacity: usize) {
        if self.pool.stats().1 == capacity {
            return;
        }
        let mut pool = ParticlePool::new(capacity);
        pool.set_registry(self.registry.clone());
        pool.set_collider(self.collider.clone());
        self.pool = pool;
    }

    /// Upload particle textures on first use and register them with the
    /// renderer's particle pass. Failed loads retry next frame.
    pub(crate) fn ensure_textures_registered(
        &mut self,
        resources: &ResourceManager,
        renderer: &mut deerpunk_engine_renderer::Renderer,
    ) {
        let mut want: Vec<Arc<str>> = self
            .instances
            .iter()
            .filter_map(|i| i.tex_uri.as_ref().map(Arc::clone))
            .collect();
        // Payload texture overrides spawn ranges with URIs that no effect
        // instance carries — register whatever the last pack actually drew.
        want.extend(
            self.frames[self.flip ^ 1]
                .ranges
                .iter()
                .map(|r| Arc::clone(&r.tex_uri)),
        );
        for uri in want {
            if self.registered.contains(uri.as_ref()) {
                continue;
            }
            let mut resolver = GpuTextureResolver {
                resources,
                renderer,
                uploaded: &mut self.uploaded,
            };
            let resolved = resolver.resolve(&uri);
            drop(resolver);
            match resolved {
                Ok(tex) => {
                    renderer.register_particle_texture(&uri, tex);
                    self.registered.insert(uri.to_string());
                }
                Err(e) => log::error!("[particles] texture '{uri}': {e}"),
            }
        }
    }
}

/// Find or create the emission instance for an effect name.
fn instance_for<'a>(
    instances: &'a mut Vec<EffectInstance>,
    catalog: &HashMap<String, EffectDef>,
    registry: &AffectorRegistry,
    has_collider: bool,
    effect: &str,
) -> Option<&'a mut EffectInstance> {
    if let Some(idx) = instances.iter().position(|i| i.name == effect) {
        return instances.get_mut(idx);
    }
    let def = catalog.get(effect).cloned()?;
    let tex_uri = def.sprite.texture_uri().map(Arc::<str>::from);
    match ParticleSim::new(def, registry, has_collider) {
        Ok(sim) => {
            instances.push(EffectInstance {
                name: effect.to_string(),
                tex_uri,
                sim,
            });
            instances.last_mut()
        }
        Err(e) => {
            log::error!("[particles] effect '{effect}' init failed: {e}");
            None
        }
    }
}

/// Drain queued bursts + stream emitter components into the shared pool,
/// integrate once, and pack the frame. Called from `extract_scene_view`.
pub(crate) fn tick(state: &mut ParticleFx, world: &mut World, dt: f32) {
    if !state.loaded {
        return;
    }
    state.time += dt;
    let time = state.time;

    let ParticleFx {
        instances,
        catalog,
        registry,
        collider,
        pool,
        pending_bursts,
        ..
    } = state;

    // 1. Queued bursts.
    let bursts = std::mem::take(pending_bursts);
    for b in bursts {
        if let Some(inst) =
            instance_for(instances, catalog, registry, collider.is_some(), &b.effect)
        {
            inst.sim.spawn_burst(
                pool,
                Vec3::new(b.pos[0], b.pos[1], b.pos[2]),
                Vec3::new(b.base_vel[0], b.base_vel[1], b.base_vel[2]),
                &b.payload,
                time,
            );
        }
    }

    // 2. Stream emitters (component-driven accumulators).
    drain_stream_emitters(
        instances, catalog, registry, collider, world, dt, time, pool,
    );

    // 3. Integrate + pack (fade × opacity math lives in the pool).
    pool.integrate(dt);
    pack_frame(state);
}

/// Pack the pool into the active frame slot and flip. `Arc::get_mut` keeps
/// the packed vectors' capacity across frames when the renderer has already
/// released the previous frame; otherwise the slot is replaced with a fresh
/// allocation (renderer still holds it).
fn pack_frame(state: &mut ParticleFx) {
    let slot = &mut state.frames[state.flip];
    let frame = match Arc::get_mut(slot) {
        Some(frame) => frame,
        None => {
            *slot = Arc::new(ParticleFrame::default());
            Arc::get_mut(slot).expect("freshly allocated Arc is exclusive")
        }
    };
    state.pool.pack_into(frame);
    state.flip ^= 1;
}

/// Stream emitters: advance per-entity accumulators (stored on the component),
/// spawn owed particles at the entity's position into the shared pool.
fn drain_stream_emitters(
    instances: &mut Vec<EffectInstance>,
    catalog: &HashMap<String, EffectDef>,
    registry: &AffectorRegistry,
    collider: &Option<Arc<dyn WorldCollider>>,
    world: &mut World,
    dt: f32,
    time: f32,
    pool: &mut ParticlePool,
) {
    let t_id = world.component_init::<TransformComponent>();
    let pe_id = world.component_init::<ParticleEmitter>();
    let q = world.query_prepare_ids(&[t_id, pe_id]);
    let mut to_spawn: Vec<(String, [f32; 3], f32, u32)> = Vec::new();
    let mut acc_updates: Vec<(deerpunk_engine_common::world::EntityId, f32)> = Vec::new();
    if let Some(batches) = world.query_batches(q) {
        for batch in batches {
            let ents = unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
            let ts = unsafe {
                std::slice::from_raw_parts(
                    batch.columns[0] as *const TransformComponent,
                    batch.count,
                )
            };
            let pes = unsafe {
                std::slice::from_raw_parts(batch.columns[1] as *const ParticleEmitter, batch.count)
            };
            for i in 0..batch.count {
                let pe = &pes[i];
                let effect = pe.effect_str().to_string();
                if effect.is_empty() || !catalog.contains_key(&effect) {
                    continue;
                }
                let rate = if pe.rate > 0.0 {
                    pe.rate
                } else {
                    catalog.get(&effect).map(|d| d.emitter.rate).unwrap_or(0.0)
                };
                if rate <= 0.0 || !pe.enabled {
                    continue;
                }
                // Per-entity fractional accumulator (client-side runtime state).
                let mut acc = pe.acc + rate * dt;
                let owed = acc as u32;
                acc -= owed as f32;
                if owed > 0 {
                    to_spawn.push((effect, ts[i].position.to_array(), pe.inherit_velocity, owed));
                }
                acc_updates.push((ents[i], acc));
            }
        }
    }
    world.query_release(q);
    for (entity, acc) in acc_updates {
        let pe_id = world.component_init::<ParticleEmitter>();
        if let Some(mut pe) = world.get::<ParticleEmitter>(entity, pe_id) {
            pe.acc = acc;
            world.set(entity, pe_id, pe);
        }
    }
    for (effect, pos, inherit, owed) in to_spawn {
        let Some(inst) = instance_for(instances, catalog, registry, collider.is_some(), &effect)
        else {
            continue;
        };
        let inherit = if inherit < 0.0 {
            inst.sim.def.emitter.inherit_velocity
        } else {
            inherit
        };
        let saved = inst.sim.def.emitter.inherit_velocity;
        inst.sim.def.emitter.inherit_velocity = inherit;
        inst.sim.spawn_stream_at(
            pool,
            Vec3::new(pos[0], pos[1], pos[2]),
            Vec3::ZERO, // emitter entity velocity tracking is a v2 feature
            owed,
            &SpawnPayload::default(),
            time,
        );
        inst.sim.def.emitter.inherit_velocity = saved;
    }
}
