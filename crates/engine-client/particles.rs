//! Engine-side particle runtime: effect catalog, emitter ticking, shared mesh
//! upload per texture group.
//!
//! Design notes:
//! - `ParticleEmitter` components (engine-common) carry per-entity stream state.
//! - Bursts go through [`ClientEngine::spawn_particles_burst`] (also exposed as
//!   `v1:particles:spawn_burst`); the payload is plain serializable data so the
//!   wire layer can encode it verbatim.
//! - One shared `Mesh` per texture group is rebuilt per frame and pushed through
//!   `MeshRegistry::replace` (renderer re-uploads on `gpu_revision` change).
//! - Effects are loaded from `<gamedir>/particles/*.json` at init; unknown
//!   affector types are hard load errors.
//!
//! `ClientEngine::tick_particles` drives the three phases: [`simulate`], then
//! material/entity/mesh setup per texture group, then [`rebuild_group_mesh`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use deerpunk_engine_common::components::{Mesh3D, MeshDrawHidden, ParticleEmitter, TransformComponent};
use deerpunk_engine_common::material::DrawMaterial;
use deerpunk_engine_common::material::MaterialHandle;
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
use deerpunk_engine_common::resource::mesh_registry::MeshRegistry;
use deerpunk_engine_common::world::{EntityId, World};
use deerpunk_engine_particles::{
    atlas_uv_rect, AffectorRegistry, CustomAffector, EffectDef, ParticleSim, SpawnPayload,
    SpriteSource, WorldCollider, PARTICLE_VERTEX_STRIDE,
};
use glam::Vec3;

pub(crate) const PARTICLE_SHADER_URI: &str = "internal://shaders/particles/billboard.wgsl";

/// One live effect instance (own pool; meshes are shared by texture group).
pub(crate) struct EffectInstance {
    pub name: String,
    pub tex_uri: Option<String>,
    pub sim: ParticleSim,
}

/// One shared mesh + draw entity per texture.
pub(crate) struct MeshGroup {
    pub tex_uri: String,
    pub mesh: Option<MeshHandle>,
    pub entity: Option<EntityId>,
    pub material: Option<MaterialHandle>,
}

/// Burst queued from the spawn API; drained on the next tick.
struct PendingBurst {
    effect: String,
    pos: [f32; 3],
    base_vel: [f32; 3],
    payload: SpawnPayload,
}

/// Engine particle backend state, owned by `ClientEngine`.
#[derive(Default)]
pub struct ParticleFx {
    registry: AffectorRegistry,
    /// Game-optional world query hook for `collide_world`.
    collider: Option<Arc<dyn WorldCollider>>,
    catalog: HashMap<String, EffectDef>,
    instances: Vec<EffectInstance>,
    pub(crate) groups: Vec<MeshGroup>,
    pending_bursts: Vec<PendingBurst>,
    time: f32,
    /// Set once the catalog has been loaded (init gate).
    loaded: bool,
}

impl ParticleFx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a game custom affector (must happen before catalog load).
    pub fn register_custom_affector(
        &mut self,
        type_name: impl Into<String>,
        f: Arc<dyn CustomAffector>,
    ) {
        self.registry.register_custom(type_name, f);
    }

    /// Register the world query hook used by `collide_world` affectors.
    pub fn set_world_collider(&mut self, collider: Arc<dyn WorldCollider>) {
        self.collider = Some(collider);
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

    pub(crate) fn instance_for(&mut self, effect: &str) -> Option<&mut EffectInstance> {
        let name = effect.to_string();
        if let Some(idx) = self.instances.iter().position(|i| i.name == name) {
            return self.instances.get_mut(idx);
        }
        let def = self.catalog.get(&name).cloned()?;
        let tex_uri = def.sprite.texture_uri().map(String::from);
        match ParticleSim::new(def, &self.registry, self.collider.is_some()) {
            Ok(sim) => {
                self.instances.push(EffectInstance { name, tex_uri, sim });
                self.instances.last_mut()
            }
            Err(e) => {
                log::error!("[particles] effect '{effect}' init failed: {e}");
                None
            }
        }
    }

    pub(crate) fn group_for(&mut self, tex_uri: &str) -> usize {
        if let Some(idx) = self.groups.iter().position(|g| g.tex_uri == tex_uri) {
            return idx;
        }
        self.groups.push(MeshGroup {
            tex_uri: tex_uri.to_string(),
            mesh: None,
            entity: None,
            material: None,
        });
        self.groups.len() - 1
    }

    /// JSON material asset for a group's texture (engine particle shader).
    pub(crate) fn group_material_json(tex_uri: &str) -> String {
        format!(
            r#"{{"shader":"{PARTICLE_SHADER_URI}","shader_defines":[],"properties":{{"_BaseColor":[1.0,1.0,1.0,1.0],"_BaseMap":"{tex_uri}"}}}}"#
        )
    }
}

/// Phase 1: drain queued bursts + stream emitter components, integrate.
/// Returns the texture uris that have live particles this frame.
pub(crate) fn simulate(state: &mut ParticleFx, world: &mut World, dt: f32) -> Vec<String> {
    let mut live_tex: Vec<String> = Vec::new();
    if !state.loaded {
        return live_tex;
    }
    state.time += dt;
    let time = state.time;

    // 1. Queued bursts.
    let bursts = std::mem::take(&mut state.pending_bursts);
    for b in bursts {
        if let Some(inst) = state.instance_for(&b.effect) {
            inst.sim.spawn_burst(
                Vec3::new(b.pos[0], b.pos[1], b.pos[2]),
                Vec3::new(b.base_vel[0], b.base_vel[1], b.base_vel[2]),
                &b.payload,
                time,
            );
        }
    }

    // 2. Stream emitters (component-driven accumulators).
    drain_stream_emitters(state, world, dt, time);

    // 3. Integrate all instances; track live texture groups.
    for inst in &mut state.instances {
        inst.sim.tick(
            dt,
            time,
            state.collider.as_deref().map(|c| c as &dyn WorldCollider),
        );
        if inst.sim.live() > 0 {
            if let Some(tex) = inst.tex_uri.clone() {
                if !live_tex.contains(&tex) {
                    live_tex.push(tex);
                }
            }
        }
    }
    live_tex
}

/// Phase 2: ensure the draw entity (material set by the caller when loaded).
pub(crate) fn ensure_group_draw(
    state: &mut ParticleFx,
    group_idx: usize,
    world: &mut World,
    material: Option<MaterialHandle>,
) {
    if let Some(h) = material {
        state.groups[group_idx].material = Some(h);
    }
    let group = &mut state.groups[group_idx];
    let Some(material) = group.material else {
        return; // material creation failed this frame; caller logs
    };
    if group.entity.is_some() {
        unhide_group(world, group);
        return;
    }
    let entity = world.spawn();
    world.set(
        entity,
        world.component_init::<TransformComponent>(),
        TransformComponent::default(),
    );
    world.set(entity, world.component_init::<Mesh3D>(), Mesh3D::default());
    world.set(
        entity,
        world.component_init::<DrawMaterial>(),
        DrawMaterial::new(material),
    );
    group.entity = Some(entity);
}

/// Phase 2b (needs `MeshRegistry`): create/refresh the group's mesh slot and
/// wire it onto the draw entity.
pub(crate) fn ensure_group_mesh(
    state: &mut ParticleFx,
    group_idx: usize,
    world: &mut World,
    meshes: &mut MeshRegistry,
) {
    let group = &mut state.groups[group_idx];
    let Some(entity) = group.entity else {
        return;
    };
    if group.mesh.is_some() {
        return;
    }
    let mut mesh = Mesh::empty();
    mesh.vertex_size = PARTICLE_VERTEX_STRIDE;
    mesh.bounds_min = [0.0; 3];
    mesh.bounds_max = [0.001; 3];
    let handle = meshes.insert(mesh);
    meshes.attach(handle);
    group.mesh = Some(handle);
    let m3 = world.component_init::<Mesh3D>();
    if let Some(mut m) = world.get::<Mesh3D>(entity, m3) {
        m.mesh = handle;
        m.bounds_min = [0.0; 3];
        m.bounds_max = [0.001; 3];
        world.set(entity, m3, m);
    }
}

/// Phase 3: rebuild the group's shared mesh from all live instances on that
/// texture. Returns true when a mesh was replaced.
pub(crate) fn rebuild_group_mesh(
    state: &mut ParticleFx,
    group_idx: usize,
    world: &mut World,
    meshes: &mut MeshRegistry,
    camera_pos: Vec3,
) -> bool {
    let tex_uri = state.groups[group_idx].tex_uri.clone();
    let Some(handle) = state.groups[group_idx].mesh else {
        return false;
    };
    let entity = state.groups[group_idx].entity;
    let full_rect = [0.0, 0.0, 1.0, 1.0];
    let mut mesh = Mesh::empty();
    mesh.vertex_size = PARTICLE_VERTEX_STRIDE;
    for inst in &mut state.instances {
        if inst.tex_uri.as_deref() != Some(tex_uri.as_str()) || inst.sim.live() == 0 {
            continue;
        }
        let def = &inst.sim.def;
        let fade_out = def.particle.fade_out;
        let use_gradient = def.particle.color.start != def.particle.color.end;
        for p in &inst.sim.particles {
            let t = (p.age / p.ttl).clamp(0.0, 1.0);
            let mut tint = p.tint;
            if use_gradient {
                tint = def.particle.color.eval(t);
            }
            if fade_out > 0.0 {
                let fade_start = (1.0 - fade_out).max(0.0);
                if t > fade_start {
                    let k = ((t - fade_start) / fade_out).clamp(0.0, 1.0);
                    tint[3] *= 1.0 - k;
                }
            }
            let uvr = p.uv.unwrap_or(match def.sprite {
                SpriteSource::Atlas { tile, .. } => atlas_uv_rect(tile.max(1), p.frame as usize),
                _ => full_rect,
            });
            deerpunk_engine_particles::push_billboard_vertex(
                &mut mesh,
                p.pos,
                p.size,
                p.rot,
                uvr,
                full_rect,
                tint,
                camera_pos,
            );
        }
    }
    if mesh.vertex_count() == 0 {
        return false;
    }
    update_entity_bounds(world, entity, &mesh);
    if meshes.replace(handle, mesh) {
        true
    } else {
        log::warn!("[particles] mesh replace failed — dropping group mesh");
        state.groups[group_idx].mesh = None;
        false
    }
}

/// Hide a drained group's draw entity.
pub(crate) fn hide_drained_groups(state: &mut ParticleFx, world: &mut World, live_tex: &[String]) {
    for group in &mut state.groups {
        if live_tex.contains(&group.tex_uri) {
            continue;
        }
        let Some(entity) = group.entity else {
            continue;
        };
        let hid = world.component_init::<MeshDrawHidden>();
        if !world.has(entity, hid) {
            world.set(entity, hid, MeshDrawHidden::marker());
        }
    }
}

fn unhide_group(world: &World, group: &mut MeshGroup) {
    let Some(entity) = group.entity else {
        return;
    };
    let hid = world.component_init::<MeshDrawHidden>();
    if world.has(entity, hid) {
        world.remove(entity, hid);
    }
}

fn update_entity_bounds(world: &World, entity: Option<EntityId>, mesh: &Mesh) {
    let Some(entity) = entity else {
        return;
    };
    let m3 = world.component_init::<Mesh3D>();
    if let Some(mut m) = world.get::<Mesh3D>(entity, m3) {
        m.bounds_min = mesh.bounds_min;
        m.bounds_max = mesh.bounds_max;
        m.world_baked = 0;
        world.set(entity, m3, m);
    }
}

/// Stream emitters: advance per-entity accumulators (stored on the component),
/// spawn owed particles at the entity's position.
fn drain_stream_emitters(state: &mut ParticleFx, world: &mut World, dt: f32, _time: f32) {
    let t_id = world.component_init::<TransformComponent>();
    let pe_id = world.component_init::<ParticleEmitter>();
    let q = world.query_prepare_ids(&[t_id, pe_id]);
    let mut to_spawn: Vec<(String, [f32; 3], f32, u32)> = Vec::new();
    let mut acc_updates: Vec<(EntityId, f32)> = Vec::new();
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
                if effect.is_empty() || !state.catalog.contains_key(&effect) {
                    continue;
                }
                let rate = if pe.rate > 0.0 {
                    pe.rate
                } else {
                    state
                        .catalog
                        .get(&effect)
                        .map(|d| d.emitter.rate)
                        .unwrap_or(0.0)
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
    let time = state.time;
    for (effect, pos, inherit, owed) in to_spawn {
        let Some(inst) = state.instance_for(&effect) else {
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
            Vec3::new(pos[0], pos[1], pos[2]),
            Vec3::ZERO, // emitter entity velocity tracking is a v2 feature
            owed,
            &SpawnPayload::default(),
            time,
        );
        inst.sim.def.emitter.inherit_velocity = saved;
    }
}
